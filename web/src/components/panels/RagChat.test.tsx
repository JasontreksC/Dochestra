import { act, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { NodeSpec } from '@/bindings/NodeSpec'
import type { RunDetail } from '@/bindings/RunDetail'
import type { WorkflowGraph } from '@/bindings/WorkflowGraph'
import { useEditorStore } from '@/store/editorStore.ts'
import { useRagChatStore } from '@/store/ragChatStore.ts'
import { CATALOG, callsTo, jsonResponse, renderWithProviders, stubApi } from '@/test/utils.tsx'
import Inspector from './Inspector.tsx'
import RagChat from './RagChat.tsx'

const RAG_SPEC: NodeSpec = {
  kind: 'rag',
  label: 'RAG 챗봇',
  description: 'RAG',
  inputs: [{ id: 'md', label: 'Markdown', dataType: { scalar: 'MD', list: false }, required: true }],
  outputs: [{ id: 'api', label: 'API 정보(JSON)', dataType: { scalar: 'JSON', list: false }, required: true }],
  defaultParams: {},
}
const GRAPH: WorkflowGraph = {
  nodes: [{ id: 'r', kind: 'rag', position: { x: 0, y: 0 }, params: {}, isOutput: true }],
  edges: [],
}
const INDEX = '11111111-2222-4333-8444-555555555555'
const QUERY = `POST /api/rag/${INDEX}/query`
const ART = '/api/runs/run12345678/artifacts/r/api?index=0'

const RUN: RunDetail = {
  id: 'run12345678',
  workflowId: 'w',
  status: 'succeeded',
  startedAt: '',
  finishedAt: null,
  nodes: [
    {
      nodeId: 'r',
      kind: 'rag',
      status: 'succeeded',
      error: null,
      startedAt: null,
      finishedAt: null,
      outputs: [
        {
          socketId: 'api',
          dataType: { scalar: 'JSON', list: false },
          files: [{ index: 0, fileName: 'api.json', url: ART }],
        },
      ],
    },
  ],
}
const API_JSON = {
  type: 'rag_api',
  indexId: INDEX,
  method: 'POST',
  endpoint: `/api/rag/${INDEX}/query`,
  request: {},
  chunkCount: 42,
  embeddingModel: 'baai/bge-m3',
}
const LONG = 'L'.repeat(350)
const ANSWER = {
  answer: 'Revenue was 482 million [1]. Margin grew [2, 3]. Bogus [9].',
  sources: [
    { chunkIndex: 4, content: 'Total revenue 482', score: 0.8765 },
    { chunkIndex: 9, content: LONG, score: 0.5 },
    { chunkIndex: 1, content: 'third', score: 0.1 },
  ],
}

function stubIndex(extra: Parameters<typeof stubApi>[0] = {}) {
  return stubApi({
    'GET /api/workflows/w/runs': jsonResponse([
      { id: RUN.id, status: 'succeeded', startedAt: '', finishedAt: null },
    ]),
    [`GET /api/runs/${RUN.id}`]: jsonResponse(RUN),
    [`GET ${ART}`]: jsonResponse(API_JSON),
    ...extra,
  })
}

const renderChat = () => renderWithProviders(<RagChat nodeId="r" selectedRunId={null} />)
const input = () => screen.getByRole('textbox', { name: '질문 입력' })
const sendBtn = () => screen.getByRole('button', { name: '보내기' })

beforeEach(() => {
  useEditorStore.getState().reset()
  useEditorStore
    .getState()
    .load({ id: 'w', name: 'n', createdAt: '', updatedAt: '', graph: GRAPH }, [...CATALOG, RAG_SPEC])
  useRagChatStore.getState().clearAll()
})
afterEach(() => vi.unstubAllGlobals())

describe('RagChat', () => {
  it('is shown in the inspector for a selected rag node with the index header', async () => {
    stubIndex()
    act(() => useEditorStore.setState({ selectedNodeId: 'r' }))
    renderWithProviders(<Inspector />)
    expect(screen.getByRole('checkbox', { name: '출력 노드로 지정' })).toBeChecked()
    expect(await screen.findByText(`인덱스: 청크 42개 · 실행 run12345`)).toBeInTheDocument()
    expect(input()).toBeInTheDocument()
  })

  it('shows guidance when no run built the index', async () => {
    stubApi({ 'GET /api/workflows/w/runs': jsonResponse([]) })
    renderChat()
    expect(
      await screen.findByText('워크플로를 실행해 RAG 인덱스를 만든 뒤 질문할 수 있습니다'),
    ).toBeInTheDocument()
    expect(screen.queryByRole('textbox', { name: '질문 입력' })).toBeNull()
  })

  it('shows an alert when the api artifact is invalid', async () => {
    stubIndex({ [`GET ${ART}`]: new Response('garbage', { status: 200 }) })
    renderChat()
    expect(await screen.findByRole('alert')).toHaveTextContent('올바른 JSON 이 아닙니다')
  })

  it('sends {question, topK}, renders the answer, citation badges and sources', async () => {
    let body: unknown = null
    const fetchMock = stubIndex({
      [QUERY]: (init) => {
        body = JSON.parse(String(init?.body))
        return jsonResponse(ANSWER)
      },
    })
    renderChat()
    await screen.findByRole('textbox', { name: '질문 입력' })
    expect(sendBtn()).toBeDisabled()
    await userEvent.type(input(), '   ')
    expect(sendBtn()).toBeDisabled()

    await userEvent.selectOptions(screen.getByRole('combobox', { name: 'topK' }), '8')
    await userEvent.clear(input())
    await userEvent.type(input(), '  What was revenue?  ')
    await userEvent.click(sendBtn())

    expect(await screen.findByTestId('rag-answer')).toHaveTextContent('Revenue was 482 million')
    expect(callsTo(fetchMock, QUERY)).toHaveLength(1)
    expect(body).toEqual({ question: 'What was revenue?', topK: 8 })
    expect(screen.getByTestId('rag-question')).toHaveTextContent('What was revenue?')
    expect(input()).toHaveValue('')

    // [1], [2, 3] become badges; [9] (out of range) stays text
    expect(screen.getAllByRole('button', { name: /^근거 \d$/ }).map((b) => b.textContent)).toEqual([
      '1',
      '2',
      '3',
    ])
    expect(screen.getByTestId('rag-answer')).toHaveTextContent('Bogus [9].')

    // collapsed sources list; badge click opens it and expands the cited source
    const toggle = screen.getByRole('button', { name: '근거 3개' })
    expect(toggle).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryAllByTestId('rag-source')).toHaveLength(0)
    await userEvent.click(screen.getByRole('button', { name: '근거 2' }))
    expect(toggle).toHaveAttribute('aria-expanded', 'true')
    const items = screen.getAllByTestId('rag-source')
    expect(items).toHaveLength(3)
    expect(items[0]).toHaveTextContent('청크 #4 · 점수 0.88')
    expect(items[0]).toHaveTextContent('Total revenue 482')
    expect(items[1]).toHaveTextContent(LONG) // expanded by the badge
    expect(within(items[1]!).getByRole('button', { name: '접기' })).toBeInTheDocument()
    await userEvent.click(within(items[1]!).getByRole('button', { name: '접기' }))
    expect(items[1]).not.toHaveTextContent(LONG)
    expect(items[1]).toHaveTextContent(`${'L'.repeat(300)}…`)
  })

  it('shows "답변 생성 중…" while pending and an error bubble on 502, keeping the question', async () => {
    let release: (r: Response) => void = () => {}
    const base = stubIndex()
    vi.stubGlobal(
      'fetch',
      vi.fn((u: RequestInfo | URL, init?: RequestInit) =>
        String(u).startsWith('/api/rag/')
          ? new Promise<Response>((res) => {
              release = res
            })
          : base(u, init),
      ),
    )
    renderChat()
    await userEvent.type(await screen.findByRole('textbox', { name: '질문 입력' }), 'Why?')
    await userEvent.keyboard('{Enter}')
    expect(await screen.findByText('답변 생성 중…')).toBeInTheDocument()
    expect(sendBtn()).toBeDisabled()

    act(() =>
      release(jsonResponse({ error: { code: 'UPSTREAM', message: 'LLM 호출 실패' } }, 502)),
    )
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('502 UPSTREAM')
    expect(alert).toHaveTextContent('LLM 호출 실패')
    expect(screen.queryByText('답변 생성 중…')).toBeNull()
    expect(input()).toHaveValue('Why?')
  })

  it('Enter sends, Shift+Enter inserts a newline', async () => {
    const fetchMock = stubIndex({ [QUERY]: jsonResponse({ answer: 'ok', sources: [] }) })
    renderChat()
    await userEvent.type(await screen.findByRole('textbox', { name: '질문 입력' }), 'line1')
    await userEvent.keyboard('{Shift>}{Enter}{/Shift}line2')
    expect(input()).toHaveValue('line1\nline2')
    expect(callsTo(fetchMock, QUERY)).toHaveLength(0)
    await userEvent.keyboard('{Enter}')
    await screen.findByTestId('rag-answer')
    expect(callsTo(fetchMock, QUERY)).toHaveLength(1)
    expect(JSON.parse(String(callsTo(fetchMock, QUERY)[0]?.[1]?.body))).toEqual({
      question: 'line1\nline2',
      topK: 5,
    })
  })

  it('keeps history across unmount/remount for the same index and clears it on demand', async () => {
    stubIndex({ [QUERY]: jsonResponse({ answer: 'first answer', sources: [] }) })
    const first = renderChat()
    await userEvent.type(await screen.findByRole('textbox', { name: '질문 입력' }), 'q1{Enter}')
    await screen.findByText('first answer')
    first.unmount()

    renderChat()
    expect(await screen.findByText('first answer')).toBeInTheDocument()
    expect(screen.getByTestId('rag-question')).toHaveTextContent('q1')

    await userEvent.click(screen.getByRole('button', { name: '대화 지우기' }))
    expect(screen.queryByText('first answer')).toBeNull()
    expect(useRagChatStore.getState().histories[INDEX]).toBeUndefined()
    await waitFor(() => expect(screen.getByRole('button', { name: '대화 지우기' })).toBeDisabled())
  })
})
