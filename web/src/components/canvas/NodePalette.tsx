import type { DragEvent } from 'react'
import { useReactFlow, useReactFlowStoreApi } from '@xyflow/react'
import type { NodeKind } from '@/bindings/NodeKind'
import { useNodeCatalog } from '@/api/queries.ts'
import {
  DEFAULT_NODE_SIZE,
  NODE_DRAG_MIME,
  contains,
  findFreePosition,
  nodeBox,
} from '@/lib/placement.ts'
import { useEditorStore } from '@/store/editorStore.ts'

// Adapter / source nodes (CONTEXT D-2, D-3), listed under a separate group
const AUXILIARY: ReadonlySet<NodeKind> = new Set<NodeKind>(['pick_pdf', 'pick_jpg', 'json_input'])

function onDragStart(event: DragEvent<HTMLButtonElement>, kind: NodeKind) {
  event.dataTransfer.setData(NODE_DRAG_MIME, kind)
  event.dataTransfer.effectAllowed = 'move'
}

export default function NodePalette() {
  const catalog = useNodeCatalog()
  const addNode = useEditorStore((s) => s.addNode)
  const { screenToFlowPosition, setCenter, getZoom } = useReactFlow()
  const flowStores = useReactFlowStoreApi()

  /** Click-to-add: near the visible canvas center, shifted until it overlaps no existing node. */
  function add(kind: NodeKind) {
    const { domNode } = flowStores.optionsStore.getState()
    const { width, height } = flowStores.viewportStore.getState()
    const rect = domNode?.getBoundingClientRect()
    const left = rect?.left ?? 0
    const top = rect?.top ?? 0
    const center = screenToFlowPosition({ x: left + width / 2, y: top + height / 2 })
    const size = DEFAULT_NODE_SIZE
    const existing = useEditorStore.getState().nodes.map(nodeBox)
    const pos = findFreePosition({ x: center.x - size.width / 2, y: center.y - size.height / 2 }, existing)
    if (addNode(kind, pos) === null) return

    // Pan (keeping the zoom) when the new node landed outside the visible area.
    const tl = screenToFlowPosition({ x: left, y: top })
    const br = screenToFlowPosition({ x: left + width, y: top + height })
    const visible = { x: tl.x, y: tl.y, width: br.x - tl.x, height: br.y - tl.y }
    if (!contains(visible, { ...pos, ...size })) {
      void setCenter(pos.x + size.width / 2, pos.y + size.height / 2, { zoom: getZoom(), duration: 200 })
    }
  }

  if (catalog.isPending) return <p className="p-3 text-sm text-slate-500">불러오는 중…</p>
  if (catalog.isError)
    return (
      <p role="alert" className="p-3 text-sm text-red-600">
        {catalog.error.message}
      </p>
    )

  const main = catalog.data.filter((s) => !AUXILIARY.has(s.kind))
  const aux = catalog.data.filter((s) => AUXILIARY.has(s.kind))

  const group = (title: string, items: typeof main) => (
    <section>
      <h3 className="px-1 pb-1 text-xs font-semibold text-slate-500">{title}</h3>
      <ul className="space-y-1">
        {items.map((spec) => (
          <li key={spec.kind}>
            <button
              type="button"
              title={spec.description}
              draggable
              onDragStart={(e) => onDragStart(e, spec.kind)}
              onClick={() => add(spec.kind)}
              className="w-full cursor-grab rounded border border-slate-200 bg-white px-2 py-1 text-left text-sm hover:bg-slate-50 active:cursor-grabbing"
            >
              {spec.label}
            </button>
          </li>
        ))}
      </ul>
    </section>
  )

  return (
    <nav aria-label="노드 팔레트" className="w-44 shrink-0 space-y-4 overflow-y-auto border-r border-slate-200 bg-slate-50 p-3">
      <p className="text-xs text-slate-500">클릭하거나 캔버스로 끌어다 놓으세요</p>
      {group('노드', main)}
      {aux.length > 0 && group('보조', aux)}
    </nav>
  )
}
