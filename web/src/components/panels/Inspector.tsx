import { useState } from 'react'
import type { NodeKind } from '@/bindings/NodeKind'
import { formatDataType } from '@/lib/socketColors.ts'
import { validateParams } from '@/lib/params.ts'
import { useEditorStore } from '@/store/editorStore.ts'
import BundlePicker from './BundlePicker.tsx'

type ParamRecord = Record<string, unknown>
const asRecord = (p: unknown): ParamRecord =>
  typeof p === 'object' && p !== null && !Array.isArray(p) ? (p as ParamRecord) : {}

function IndexEditor({ params, onChange }: { params: unknown; onChange: (p: unknown) => void }) {
  const initial = asRecord(params).index
  const [text, setText] = useState(typeof initial === 'number' ? String(initial) : '')
  const [error, setError] = useState<string | null>(null)

  return (
    <div>
      <label className="block text-xs text-slate-600">
        index
        <input
          type="number"
          aria-label="index"
          min={0}
          step={1}
          value={text}
          onChange={(e) => {
            const raw = e.target.value
            setText(raw)
            if (/^\d+$/.test(raw)) {
              setError(null)
              onChange({ ...asRecord(params), index: Number(raw) })
            } else {
              setError('0 이상의 정수를 입력하세요')
            }
          }}
          className="mt-1 block w-full rounded border border-slate-300 px-2 py-1 text-sm"
        />
      </label>
      {error && <p className="mt-1 text-xs text-red-600">{error}</p>}
    </div>
  )
}

function JsonEditor({ params, onChange }: { params: unknown; onChange: (p: unknown) => void }) {
  const [text, setText] = useState(() => JSON.stringify(asRecord(params).value ?? null, null, 2))
  const [error, setError] = useState<string | null>(null)

  return (
    <div>
      <label className="block text-xs text-slate-600">
        JSON 값
        <textarea
          aria-label="JSON 값"
          value={text}
          rows={8}
          spellCheck={false}
          onChange={(e) => {
            const raw = e.target.value
            setText(raw)
            try {
              const value: unknown = JSON.parse(raw)
              setError(null)
              onChange({ ...asRecord(params), value })
            } catch {
              setError('올바른 JSON 이 아닙니다')
            }
          }}
          className="mt-1 block w-full rounded border border-slate-300 px-2 py-1 font-mono text-xs"
        />
      </label>
      {error && <p className="mt-1 text-xs text-red-600">{error}</p>}
    </div>
  )
}

function ParamEditor({
  kind,
  params,
  onChange,
}: {
  kind: NodeKind
  params: unknown
  onChange: (p: unknown) => void
}) {
  switch (kind) {
    case 'bundle': {
      const id = asRecord(params).bundleId
      return (
        <BundlePicker
          value={typeof id === 'string' ? id : null}
          onChange={(bundleId) => onChange({ ...asRecord(params), bundleId })}
        />
      )
    }
    case 'pick_pdf':
    case 'pick_jpg':
      return <IndexEditor params={params} onChange={onChange} />
    case 'json_input':
      return <JsonEditor params={params} onChange={onChange} />
    default:
      return <p className="text-xs text-slate-500">설정할 파라미터가 없습니다</p>
  }
}

export default function Inspector() {
  const node = useEditorStore((s) => s.nodes.find((n) => n.id === s.selectedNodeId))
  const spec = useEditorStore((s) => s.catalog.find((c) => c.kind === node?.data.kind))
  const setOutput = useEditorStore((s) => s.setOutput)
  const updateNodeParams = useEditorStore((s) => s.updateNodeParams)
  const removeNode = useEditorStore((s) => s.removeNode)

  const shell = (children: React.ReactNode) => (
    <aside aria-label="인스펙터" className="w-80 shrink-0 overflow-y-auto border-l border-slate-200 bg-white p-4">
      {children}
    </aside>
  )

  if (!node) return shell(<p className="text-sm text-slate-500">노드를 선택하세요</p>)
  if (!spec) return shell(<p className="text-sm text-red-600">알 수 없는 노드</p>)

  const bundleIdValue = asRecord(node.data.params).bundleId
  const needsUpload =
    node.data.kind === 'bundle' && (bundleIdValue === undefined || bundleIdValue === null || bundleIdValue === '')
  const warning = needsUpload ? null : validateParams(node.data.kind, node.data.params)

  return shell(
    <div className="space-y-4">
      <div>
        <h2 className="text-base font-semibold text-slate-900">{spec.label}</h2>
        <p className="text-sm text-slate-600">{spec.description}</p>
        <p className="mt-1 break-all text-[10px] text-slate-400">{node.id}</p>
      </div>

      <div className="space-y-1 text-xs text-slate-600">
        {spec.inputs.length > 0 && (
          <p>입력: {spec.inputs.map((s) => `${s.label}(${formatDataType(s.dataType)})`).join(', ')}</p>
        )}
        {spec.outputs.length > 0 && (
          <p>출력: {spec.outputs.map((s) => `${s.label}(${formatDataType(s.dataType)})`).join(', ')}</p>
        )}
      </div>

      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          aria-label="출력 노드로 지정"
          checked={node.data.isOutput}
          onChange={(e) => setOutput(node.id, e.target.checked)}
        />
        출력 노드로 지정
      </label>

      <section>
        <h3 className="mb-2 text-xs font-semibold text-slate-500">파라미터</h3>
        <ParamEditor
          key={node.id}
          kind={node.data.kind}
          params={node.data.params}
          onChange={(p) => updateNodeParams(node.id, p)}
        />
        {needsUpload && (
          <p className="mt-2 text-xs text-slate-500">PDF 를 업로드하면 번들이 자동으로 만들어집니다</p>
        )}
        {warning && (
          <p role="alert" className="mt-2 text-xs text-amber-600">
            {warning}
          </p>
        )}
      </section>

      <button
        type="button"
        onClick={() => removeNode(node.id)}
        className="rounded border border-red-300 px-3 py-1 text-sm text-red-600 hover:bg-red-50"
      >
        노드 삭제
      </button>
    </div>,
  )
}
