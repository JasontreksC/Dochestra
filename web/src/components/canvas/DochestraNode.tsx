import { Handle, Position, type NodeProps } from '@xyflow/react'
import type { NodeStatus } from '@/bindings/NodeStatus'
import type { SocketSpec } from '@/bindings/SocketSpec'
import { formatDataType, socketColor } from '@/lib/socketColors.ts'
import type { DochestraFlowNode } from '@/lib/graphMapping.ts'
import { useEditorStore } from '@/store/editorStore.ts'

function handleStyle(socket: SocketSpec) {
  return {
    background: socketColor(socket.dataType),
    width: 12,
    height: 12,
    borderRadius: socket.dataType.list ? 2 : 9999,
  }
}

function SocketRow({ socket, side }: { socket: SocketSpec; side: 'input' | 'output' }) {
  const isInput = side === 'input'
  return (
    <div
      className={`relative px-3 py-1 text-xs text-slate-700 ${isInput ? 'text-left' : 'text-right'}`}
    >
      <Handle
        type={isInput ? 'target' : 'source'}
        position={isInput ? Position.Left : Position.Right}
        id={socket.id}
        style={handleStyle(socket)}
      />
      {socket.label} · {formatDataType(socket.dataType)}
      {!socket.required && <span className="text-slate-400"> (선택)</span>}
    </div>
  )
}

const STATUS_BADGE: Record<NodeStatus, { text: string; className: string }> = {
  pending: { text: '대기', className: 'bg-slate-200 text-slate-700' },
  running: { text: '실행 중', className: 'animate-pulse bg-blue-500 text-white' },
  succeeded: { text: '완료', className: 'bg-green-600 text-white' },
  failed: { text: '실패', className: 'bg-red-600 text-white' },
  skipped: { text: '건너뜀', className: 'border border-dashed border-slate-400 text-slate-500' },
}

export default function DochestraNode({ id, data, selected }: NodeProps<DochestraFlowNode>) {
  const spec = useEditorStore((s) => s.catalog.find((c) => c.kind === data.kind))
  const hasIssue = useEditorStore((s) => s.issueNodeIds.has(id))
  const status = useEditorStore((s) => s.runNodeStatus[id])
  const runShown = useEditorStore((s) => Object.keys(s.runNodeStatus).length > 0)
  const border = hasIssue
    ? 'border-red-500 ring-2 ring-red-300'
    : selected
      ? 'border-indigo-500 ring-2 ring-indigo-200'
      : 'border-slate-300'
  const dim = runShown && !status ? 'opacity-50' : ''

  if (!spec) {
    return (
      <div className={`rounded-md border bg-white px-3 py-2 text-xs text-red-600 ${border}`}>
        알 수 없는 노드
      </div>
    )
  }

  return (
    <div
      className={`min-w-48 rounded-md border bg-white shadow-sm ${border} ${dim}`}
      data-status={status}
      data-issue={hasIssue || undefined}
    >
      <div className="flex items-center justify-between rounded-t-md bg-slate-100 px-3 py-1.5">
        <span className="text-sm font-semibold text-slate-800">{spec.label}</span>
        <span className="flex items-center gap-1">
          {status && (
            <span className={`rounded px-1.5 text-[10px] ${STATUS_BADGE[status].className}`}>
              {STATUS_BADGE[status].text}
            </span>
          )}
          {data.isOutput && (
            <span className="rounded bg-indigo-600 px-1.5 text-[10px] text-white">출력</span>
          )}
        </span>
      </div>
      <div className="flex justify-between gap-4 py-1">
        <div>
          {spec.inputs.map((s) => (
            <SocketRow key={s.id} socket={s} side="input" />
          ))}
        </div>
        <div>
          {spec.outputs.map((s) => (
            <SocketRow key={s.id} socket={s} side="output" />
          ))}
        </div>
      </div>
    </div>
  )
}
