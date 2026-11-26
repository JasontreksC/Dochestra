import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render } from '@testing-library/react'
import type { ReactElement } from 'react'
import { MemoryRouter, Route, Routes } from 'react-router'
import { vi } from 'vitest'

interface Options {
  route?: string
  /** When set, `ui` is rendered under `<Route path={path}>` so route params work. */
  path?: string
}

export function renderWithProviders(ui: ReactElement, { route = '/', path }: Options = {}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  })
  const result = render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={[route]}>
        {path ? (
          <Routes>
            <Route path={path} element={ui} />
          </Routes>
        ) : (
          ui
        )}
      </MemoryRouter>
    </QueryClientProvider>,
  )
  return { ...result, queryClient }
}

type Responder = Response | ((init: RequestInit | undefined) => Response)

export function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

/**
 * Stubs global fetch. Keys are `"METHOD /api/path"`; unmatched requests fail the test with a 500.
 * Returns the mock so tests can assert on calls.
 */
export function stubApi(routes: Record<string, Responder>) {
  const mock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const key = `${init?.method ?? 'GET'} ${String(input)}`
    const responder = routes[key]
    if (!responder) return new Response(`unstubbed request: ${key}`, { status: 500 })
    return typeof responder === 'function' ? responder(init) : responder.clone()
  })
  vi.stubGlobal('fetch', mock)
  return mock
}

export function callsTo(mock: ReturnType<typeof stubApi>, key: string) {
  return mock.mock.calls.filter(([url, init]) => `${init?.method ?? 'GET'} ${String(url)}` === key)
}

// ---- node catalog fixture (CONTEXT §3.3) ----
import type { DataType } from '@/bindings/DataType'
import type { NodeSpec } from '@/bindings/NodeSpec'
import type { ScalarType } from '@/bindings/ScalarType'

const dt = (scalar: ScalarType, list = false): DataType => ({ scalar, list })
const sock = (id: string, label: string, dataType: DataType, required = true) => ({
  id,
  label,
  dataType,
  required,
})

export const CATALOG: NodeSpec[] = [
  {
    kind: 'bundle',
    label: 'PDF 번들',
    description: 'PDF 번들',
    inputs: [],
    outputs: [sock('files', '파일', dt('PDF', true))],
    defaultParams: { bundleId: null },
  },
  {
    kind: 'pick_pdf',
    label: 'PDF 선택',
    description: '리스트에서 PDF 하나',
    inputs: [sock('files', '파일', dt('PDF', true))],
    outputs: [sock('pdf', 'PDF', dt('PDF'))],
    defaultParams: { index: 0 },
  },
  {
    kind: 'extract',
    label: '추출',
    description: 'PDF 추출',
    inputs: [sock('pdf', 'PDF', dt('PDF'))],
    outputs: [
      sock('md', 'MD', dt('MD')),
      sock('json', 'JSON', dt('JSON')),
      sock('tables', '표', dt('CSV', true)),
      sock('images', '이미지', dt('JPG', true)),
    ],
    defaultParams: {},
  },
  {
    kind: 'translate',
    label: '번역',
    description: '번역',
    inputs: [sock('md', 'MD', dt('MD')), sock('settings', '설정', dt('JSON'))],
    outputs: [sock('md', 'MD', dt('MD'))],
    defaultParams: {},
  },
  {
    kind: 'summary',
    label: '요약',
    description: '요약',
    inputs: [sock('md', 'MD', dt('MD')), sock('images', '이미지', dt('JPG', true), false)],
    outputs: [sock('md', 'MD', dt('MD'))],
    defaultParams: {},
  },
  {
    kind: 'json_input',
    label: 'JSON 입력',
    description: 'JSON 값',
    inputs: [],
    outputs: [sock('json', 'JSON', dt('JSON'))],
    defaultParams: { value: {} },
  },
]
