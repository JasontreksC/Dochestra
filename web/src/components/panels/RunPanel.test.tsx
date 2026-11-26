import { screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { RunDetail } from '@/bindings/RunDetail'
import { useEditorStore } from '@/store/editorStore.ts'
import { CATALOG, callsTo, jsonResponse, renderWithProviders, stubApi } from '@/test/utils.tsx'
import RunPanel from './RunPanel.tsx'

const RUN: RunDetail = {
  id: 'r1',
  workflowId: 'w1',
  status: 'failed',
  startedAt: '2026-01-01T00:00:00Z',
  finishedAt: '2026-01-01T00:01:00Z',
  nodes: [
    {
      nodeId: 'x',
      kind: 'extract',
      status: 'succeeded',
      error: null,
      startedAt: null,
      finishedAt: null,
      outputs: [
        {
          socketId: 'md',
          dataType: { scalar: 'MD', list: false },
          files: [{ index: 0, fileName: 'md.md', url: '/api/runs/r1/artifacts/x/md?index=0' }],
        },
        {
          socketId: 'images',
          dataType: { scalar: 'JPG', list: true },
          files: [{ index: 0, fileName: '0000.jpg', url: '/api/runs/r1/artifacts/x/images?index=0' }],
        },
      ],
    },
    {
      nodeId: 's',
      kind: 'summary',
      status: 'failed',
      error: 'LLM 호출 실패',
      startedAt: null,
      finishedAt: null,
      outputs: [],
    },
  ],
}

const HISTORY = [
  { id: 'r1', status: 'failed', startedAt: '2026-01-01T00:00:00Z', finishedAt: '2026-01-01T00:01:00Z' },
  { id: 'r2', status: 'succeeded', startedAt: '2026-01-02T00:00:00Z', finishedAt: '2026-01-02T00:01:00Z' },
]

beforeEach(() => {
  useEditorStore.getState().reset()
  useEditorStore.setState({ catalog: CATALOG })
})
afterEach(() => vi.unstubAllGlobals())

describe('RunPanel', () => {
  it('prompts when no run is selected and lists history', async () => {
    stubApi({ 'GET /api/workflows/w1/runs': jsonResponse(HISTORY) })
    renderWithProviders(<RunPanel workflowId="w1" activeRun={null} onSelectRun={() => {}} />)
    expect(screen.getByText('실행하거나 이력에서 런을 선택하세요')).toBeInTheDocument()
    expect(await screen.findAllByRole('button', { name: /^(실패|성공) ·/ })).toHaveLength(2)
  })

  it('shows an empty-history message', async () => {
    stubApi({ 'GET /api/workflows/w1/runs': jsonResponse([]) })
    renderWithProviders(<RunPanel workflowId="w1" activeRun={null} onSelectRun={() => {}} />)
    expect(await screen.findByText('실행 이력이 없습니다')).toBeInTheDocument()
  })

  it('renders a finished run: nodes, error, output links, and syncs run state to the store', async () => {
    stubApi({
      'GET /api/workflows/w1/runs': jsonResponse(HISTORY),
      'GET /api/runs/r1': jsonResponse(RUN),
    })
    renderWithProviders(
      <RunPanel workflowId="w1" activeRun={{ id: 'r1', live: false }} onSelectRun={() => {}} />,
    )
    expect(await screen.findByText('LLM 호출 실패')).toBeInTheDocument()
    expect(screen.getByText('런 실패')).toBeInTheDocument()
    const link = screen.getByRole('link', { name: 'md.md' })
    expect(link).toHaveAttribute('href', '/api/runs/r1/artifacts/x/md?index=0')
    expect(link).toHaveAttribute('target', '_blank')
    expect(screen.getByText('images (LIST[JPG])')).toBeInTheDocument()
    expect(useEditorStore.getState().runNodeStatus).toEqual({ x: 'succeeded', s: 'failed' })
    expect(useEditorStore.getState().runStatus).toBe('failed')
  })

  it('previews markdown by fetching the artifact (truncated) and images via <img>', async () => {
    const mock = stubApi({
      'GET /api/workflows/w1/runs': jsonResponse([]),
      'GET /api/runs/r1': jsonResponse(RUN),
      'GET /api/runs/r1/artifacts/x/md?index=0': new Response('# 제목\n' + 'a'.repeat(6000)),
    })
    renderWithProviders(
      <RunPanel workflowId="w1" activeRun={{ id: 'r1', live: false }} onSelectRun={() => {}} />,
    )
    const buttons = await screen.findAllByRole('button', { name: '미리보기' })
    await userEvent.click(buttons[0]!)
    const pre = await screen.findByText(/# 제목/)
    expect(pre.textContent).toHaveLength(5000)
    expect(callsTo(mock, 'GET /api/runs/r1/artifacts/x/md?index=0')).toHaveLength(1)

    await userEvent.click(screen.getByRole('button', { name: '미리보기' }))
    const img = screen.getByRole('img', { name: '0000.jpg' })
    expect(img).toHaveAttribute('src', '/api/runs/r1/artifacts/x/images?index=0')
    expect(img).toHaveAttribute('loading', 'lazy')
  })

  it('selecting a history entry reports it with live=false for finished runs, live=true for running', async () => {
    stubApi({
      'GET /api/workflows/w1/runs': jsonResponse([
        ...HISTORY,
        { id: 'r3', status: 'running', startedAt: '2026-01-03T00:00:00Z', finishedAt: null },
      ]),
    })
    const onSelect = vi.fn()
    renderWithProviders(<RunPanel workflowId="w1" activeRun={null} onSelectRun={onSelect} />)
    await userEvent.click(await screen.findByRole('button', { name: /^성공 ·/ }))
    expect(onSelect).toHaveBeenLastCalledWith({ id: 'r2', live: false })
    await userEvent.click(screen.getByRole('button', { name: /^실행 중 ·/ }))
    expect(onSelect).toHaveBeenLastCalledWith({ id: 'r3', live: true })
  })

  it('fetches a finished selected run with GET /api/runs/{id} and marks it current', async () => {
    const mock = stubApi({
      'GET /api/workflows/w1/runs': jsonResponse(HISTORY),
      'GET /api/runs/r2': jsonResponse({ ...RUN, id: 'r2', status: 'succeeded', nodes: [] }),
    })
    renderWithProviders(
      <RunPanel workflowId="w1" activeRun={{ id: 'r2', live: false }} onSelectRun={() => {}} />,
    )
    expect(await screen.findByText('런 성공', { selector: 'span' })).toBeInTheDocument()
    expect(callsTo(mock, 'GET /api/runs/r2')).toHaveLength(1)
    expect(screen.getByRole('button', { name: /^성공 ·/ })).toHaveAttribute('aria-current', 'true')
  })
})
