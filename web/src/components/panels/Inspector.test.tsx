import { act, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { WorkflowGraph } from '@/bindings/WorkflowGraph'
import { useEditorStore } from '@/store/editorStore.ts'
import { CATALOG, jsonResponse, renderWithProviders, stubApi } from '@/test/utils.tsx'
import Inspector from './Inspector.tsx'

const GRAPH: WorkflowGraph = {
  nodes: [
    { id: 'p', kind: 'pick_pdf', position: { x: 0, y: 0 }, params: { index: 1 }, isOutput: false },
    { id: 'j', kind: 'json_input', position: { x: 0, y: 0 }, params: { value: { a: 1 } }, isOutput: false },
    { id: 'x', kind: 'extract', position: { x: 0, y: 0 }, params: {}, isOutput: false },
    { id: 'b', kind: 'bundle', position: { x: 0, y: 0 }, params: { bundleId: null }, isOutput: false },
  ],
  edges: [{ id: 'e1', source: 'b', sourceHandle: 'files', target: 'p', targetHandle: 'files' }],
}

const store = () => useEditorStore.getState()
const select = (id: string | null) => act(() => useEditorStore.setState({ selectedNodeId: id }))
const params = (id: string) => store().nodes.find((n) => n.id === id)?.data.params

beforeEach(() => {
  stubApi({ 'GET /api/bundles': jsonResponse([]) })
  store().reset()
  store().load({ id: 'w', name: 'n', createdAt: '', updatedAt: '', graph: GRAPH }, CATALOG)
})
afterEach(() => vi.unstubAllGlobals())

describe('Inspector', () => {
  it('asks to select a node when none is selected', () => {
    renderWithProviders(<Inspector />)
    expect(screen.getByText('노드를 선택하세요')).toBeInTheDocument()
  })

  it('shows node info and sockets', () => {
    select('x')
    renderWithProviders(<Inspector />)
    expect(screen.getByRole('heading', { name: '추출' })).toBeInTheDocument()
    expect(screen.getByText('PDF 추출')).toBeInTheDocument()
    expect(screen.getByText(/입력: PDF\(PDF\)/)).toBeInTheDocument()
    expect(screen.getByText(/출력: .*표\(LIST\[CSV\]\)/)).toBeInTheDocument()
    expect(screen.getByText('설정할 파라미터가 없습니다')).toBeInTheDocument()
  })

  it('toggles the output flag in the store and marks dirty', async () => {
    select('x')
    renderWithProviders(<Inspector />)
    await userEvent.click(screen.getByRole('checkbox', { name: '출력 노드로 지정' }))
    expect(store().nodes.find((n) => n.id === 'x')?.data.isOutput).toBe(true)
    expect(store().dirty).toBe(true)
  })

  it('edits index: valid values update the store, invalid ones do not', async () => {
    select('p')
    renderWithProviders(<Inspector />)
    const input = screen.getByRole('spinbutton', { name: 'index' })
    expect(input).toHaveValue(1)

    await userEvent.clear(input)
    expect(screen.getByText('0 이상의 정수를 입력하세요')).toBeInTheDocument()
    expect(params('p')).toEqual({ index: 1 })

    await userEvent.type(input, '4')
    expect(params('p')).toEqual({ index: 4 })
    expect(screen.queryByText('0 이상의 정수를 입력하세요')).not.toBeInTheDocument()

    await userEvent.clear(input)
    await userEvent.type(input, '-2')
    expect(screen.getByText('0 이상의 정수를 입력하세요')).toBeInTheDocument()
    expect(params('p')).toEqual({ index: 4 })

    await userEvent.clear(input)
    await userEvent.type(input, '1.5') // '1' is valid on its own; the final '1.5' is not
    expect(params('p')).toEqual({ index: 1 })
    expect(screen.getByText('0 이상의 정수를 입력하세요')).toBeInTheDocument()
  })

  it('edits JSON: valid updates the store, invalid shows an error', async () => {
    select('j')
    renderWithProviders(<Inspector />)
    const area = screen.getByRole('textbox', { name: 'JSON 값' })
    expect(JSON.parse((area as HTMLTextAreaElement).value)).toEqual({ a: 1 })

    await userEvent.clear(area)
    await userEvent.type(area, '{{"b": 2}')
    expect(params('j')).toEqual({ value: { b: 2 } })
    expect(screen.queryByText('올바른 JSON 이 아닙니다')).not.toBeInTheDocument()

    await userEvent.type(area, 'oops')
    expect(screen.getByText('올바른 JSON 이 아닙니다')).toBeInTheDocument()
    expect(params('j')).toEqual({ value: { b: 2 } })
  })

  it('shows the engine-aligned params warning without blocking edits', () => {
    select('b')
    renderWithProviders(<Inspector />)
    expect(screen.getByRole('alert')).toHaveTextContent('bundleId 가 올바른 UUID 가 아닙니다')
  })

  it('resets local editor state when the selection changes', () => {
    select('p')
    renderWithProviders(<Inspector />)
    expect(screen.getByRole('spinbutton', { name: 'index' })).toHaveValue(1)
    select('j')
    expect(screen.queryByRole('spinbutton', { name: 'index' })).not.toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: 'JSON 값' })).toBeInTheDocument()
  })

  it('deletes the node together with its edges', async () => {
    select('p')
    renderWithProviders(<Inspector />)
    await userEvent.click(screen.getByRole('button', { name: '노드 삭제' }))
    expect(store().nodes.map((n) => n.id)).toEqual(['j', 'x', 'b'])
    expect(store().edges).toHaveLength(0)
    expect(store().selectedNodeId).toBeNull()
    expect(store().dirty).toBe(true)
    expect(screen.getByText('노드를 선택하세요')).toBeInTheDocument()
  })
})
