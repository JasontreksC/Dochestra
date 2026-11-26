import type { Edge, Node } from '@xyflow/react'
import type { DataType } from '@/bindings/DataType'
import type { NodeKind } from '@/bindings/NodeKind'
import type { NodeSpec } from '@/bindings/NodeSpec'
import type { WorkflowGraph } from '@/bindings/WorkflowGraph'

export type DochestraNodeData = { kind: NodeKind; params: unknown; isOutput: boolean }
export type DochestraFlowNode = Node<DochestraNodeData, 'dochestra'>
export type TypedEdgeData = { dataType: DataType }
export type TypedFlowEdge = Edge<TypedEdgeData, 'typed'>

export function toFlow(
  graph: WorkflowGraph,
  catalog: NodeSpec[],
): { nodes: DochestraFlowNode[]; edges: TypedFlowEdge[] } {
  const nodes: DochestraFlowNode[] = graph.nodes.map((n) => ({
    id: n.id,
    type: 'dochestra',
    position: { x: n.position.x, y: n.position.y },
    data: { kind: n.kind, params: n.params, isOutput: n.isOutput },
  }))

  const kindOf = new Map(graph.nodes.map((n) => [n.id, n.kind]))
  const edges: TypedFlowEdge[] = []
  for (const e of graph.edges) {
    const kind = kindOf.get(e.source)
    const socket = catalog
      .find((s) => s.kind === kind)
      ?.outputs.find((o) => o.id === e.sourceHandle)
    if (!socket) {
      console.warn(`toFlow: dropping edge ${e.id} (unknown source socket)`)
      continue
    }
    edges.push({
      id: e.id,
      type: 'typed',
      source: e.source,
      sourceHandle: e.sourceHandle,
      target: e.target,
      targetHandle: e.targetHandle,
      data: { dataType: socket.dataType },
    })
  }
  return { nodes, edges }
}

export function toGraph(nodes: DochestraFlowNode[], edges: TypedFlowEdge[]): WorkflowGraph {
  return {
    nodes: nodes.map((n) => ({
      id: n.id,
      kind: n.data.kind,
      position: { x: n.position.x, y: n.position.y },
      params: n.data.params,
      isOutput: n.data.isOutput,
    })),
    edges: edges.flatMap((e) =>
      e.sourceHandle && e.targetHandle
        ? [
            {
              id: e.id,
              source: e.source,
              sourceHandle: e.sourceHandle,
              target: e.target,
              targetHandle: e.targetHandle,
            },
          ]
        : [],
    ),
  }
}
