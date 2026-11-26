import { useQuery } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import type { NodeOutput } from '@/bindings/NodeOutput'
import type { NodeStatus } from '@/bindings/NodeStatus'
import type { RunDetail } from '@/bindings/RunDetail'
import { useRun, useRuns } from '@/api/queries.ts'
import { useRunEvents } from '@/api/sse.ts'
import { formatDataType } from '@/lib/socketColors.ts'
import { useEditorStore } from '@/store/editorStore.ts'

export interface ActiveRun {
  id: string
  /** true: follow via SSE (running); false: finished run, fetched once. */
  live: boolean
}

interface Props {
  workflowId: string
  activeRun: ActiveRun | null
  onSelectRun: (run: ActiveRun) => void
}

const PREVIEW_LIMIT = 5000

const STATUS_TEXT: Record<NodeStatus, string> = {
  pending: '대기',
  running: '실행 중',
  succeeded: '완료',
  failed: '실패',
  skipped: '건너뜀',
}
const RUN_STATUS_TEXT = { running: '실행 중', succeeded: '성공', failed: '실패' } as const
const CONNECTION_TEXT = { idle: '', connecting: '연결 중…', open: '연결됨', closed: '종료' } as const

const fmtTime = (iso: string | null) => (iso ? new Date(iso).toLocaleString('ko-KR') : '-')

function TextPreview({ url }: { url: string }) {
  const text = useQuery({
    queryKey: ['artifact-text', url],
    queryFn: async () => {
      const res = await fetch(url)
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      return (await res.text()).slice(0, PREVIEW_LIMIT)
    },
  })
  if (text.isPending) return <p className="text-xs text-slate-500">불러오는 중…</p>
  if (text.isError) return <p role="alert" className="text-xs text-red-600">{text.error.message}</p>
  return (
    <pre className="max-h-40 overflow-auto rounded bg-slate-50 p-2 text-[11px] whitespace-pre-wrap">
      {text.data}
    </pre>
  )
}

function FileItem({ output, file }: { output: NodeOutput; file: NodeOutput['files'][number] }) {
  const [open, setOpen] = useState(false)
  const scalar = output.dataType.scalar
  const previewable = scalar !== 'PDF'

  return (
    <li className="space-y-1">
      <div className="flex items-center gap-2">
        <a
          href={file.url}
          target="_blank"
          rel="noreferrer"
          className="text-indigo-600 underline"
        >
          {file.fileName}
        </a>
        {previewable && (
          <button
            type="button"
            onClick={() => setOpen((o) => !o)}
            aria-expanded={open}
            className="rounded border border-slate-300 px-1.5 text-[11px]"
          >
            {open ? '미리보기 닫기' : '미리보기'}
          </button>
        )}
      </div>
      {open &&
        (scalar === 'JPG' ? (
          <img src={file.url} alt={file.fileName} loading="lazy" className="max-h-40 rounded" />
        ) : scalar === 'MP3' ? (
          <audio controls src={file.url} />
        ) : (
          <TextPreview url={file.url} />
        ))}
    </li>
  )
}

function RunDetails({ run, connection }: { run: RunDetail; connection: string }) {
  const catalog = useEditorStore((s) => s.catalog)
  const labelOf = (kind: string) => catalog.find((c) => c.kind === kind)?.label ?? kind

  return (
    <div className="space-y-2">
      <p className="text-xs text-slate-600">
        <span className="font-semibold">런 {RUN_STATUS_TEXT[run.status]}</span> · 시작{' '}
        {fmtTime(run.startedAt)} · 종료 {fmtTime(run.finishedAt)}
        {connection && <span> · {connection}</span>}
      </p>
      <ul className="space-y-2">
        {run.nodes.map((n) => (
          <li key={n.nodeId} data-testid={`run-node-${n.nodeId}`} className="text-xs">
            <div className="flex items-center gap-2">
              <span className="font-medium">{labelOf(n.kind)}</span>
              <span className="text-slate-500">{STATUS_TEXT[n.status]}</span>
            </div>
            {n.error && <p className="text-red-600">{n.error}</p>}
            {n.outputs.map((o) =>
              o.files.length === 0 ? null : (
                <div key={o.socketId} className="mt-1 ml-3">
                  <p className="text-slate-500">
                    {o.socketId} ({formatDataType(o.dataType)})
                  </p>
                  <ul className="ml-2 space-y-1">
                    {o.files.map((f) => (
                      <FileItem key={f.index} output={o} file={f} />
                    ))}
                  </ul>
                </div>
              ),
            )}
          </li>
        ))}
      </ul>
    </div>
  )
}

export default function RunPanel({ workflowId, activeRun, onSelectRun }: Props) {
  const history = useRuns(workflowId)
  const setRunState = useEditorStore((s) => s.setRunState)
  const clearRun = useEditorStore((s) => s.clearRun)

  const live = activeRun?.live ?? false
  const sse = useRunEvents(activeRun && live ? activeRun.id : null)
  const fetched = useRun(activeRun && !live ? activeRun.id : undefined)

  const run = live ? sse.run : (fetched.data ?? null)
  const connection = live ? CONNECTION_TEXT[sse.connection] : ''

  useEffect(() => {
    if (!run) {
      clearRun()
      return
    }
    setRunState(Object.fromEntries(run.nodes.map((n) => [n.nodeId, n.status])), run.status)
  }, [run, setRunState, clearRun])
  useEffect(() => clearRun, [clearRun])

  return (
    <section aria-label="실행 패널" className="flex h-56 shrink-0 border-t border-slate-200 bg-white">
      <div className="w-56 shrink-0 overflow-y-auto border-r border-slate-200 p-3">
        <h3 className="mb-2 text-xs font-semibold text-slate-500">실행 이력</h3>
        {history.isPending ? (
          <p className="text-xs text-slate-500">불러오는 중…</p>
        ) : history.isError ? (
          <p role="alert" className="text-xs text-red-600">{history.error.message}</p>
        ) : history.data.length === 0 ? (
          <p className="text-xs text-slate-500">실행 이력이 없습니다</p>
        ) : (
          <ul className="space-y-1">
            {history.data.map((r) => (
              <li key={r.id}>
                <button
                  type="button"
                  aria-current={activeRun?.id === r.id ? 'true' : undefined}
                  onClick={() => onSelectRun({ id: r.id, live: r.status === 'running' })}
                  className={`w-full rounded px-2 py-1 text-left text-xs ${
                    activeRun?.id === r.id ? 'bg-indigo-50 text-indigo-700' : 'hover:bg-slate-100'
                  }`}
                >
                  {RUN_STATUS_TEXT[r.status]} · {fmtTime(r.startedAt)}
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
      <div className="min-w-0 flex-1 overflow-y-auto p-3">
        {!activeRun ? (
          <p className="text-sm text-slate-500">실행하거나 이력에서 런을 선택하세요</p>
        ) : fetched.isError ? (
          <p role="alert" className="text-sm text-red-600">{fetched.error.message}</p>
        ) : run ? (
          <RunDetails run={run} connection={connection} />
        ) : (
          <p className="text-sm text-slate-500">{connection || '불러오는 중…'}</p>
        )}
      </div>
    </section>
  )
}
