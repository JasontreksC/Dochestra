import { useReactFlow, useReactFlowStoreApi } from '@xyflow/react'
import type { NodeKind } from '@/bindings/NodeKind'
import { useNodeCatalog } from '@/api/queries.ts'
import { useEditorStore } from '@/store/editorStore.ts'

// Adapter / source nodes (CONTEXT D-2, D-3), listed under a separate group
const AUXILIARY: ReadonlySet<NodeKind> = new Set<NodeKind>(['pick_pdf', 'pick_jpg', 'json_input'])
const STEP = 24

export default function NodePalette() {
  const catalog = useNodeCatalog()
  const addNode = useEditorStore((s) => s.addNode)
  const { screenToFlowPosition } = useReactFlow()
  const flowStores = useReactFlowStoreApi()

  function add(kind: NodeKind) {
    const { domNode } = flowStores.optionsStore.getState()
    const { width, height } = flowStores.viewportStore.getState()
    const rect = domNode?.getBoundingClientRect()
    const center = screenToFlowPosition({
      x: (rect?.left ?? 0) + width / 2,
      y: (rect?.top ?? 0) + height / 2,
    })
    // Avoid stacking exactly on top of an existing node.
    const taken = useEditorStore.getState().nodes.map((n) => n.position)
    const pos = { ...center }
    while (taken.some((p) => p.x === pos.x && p.y === pos.y)) {
      pos.x += STEP
      pos.y += STEP
    }
    addNode(kind, pos)
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
              onClick={() => add(spec.kind)}
              className="w-full rounded border border-slate-200 bg-white px-2 py-1 text-left text-sm hover:bg-slate-50"
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
      {group('노드', main)}
      {aux.length > 0 && group('보조', aux)}
    </nav>
  )
}
