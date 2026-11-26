import { act, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { CATALOG, callsTo, jsonResponse, renderWithProviders, stubApi } from '@/test/utils.tsx'
import EditorPage from './EditorPage.tsx'

afterEach(() => vi.unstubAllGlobals())

const WORKFLOW = {
  id: 'abc',
  name: 'My Flow',
  graph: { nodes: [], edges: [] },
  createdAt: '2026-01-01T00:00:00Z',
  updatedAt: '2026-01-01T00:00:00Z',
}

describe('EditorPage', () => {
  it('shows guidance when no workflow is selected', async () => {
    stubApi({ 'GET /api/workflows': jsonResponse([]) })
    renderWithProviders(<EditorPage />, { route: '/editor', path: '/editor' })
    expect(
      screen.getByText('왼쪽에서 워크플로를 선택하거나 새로 만드세요'),
    ).toBeInTheDocument()
    expect(await screen.findByText('워크플로가 없습니다')).toBeInTheDocument()
  })

  it('renders the workflow name and the canvas area', async () => {
    stubApi({
      'GET /api/workflows': jsonResponse([]),
      'GET /api/nodes': jsonResponse(CATALOG),
      'GET /api/workflows/abc/runs': jsonResponse([]),
      'GET /api/workflows/abc': jsonResponse(WORKFLOW),
    })
    renderWithProviders(<EditorPage />, {
      route: '/editor/abc',
      path: '/editor/:workflowId',
    })
    expect(screen.getByText('불러오는 중…', { selector: 'main *' })).toBeInTheDocument()
    expect(await screen.findByRole('heading', { level: 1, name: 'My Flow' })).toBeInTheDocument()
    expect(screen.getByTestId('canvas-area')).toBeInTheDocument()
  })

  it('shows a not-found message with a link back to /editor on 404', async () => {
    stubApi({
      'GET /api/workflows': jsonResponse([]),
      'GET /api/nodes': jsonResponse(CATALOG),
      'GET /api/workflows/abc/runs': jsonResponse([]),
      'GET /api/workflows/abc': jsonResponse(
        { error: { code: 'NOT_FOUND', message: 'workflow not found' } },
        404,
      ),
    })
    renderWithProviders(<EditorPage />, {
      route: '/editor/abc',
      path: '/editor/:workflowId',
    })
    expect(await screen.findByText('워크플로를 찾을 수 없습니다')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: '에디터로 돌아가기' })).toHaveAttribute(
      'href',
      '/editor',
    )
  })

  it('shows other errors in an alert', async () => {
    stubApi({
      'GET /api/workflows': jsonResponse([]),
      'GET /api/nodes': jsonResponse(CATALOG),
      'GET /api/workflows/abc/runs': jsonResponse([]),
      'GET /api/workflows/abc': jsonResponse(
        { error: { code: 'INTERNAL', message: 'boom' } },
        500,
      ),
    })
    renderWithProviders(<EditorPage />, {
      route: '/editor/abc',
      path: '/editor/:workflowId',
    })
    expect(await screen.findByText('boom')).toBeInTheDocument()
  })

  describe('editing', () => {
    const route = { route: '/editor/abc', path: '/editor/:workflowId' }

    it('adds a node from the palette and saves the graph via PUT', async () => {
      const mock = stubApi({
        'GET /api/workflows': jsonResponse([]),
        'GET /api/nodes': jsonResponse(CATALOG),
      'GET /api/workflows/abc/runs': jsonResponse([]),
        'GET /api/workflows/abc': jsonResponse(WORKFLOW),
        'PUT /api/workflows/abc': (init) => {
          const body = JSON.parse(String(init?.body)) as { graph: unknown }
          return jsonResponse({ ...WORKFLOW, graph: body.graph })
        },
      })
      renderWithProviders(<EditorPage />, route)

      const save = await screen.findByRole('button', { name: '저장' })
      expect(save).toBeDisabled()
      await userEvent.click(await screen.findByRole('button', { name: 'PDF 번들' }))
      expect(screen.getByText('저장되지 않음')).toBeInTheDocument()
      expect(save).toBeEnabled()

      await userEvent.click(save)
      await waitFor(() => expect(callsTo(mock, 'PUT /api/workflows/abc')).toHaveLength(1))
      const init = callsTo(mock, 'PUT /api/workflows/abc')[0]?.[1]
      const sent = JSON.parse(String(init?.body)) as {
        graph: { nodes: Array<{ kind: string; params: unknown; isOutput: boolean }>; edges: unknown[] }
      }
      expect(sent.graph.nodes).toHaveLength(1)
      expect(sent.graph.nodes[0]).toMatchObject({ kind: 'bundle', params: { bundleId: null }, isOutput: false })
      await waitFor(() => expect(screen.queryByText('저장되지 않음')).not.toBeInTheDocument())
    })

    it('shows validation issues from a 422 response', async () => {
      stubApi({
        'GET /api/workflows': jsonResponse([]),
        'GET /api/nodes': jsonResponse(CATALOG),
      'GET /api/workflows/abc/runs': jsonResponse([]),
        'GET /api/workflows/abc': jsonResponse(WORKFLOW),
        'PUT /api/workflows/abc': jsonResponse(
          {
            error: {
              code: 'VALIDATION_FAILED',
              message: 'invalid graph',
              issues: [
                {
                  code: 'TYPE_MISMATCH',
                  edgeId: 'e1',
                  expected: { scalar: 'PDF', list: false },
                  actual: { scalar: 'PDF', list: true },
                },
                { code: 'DUPLICATE_ID', id: 'n1' },
              ],
            },
          },
          422,
        ),
      })
      renderWithProviders(<EditorPage />, route)
      await userEvent.click(await screen.findByRole('button', { name: 'PDF 번들' }))
      await userEvent.click(screen.getByRole('button', { name: '저장' }))

      const alert = await screen.findByRole('alert')
      expect(alert).toHaveTextContent('TYPE_MISMATCH: 엣지 e1 타입 불일치 (PDF ← LIST[PDF])')
      expect(alert).toHaveTextContent('DUPLICATE_ID')
      expect(screen.getByText('저장되지 않음')).toBeInTheDocument()
    })
  })

  describe('validate and run', () => {
    const route = { route: '/editor/abc', path: '/editor/:workflowId' }
    const WF = {
      ...WORKFLOW,
      graph: {
        nodes: [
          { id: 'n1', kind: 'bundle', position: { x: 0, y: 0 }, params: { bundleId: null }, isOutput: true },
        ],
        edges: [],
      },
    }
    const base = {
      'GET /api/workflows': jsonResponse([]),
      'GET /api/nodes': jsonResponse(CATALOG),
      'GET /api/workflows/abc': jsonResponse(WF),
      'GET /api/workflows/abc/runs': jsonResponse([]),
    }
    const RUN = {
      id: 'r1',
      workflowId: 'abc',
      status: 'running',
      startedAt: '2026-01-01T00:00:00Z',
      finishedAt: null,
      nodes: [
        { nodeId: 'n1', kind: 'bundle', status: 'pending', error: null, outputs: [], startedAt: null, finishedAt: null },
      ],
    }
    const statusOf = () =>
      document.querySelector('.react-flow__node[data-id="n1"] [data-status]')?.getAttribute('data-status')

    class FakeEventSource {
      static instances: FakeEventSource[] = []
      url: string
      closed = false
      onopen: (() => void) | null = null
      onmessage: ((ev: MessageEvent<string>) => void) | null = null
      onerror: (() => void) | null = null
      constructor(url: string) {
        this.url = url
        FakeEventSource.instances.push(this)
      }
      close() {
        this.closed = true
      }
      emit(ev: unknown) {
        this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(ev) }))
      }
    }

    it('saves a dirty graph first, then starts the run and follows SSE status', async () => {
      FakeEventSource.instances = []
      vi.stubGlobal('EventSource', FakeEventSource)
      const mock = stubApi({
        ...base,
        'PUT /api/workflows/abc': jsonResponse(WF),
        'POST /api/workflows/abc/runs': jsonResponse({ runId: 'r1' }, 202),
      })
      renderWithProviders(<EditorPage />, route)

      await userEvent.click(await screen.findByRole('button', { name: '요약' }))
      await userEvent.click(screen.getByRole('button', { name: '실행' }))

      await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1))
      const order = mock.mock.calls
        .map(([url, init]) => `${init?.method ?? 'GET'} ${String(url)}`)
        .filter((k) => k.startsWith('PUT') || k.startsWith('POST'))
      expect(order).toEqual(['PUT /api/workflows/abc', 'POST /api/workflows/abc/runs'])

      const es = FakeEventSource.instances[0]!
      expect(es.url).toBe('/api/runs/r1/events')
      act(() => es.emit({ type: 'snapshot', run: RUN }))
      await waitFor(() => expect(statusOf()).toBe('pending'))
      expect(screen.getByRole('button', { name: '실행' })).toBeDisabled()

      act(() =>
        es.emit({ type: 'node_status', nodeId: 'n1', status: 'running', error: null, outputs: [] }),
      )
      await waitFor(() => expect(statusOf()).toBe('running'))

      act(() =>
        es.emit({ type: 'node_status', nodeId: 'n1', status: 'succeeded', error: null, outputs: [] }),
      )
      act(() => es.emit({ type: 'run_finished', status: 'succeeded' }))
      await waitFor(() => expect(statusOf()).toBe('succeeded'))
      await waitFor(() => expect(screen.getByRole('button', { name: '실행' })).toBeEnabled())
      expect(es.closed).toBe(true)
    })

    it('does not start the run when the pre-save fails', async () => {
      const mock = stubApi({
        ...base,
        'PUT /api/workflows/abc': jsonResponse({ error: { code: 'INTERNAL', message: '저장 실패' } }, 500),
      })
      renderWithProviders(<EditorPage />, route)
      await userEvent.click(await screen.findByRole('button', { name: '요약' }))
      await userEvent.click(screen.getByRole('button', { name: '실행' }))
      expect(await screen.findByRole('alert')).toHaveTextContent('저장 실패')
      expect(callsTo(mock, 'POST /api/workflows/abc/runs')).toHaveLength(0)
    })

    it('shows issues and highlights nodes when the run is rejected with 422', async () => {
      stubApi({
        ...base,
        'POST /api/workflows/abc/runs': jsonResponse(
          {
            error: {
              code: 'VALIDATION_FAILED',
              message: 'invalid',
              issues: [{ code: 'UNCONNECTED_INPUT', nodeId: 'n1', socketId: 'files' }, { code: 'NO_OUTPUT_NODE' }],
            },
          },
          422,
        ),
      })
      renderWithProviders(<EditorPage />, route)
      await userEvent.click(await screen.findByRole('button', { name: '실행' }))
      const alert = await screen.findByRole('alert')
      expect(alert).toHaveTextContent("PDF 번들: 입력 'files' 이(가) 연결되지 않았습니다")
      expect(alert).toHaveTextContent('출력 노드가 없습니다')
      await waitFor(() =>
        expect(
          document.querySelector('.react-flow__node[data-id="n1"] [data-issue="true"]'),
        ).toBeInTheDocument(),
      )
    })

    it('validate: ok shows a success message; failure lists issues', async () => {
      let ok = true
      stubApi({
        ...base,
        'POST /api/workflows/abc/validate': () =>
          ok
            ? jsonResponse({ ok: true, issues: [] })
            : jsonResponse({ ok: false, issues: [{ code: 'NO_OUTPUT_NODE' }] }),
      })
      renderWithProviders(<EditorPage />, route)
      await userEvent.click(await screen.findByRole('button', { name: '검증' }))
      expect(await screen.findByRole('status')).toHaveTextContent('검증 통과')

      ok = false
      await userEvent.click(screen.getByRole('button', { name: '검증' }))
      expect(await screen.findByRole('alert')).toHaveTextContent('NO_OUTPUT_NODE')
      expect(screen.queryByRole('status')).not.toBeInTheDocument()
    })

    it('validate saves pending edits before validating', async () => {
      const mock = stubApi({
        ...base,
        'PUT /api/workflows/abc': jsonResponse(WF),
        'POST /api/workflows/abc/validate': jsonResponse({ ok: true, issues: [] }),
      })
      renderWithProviders(<EditorPage />, route)
      await userEvent.click(await screen.findByRole('button', { name: '요약' }))
      await userEvent.click(screen.getByRole('button', { name: '검증' }))
      await screen.findByRole('status')
      const order = mock.mock.calls
        .map(([url, init]) => `${init?.method ?? 'GET'} ${String(url)}`)
        .filter((k) => k.startsWith('PUT') || k.startsWith('POST'))
      expect(order).toEqual(['PUT /api/workflows/abc', 'POST /api/workflows/abc/validate'])
    })
  })
})
