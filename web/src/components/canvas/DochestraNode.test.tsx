import { ReactFlowProvider } from '@xyflow/react'
import { act, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import type { Workflow } from '@/bindings/Workflow'
import { CATALOG } from '@/test/utils.tsx'
import { useEditorStore } from '@/store/editorStore.ts'
import Canvas from './Canvas.tsx'

function workflow(kind: 'extract' | 'summary'): Workflow {
  return {
    id: 'w',
    name: 'n',
    createdAt: '',
    updatedAt: '',
    graph: {
      nodes: [{ id: 'n1', kind, position: { x: 0, y: 0 }, params: {}, isOutput: true }],
      edges: [],
    },
  }
}

function renderCanvas(kind: 'extract' | 'summary') {
  useEditorStore.getState().reset()
  useEditorStore.getState().load(workflow(kind), CATALOG)
  return render(
    <ReactFlowProvider>
      <div style={{ width: 800, height: 600 }}>
        <Canvas />
      </div>
    </ReactFlowProvider>,
  )
}

const handleIds = (container: HTMLElement, type: 'target' | 'source') =>
  [...container.querySelectorAll(`.react-flow__handle.${type === 'target' ? 'target' : 'source'}`)].map(
    (h) => h.getAttribute('data-handleid'),
  )

beforeEach(() => useEditorStore.getState().reset())

describe('DochestraNode', () => {
  it('renders 1 input and 4 output handles for extract', () => {
    const { container } = renderCanvas('extract')
    expect(handleIds(container, 'target')).toEqual(['pdf'])
    expect(handleIds(container, 'source')).toEqual(['md', 'json', 'tables', 'images'])
    expect(screen.getByText('추출')).toBeInTheDocument()
    expect(screen.getByText('출력')).toBeInTheDocument()
    expect(screen.getByText(/표 · LIST\[CSV\]/)).toBeInTheDocument()
  })

  it('marks optional sockets', () => {
    renderCanvas('summary')
    expect(screen.getByText(/이미지 · LIST\[JPG\]/)).toHaveTextContent('(선택)')
    for (const row of screen.getAllByText(/^MD · MD$/)) expect(row).not.toHaveTextContent('(선택)')
  })

  it('colors handles by data type and uses square handles for lists', () => {
    const { container } = renderCanvas('extract')
    const tables = container.querySelector('[data-handleid="tables"]') as HTMLElement
    const md = container.querySelector('.source[data-handleid="md"]') as HTMLElement
    expect(tables.style.background).toMatch(/#30a46c|rgb\(48, 164, 108\)/i)
    expect(tables.style.borderRadius).toBe('2px')
    expect(md.style.background).toMatch(/#3e63dd|rgb\(62, 99, 221\)/i)
  })
})

describe('DochestraNode run / issue overlays', () => {
  const statusOf = (container: HTMLElement) =>
    container.querySelector('.react-flow__node[data-id="n1"] [data-status]')

  it('shows a badge and data-status for each run status', () => {
    const cases = [
      ['pending', '대기'],
      ['running', '실행 중'],
      ['succeeded', '완료'],
      ['failed', '실패'],
      ['skipped', '건너뜀'],
    ] as const
    for (const [status, text] of cases) {
      const { container, unmount } = renderCanvas('extract')
      act(() => useEditorStore.getState().setRunState({ n1: status }, 'running'))
      expect(statusOf(container)).toHaveAttribute('data-status', status)
      expect(screen.getByText(text, { selector: 'span' })).toBeInTheDocument()
      unmount()
    }
  })

  it('has no status attribute without a run and dims nodes outside a shown run', () => {
    const { container } = renderCanvas('extract')
    const root = container.querySelector('.react-flow__node[data-id="n1"] > div') as HTMLElement
    expect(root).not.toHaveAttribute('data-status')
    expect(root.className).not.toContain('opacity-50')
    act(() => useEditorStore.getState().setRunState({ other: 'running' }, 'running'))
    expect(root.className).toContain('opacity-50')
  })

  it('highlights nodes with validation issues', () => {
    const { container } = renderCanvas('extract')
    const root = container.querySelector('.react-flow__node[data-id="n1"] > div') as HTMLElement
    act(() => useEditorStore.getState().setIssues(new Set(['n1'])))
    expect(root).toHaveAttribute('data-issue', 'true')
    expect(root.className).toContain('ring-red-300')
    act(() => useEditorStore.getState().clearIssues())
    expect(root).not.toHaveAttribute('data-issue')
  })
})
