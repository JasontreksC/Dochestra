import { ReactFlowProvider } from '@xyflow/react'
import { createEvent, fireEvent, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Workflow } from '@/bindings/Workflow'
import { NODE_DRAG_MIME, nodeBox, overlaps } from '@/lib/placement.ts'
import { CATALOG, jsonResponse, renderWithProviders, stubApi } from '@/test/utils.tsx'
import { useEditorStore } from '@/store/editorStore.ts'
import NodePalette from './NodePalette.tsx'

const EMPTY: Workflow = { id: 'w', name: 'n', createdAt: '', updatedAt: '', graph: { nodes: [], edges: [] } }

function renderPalette() {
  stubApi({ 'GET /api/nodes': jsonResponse(CATALOG) })
  return renderWithProviders(
    <ReactFlowProvider>
      <NodePalette />
    </ReactFlowProvider>,
  )
}

beforeEach(() => {
  useEditorStore.getState().reset()
  useEditorStore.getState().load(EMPTY, CATALOG)
})
afterEach(() => vi.unstubAllGlobals())

describe('NodePalette', () => {
  it('shows the placement hint and draggable items', async () => {
    renderPalette()
    const item = await screen.findByRole('button', { name: '추출' })
    expect(screen.getByText('클릭하거나 캔버스로 끌어다 놓으세요')).toBeInTheDocument()
    expect(item).toHaveAttribute('draggable', 'true')
    expect(item.className).toContain('cursor-grab')
  })

  it('dragstart puts the node kind on the DataTransfer', async () => {
    renderPalette()
    const item = await screen.findByRole('button', { name: '추출' })
    const setData = vi.fn()
    const dataTransfer = { setData, effectAllowed: 'all' }
    fireEvent(item, createEvent.dragStart(item, { dataTransfer }))
    expect(setData).toHaveBeenCalledWith(NODE_DRAG_MIME, 'extract')
    expect(dataTransfer.effectAllowed).toBe('move')
  })

  it('click-adding selects each new node and never overlaps existing ones', async () => {
    renderPalette()
    const user = userEvent.setup()
    const item = await screen.findByRole('button', { name: '추출' })
    for (let i = 0; i < 3; i++) await user.click(item)

    const { nodes, selectedNodeId, dirty } = useEditorStore.getState()
    expect(nodes).toHaveLength(3)
    expect(dirty).toBe(true)
    expect(selectedNodeId).toBe(nodes[2]!.id)
    expect(nodes.filter((n) => n.selected).map((n) => n.id)).toEqual([nodes[2]!.id])
    const boxes = nodes.map(nodeBox)
    for (let i = 0; i < boxes.length; i++)
      for (let j = i + 1; j < boxes.length; j++) expect(overlaps(boxes[i]!, boxes[j]!)).toBe(false)
  })
})
