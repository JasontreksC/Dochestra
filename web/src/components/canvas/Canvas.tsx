import { Background, Controls, MiniMap, ReactFlow, type Connection, type Edge } from '@xyflow/react'
import { checkConnection } from '@/lib/connection.ts'
import type { DochestraFlowNode, TypedFlowEdge } from '@/lib/graphMapping.ts'
import { useEditorStore } from '@/store/editorStore.ts'
import DochestraNode from './DochestraNode.tsx'
import TypedEdge from './TypedEdge.tsx'

// Module-level constants: React Flow warns when these change identity between renders.
const nodeTypes = { dochestra: DochestraNode }
const edgeTypes = { typed: TypedEdge }
const deleteKeys = ['Backspace', 'Delete']

export default function Canvas() {
  const nodes = useEditorStore((s) => s.nodes)
  const edges = useEditorStore((s) => s.edges)
  const onNodesChange = useEditorStore((s) => s.onNodesChange)
  const onEdgesChange = useEditorStore((s) => s.onEdgesChange)
  const onConnect = useEditorStore((s) => s.onConnect)

  // Read the latest graph at validation time instead of closing over render-time values.
  const isValidConnection = (c: Connection | Edge) => {
    const s = useEditorStore.getState()
    return checkConnection(
      {
        source: c.source,
        sourceHandle: c.sourceHandle ?? null,
        target: c.target,
        targetHandle: c.targetHandle ?? null,
      },
      s.nodes.map((n) => ({ id: n.id, kind: n.data.kind })),
      s.edges,
      s.catalog,
    ).ok
  }

  return (
    <ReactFlow<DochestraFlowNode, TypedFlowEdge>
      nodes={nodes}
      edges={edges}
      nodeTypes={nodeTypes}
      edgeTypes={edgeTypes}
      onNodesChange={onNodesChange}
      onEdgesChange={onEdgesChange}
      onConnect={onConnect}
      isValidConnection={isValidConnection}
      deleteKeyCode={deleteKeys}
      fitView
    >
      <Background />
      <Controls />
      <MiniMap pannable zoomable />
    </ReactFlow>
  )
}
