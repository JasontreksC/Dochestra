import { ReactFlowProvider } from '@xyflow/react'
import { useEffect, useState } from 'react'
import { Link, useParams } from 'react-router'
import type { ValidationIssue } from '@/bindings/ValidationIssue'
import { ApiError, isValidationError } from '@/api/client.ts'
import {
  useNodeCatalog,
  useStartRun,
  useUpdateWorkflow,
  useValidateWorkflow,
  useWorkflow,
} from '@/api/queries.ts'
import Canvas from '@/components/canvas/Canvas.tsx'
import NodePalette from '@/components/canvas/NodePalette.tsx'
import Inspector from '@/components/panels/Inspector.tsx'
import RunPanel, { type ActiveRun } from '@/components/panels/RunPanel.tsx'
import Sidebar from '@/components/Sidebar.tsx'
import { toGraph } from '@/lib/graphMapping.ts'
import { formatIssue, issueNodeIds } from '@/lib/issues.ts'
import { useEditorStore } from '@/store/editorStore.ts'

type Message =
  | { kind: 'issues'; issues: ValidationIssue[] }
  | { kind: 'error'; text: string }
  | { kind: 'ok'; text: string }
  | null

function Toolbar({
  workflowId,
  onRunStarted,
}: {
  workflowId: string
  onRunStarted: (runId: string) => void
}) {
  const dirty = useEditorStore((s) => s.dirty)
  const runStatus = useEditorStore((s) => s.runStatus)
  const update = useUpdateWorkflow()
  const validate = useValidateWorkflow()
  const startRun = useStartRun()
  const [message, setMessage] = useState<Message>(null)
  const [awaitingRun, setAwaitingRun] = useState(false)

  const busy = update.isPending || validate.isPending || startRun.isPending
  const running = runStatus === 'running' || awaitingRun

  function show(e: unknown) {
    if (isValidationError(e) && e.issues.length > 0) {
      setMessage({ kind: 'issues', issues: e.issues })
      useEditorStore.getState().setIssues(issueNodeIds(e.issues))
    } else {
      setMessage({ kind: 'error', text: e instanceof Error ? e.message : '오류가 발생했습니다' })
    }
  }

  /** Saves pending edits first; returns false (after showing the error) when saving fails. */
  async function ensureSaved(): Promise<boolean> {
    const { dirty: isDirty, nodes, edges, markSaved } = useEditorStore.getState()
    if (!isDirty) return true
    try {
      await update.mutateAsync({ id: workflowId, graph: toGraph(nodes, edges) })
      markSaved()
      return true
    } catch (e) {
      show(e)
      return false
    }
  }

  async function onSave() {
    setMessage(null)
    useEditorStore.getState().clearIssues()
    await ensureSaved()
  }

  async function onValidate() {
    setMessage(null)
    useEditorStore.getState().clearIssues()
    if (!(await ensureSaved())) return
    try {
      const report = await validate.mutateAsync(workflowId)
      if (report.ok) {
        setMessage({ kind: 'ok', text: '검증 통과' })
      } else {
        setMessage({ kind: 'issues', issues: report.issues })
        useEditorStore.getState().setIssues(issueNodeIds(report.issues))
      }
    } catch (e) {
      show(e)
    }
  }

  async function onRun() {
    setMessage(null)
    useEditorStore.getState().clearIssues()
    if (!(await ensureSaved())) return
    setAwaitingRun(true)
    try {
      const created = await startRun.mutateAsync(workflowId)
      onRunStarted(created.runId)
    } catch (e) {
      show(e)
    } finally {
      setAwaitingRun(false)
    }
  }

  const nodeLabel = (id: string) => {
    const s = useEditorStore.getState()
    const kind = s.nodes.find((n) => n.id === id)?.data.kind
    return s.catalog.find((c) => c.kind === kind)?.label ?? id
  }

  const btn = 'rounded px-3 py-1 text-sm disabled:opacity-50'
  return (
    <div className="flex items-start gap-3">
      {dirty && <span className="mt-1 text-xs text-amber-600">저장되지 않음</span>}
      <button
        type="button"
        onClick={() => void onSave()}
        disabled={!dirty || busy}
        className={`${btn} bg-indigo-600 text-white`}
      >
        저장
      </button>
      <button
        type="button"
        onClick={() => void onValidate()}
        disabled={busy}
        className={`${btn} border border-slate-300`}
      >
        검증
      </button>
      <button
        type="button"
        onClick={() => void onRun()}
        disabled={busy || running}
        className={`${btn} bg-green-600 text-white`}
      >
        실행
      </button>
      {message?.kind === 'ok' && (
        <p role="status" className="mt-1 text-xs text-green-700">
          {message.text}
        </p>
      )}
      {message?.kind === 'error' && (
        <p role="alert" className="mt-1 max-w-lg text-xs text-red-600">
          {message.text}
        </p>
      )}
      {message?.kind === 'issues' && (
        <ul role="alert" className="max-h-24 max-w-lg overflow-y-auto text-xs text-red-600">
          {message.issues.map((issue, i) => (
            <li key={i}>
              {issue.code}: {formatIssue(issue, nodeLabel)}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function WorkflowView({ workflowId }: { workflowId: string }) {
  const workflow = useWorkflow(workflowId)
  const catalog = useNodeCatalog()
  const load = useEditorStore((s) => s.load)
  const reset = useEditorStore((s) => s.reset)
  const loadedId = useEditorStore((s) => s.workflowId)
  const [activeRun, setActiveRun] = useState<ActiveRun | null>(null)

  const wf = workflow.data
  const cat = catalog.data
  // Load by workflow id only, so a refetch after saving never overwrites in-progress edits.
  useEffect(() => {
    if (wf && cat && useEditorStore.getState().workflowId !== wf.id) load(wf, cat)
  }, [wf, cat, load])
  useEffect(() => reset, [reset])

  if (workflow.isPending || catalog.isPending)
    return <p className="p-6 text-slate-500">불러오는 중…</p>

  const error = workflow.error ?? catalog.error
  if (error) {
    if (error instanceof ApiError && error.code === 'NOT_FOUND') {
      return (
        <div className="p-6">
          <p className="text-slate-700">워크플로를 찾을 수 없습니다</p>
          <Link to="/editor" className="text-sm text-indigo-600 underline">
            에디터로 돌아가기
          </Link>
        </div>
      )
    }
    return (
      <p role="alert" className="p-6 text-red-600">
        {error.message}
      </p>
    )
  }

  return (
    <>
      <header className="flex items-center justify-between border-b border-slate-200 bg-white px-4 py-3">
        <h1 className="text-lg font-semibold text-slate-900">{workflow.data?.name}</h1>
        <Toolbar workflowId={workflowId} onRunStarted={(id) => setActiveRun({ id, live: true })} />
      </header>
      <div data-testid="canvas-area" className="flex min-h-0 flex-1">
        {loadedId === workflowId && (
          <ReactFlowProvider>
            <NodePalette />
            <div className="min-w-0 flex-1">
              <Canvas />
            </div>
            <Inspector />
          </ReactFlowProvider>
        )}
      </div>
      <RunPanel workflowId={workflowId} activeRun={activeRun} onSelectRun={setActiveRun} />
    </>
  )
}

export default function EditorPage() {
  const { workflowId } = useParams()

  return (
    <div className="flex h-screen">
      <Sidebar />
      <main className="flex min-w-0 flex-1 flex-col">
        {workflowId ? (
          <WorkflowView key={workflowId} workflowId={workflowId} />
        ) : (
          <p className="p-6 text-slate-500">왼쪽에서 워크플로를 선택하거나 새로 만드세요</p>
        )}
      </main>
    </div>
  )
}
