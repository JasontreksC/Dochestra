import type { DataType } from '@/bindings/DataType'
import type { NodeKind } from '@/bindings/NodeKind'
import type { NodeSpec } from '@/bindings/NodeSpec'

export type ConnectionRejectReason =
  | 'missing-handle'
  | 'unknown-node'
  | 'unknown-socket'
  | 'self-loop'
  | 'type-mismatch'
  | 'input-occupied'
  | 'cycle'

export interface ConnectionCandidate {
  source: string
  sourceHandle: string | null
  target: string
  targetHandle: string | null
}

export type ConnectionCheck =
  | { ok: true; dataType: DataType }
  | { ok: false; reason: ConnectionRejectReason }

const reject = (reason: ConnectionRejectReason): ConnectionCheck => ({ ok: false, reason })

/** True when `to` is reachable from `from` following edges source -> target. */
function reachable(from: string, to: string, edges: Array<{ source: string; target: string }>) {
  const seen = new Set<string>()
  const stack = [from]
  while (stack.length > 0) {
    const id = stack.pop() as string
    if (id === to) return true
    if (seen.has(id)) continue
    seen.add(id)
    for (const e of edges) if (e.source === id) stack.push(e.target)
  }
  return false
}

/**
 * Edge connection rules (CONTEXT D-5, §4.1.2): exact type match, one edge per input socket,
 * output fan-out allowed, no cycles.
 */
export function checkConnection(
  c: ConnectionCandidate,
  nodes: Array<{ id: string; kind: NodeKind }>,
  edges: Array<{ source: string; target: string; targetHandle?: string | null }>,
  catalog: NodeSpec[],
): ConnectionCheck {
  if (c.sourceHandle === null || c.targetHandle === null) return reject('missing-handle')

  const sourceNode = nodes.find((n) => n.id === c.source)
  const targetNode = nodes.find((n) => n.id === c.target)
  if (!sourceNode || !targetNode) return reject('unknown-node')

  const output = catalog
    .find((s) => s.kind === sourceNode.kind)
    ?.outputs.find((s) => s.id === c.sourceHandle)
  const input = catalog
    .find((s) => s.kind === targetNode.kind)
    ?.inputs.find((s) => s.id === c.targetHandle)
  if (!output || !input) return reject('unknown-socket')

  if (c.source === c.target) return reject('self-loop')

  if (output.dataType.scalar !== input.dataType.scalar || output.dataType.list !== input.dataType.list)
    return reject('type-mismatch')

  if (edges.some((e) => e.target === c.target && e.targetHandle === c.targetHandle))
    return reject('input-occupied')

  if (reachable(c.target, c.source, edges)) return reject('cycle')

  return { ok: true, dataType: output.dataType }
}
