import { useState, type DragEvent } from 'react'
import {
  Background,
  Controls,
  MiniMap,
  ReactFlow,
  useReactFlow,
  type Connection,
  type Edge,
  type FitViewOptions,
} from '@xyflow/react'
import type { NodeKind } from '@/bindings/NodeKind'
import { checkConnection } from '@/lib/connection.ts'
import { DEFAULT_NODE_SIZE, NODE_DRAG_MIME } from '@/lib/placement.ts'
import type { DochestraFlowNode, TypedFlowEdge } from '@/lib/graphMapping.ts'
import { useEditorStore } from '@/store/editorStore.ts'
import DochestraNode from './DochestraNode.tsx'
import TypedEdge from './TypedEdge.tsx'

// Module-level constants: React Flow warns when these change identity between renders.
const nodeTypes = { dochestra: DochestraNode }
const edgeTypes = { typed: TypedEdge }
const deleteKeys = ['Backspace', 'Delete']
// Never zoom in past 100% when fitting, so a 1–2 node graph is not blown up.
const fitViewOptions: FitViewOptions = { maxZoom: 1, padding: 0.2 }
const MIN_ZOOM = 0.2

const isNodeDrag = (e: DragEvent) => e.dataTransfer.types.includes(NODE_DRAG_MIME)

export default function Canvas() {
  const nodes = useEditorStore((s) => s.nodes)
  const edges = useEditorStore((s) => s.edges)
  const onNodesChange = useEditorStore((s) => s.onNodesChange)
  const onEdgesChange = useEditorStore((s) => s.onEdgesChange)
  const onConnect = useEditorStore((s) => s.onConnect)
  const addNode = useEditorStore((s) => s.addNode)
  const { screenToFlowPosition } = useReactFlow()
  // Fit only a graph that already has nodes: in v13 a fitView requested on an empty graph stays
  // queued and fires on the first added node, yanking the viewport away from the drop point.
  const [fitOnInit] = useState(() => useEditorStore.getState().nodes.length > 0)

  const onDragOver = (e: DragEvent<HTMLDivElement>) => {
    if (!isNodeDrag(e)) return
    e.preventDefault()
    e.dataTransfer.dropEffect = 'move'
  }

  // Drop from the palette: center the new node on the drop point (unknown kinds are ignored).
  const onDrop = (e: DragEvent<HTMLDivElement>) => {
    const kind = e.dataTransfer.getData(NODE_DRAG_MIME)
    if (!kind) return
    e.preventDefault()
    if (!useEditorStore.getState().catalog.some((c) => c.kind === kind)) return
    const p = screenToFlowPosition({ x: e.clientX, y: e.clientY })
    addNode(kind as NodeKind, {
      x: p.x - DEFAULT_NODE_SIZE.width / 2,
      y: p.y - DEFAULT_NODE_SIZE.height / 2,
    })
  }

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
    <div data-testid="flow-drop-zone" className="h-full w-full" onDragOver={onDragOver} onDrop={onDrop}>
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
        fitView={fitOnInit}
        fitViewOptions={fitViewOptions}
        minZoom={MIN_ZOOM}
      >
        <Background />
        <Controls fitViewOptions={fitViewOptions} />
        <MiniMap pannable zoomable />
      </ReactFlow>
    </div>
  )
}
