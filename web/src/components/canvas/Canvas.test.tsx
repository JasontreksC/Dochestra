import { ReactFlowProvider } from '@xyflow/react'
import { createEvent, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import type { Workflow } from '@/bindings/Workflow'
import { NODE_DRAG_MIME } from '@/lib/placement.ts'
import { CATALOG } from '@/test/utils.tsx'
import { useEditorStore } from '@/store/editorStore.ts'
import Canvas from './Canvas.tsx'

const EMPTY: Workflow = { id: 'w', name: 'n', createdAt: '', updatedAt: '', graph: { nodes: [], edges: [] } }

/** Minimal DataTransfer stand-in (jsdom has none). */
function fakeDataTransfer(data: Record<string, string>) {
  return {
    types: Object.keys(data),
    getData: (type: string) => data[type] ?? '',
    setData: () => {},
    dropEffect: 'none',
    effectAllowed: 'all',
  }
}

function drop(target: Element, data: Record<string, string>, clientX = 400, clientY = 300) {
  const event = createEvent.drop(target, { dataTransfer: fakeDataTransfer(data) })
  Object.defineProperties(event, { clientX: { value: clientX }, clientY: { value: clientY } })
  fireEvent(target, event)
  return event
}

function renderCanvas() {
  return render(
    <ReactFlowProvider>
      <div style={{ width: 800, height: 600 }}>
        <Canvas />
      </div>
    </ReactFlowProvider>,
  )
}

beforeEach(() => {
  useEditorStore.getState().reset()
  useEditorStore.getState().load(EMPTY, CATALOG)
})

describe('Canvas drag and drop', () => {
  it('dragover with a palette node is accepted (default prevented, move effect)', () => {
    renderCanvas()
    const zone = screen.getByTestId('flow-drop-zone')
    const dt = fakeDataTransfer({ [NODE_DRAG_MIME]: 'extract' })
    const event = createEvent.dragOver(zone, { dataTransfer: dt })
    fireEvent(zone, event)
    expect(event.defaultPrevented).toBe(true)
    expect(dt.dropEffect).toBe('move')
  })

  it('dropping a palette node adds and selects a node of that kind at the drop point', () => {
    renderCanvas()
    const event = drop(screen.getByTestId('flow-drop-zone'), { [NODE_DRAG_MIME]: 'extract' }, 400, 300)
    expect(event.defaultPrevented).toBe(true)
    const { nodes, selectedNodeId, dirty } = useEditorStore.getState()
    expect(nodes).toHaveLength(1)
    const added = nodes[0]!
    expect(added.data.kind).toBe('extract')
    expect(selectedNodeId).toBe(added.id)
    expect(dirty).toBe(true)
    // jsdom: canvas at (0,0), identity viewport -> node is centered on the drop point
    expect(added.position).toEqual({ x: 400 - 130, y: 300 - 60 })
  })

  it('ignores drops with an unknown or absent kind', () => {
    renderCanvas()
    const zone = screen.getByTestId('flow-drop-zone')
    drop(zone, { [NODE_DRAG_MIME]: 'not_a_kind' })
    drop(zone, { 'text/plain': 'extract' })
    expect(useEditorStore.getState().nodes).toHaveLength(0)
    expect(useEditorStore.getState().dirty).toBe(false)
  })
})
