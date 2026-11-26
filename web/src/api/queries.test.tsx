import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Workflow } from '@/bindings/Workflow'
import { qk, useCreateWorkflow, useUpdateWorkflow, useWorkflow } from './queries.ts'

const WORKFLOW: Workflow = {
  id: 'w1',
  name: 'demo',
  graph: { nodes: [], edges: [] },
  createdAt: '2026-01-01T00:00:00Z',
  updatedAt: '2026-01-01T00:00:00Z',
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

describe('query hooks', () => {
  let queryClient: QueryClient
  let fetchMock: ReturnType<typeof vi.fn>
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  )

  beforeEach(() => {
    queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    fetchMock = vi.fn()
    vi.stubGlobal('fetch', fetchMock)
  })
  afterEach(() => vi.unstubAllGlobals())

  it('useCreateWorkflow invalidates the workflows key on success', async () => {
    fetchMock.mockResolvedValue(json(WORKFLOW, 201))
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries')
    const { result } = renderHook(() => useCreateWorkflow(), { wrapper })

    await act(() => result.current.mutateAsync({ name: 'demo' }))

    expect(fetchMock).toHaveBeenCalledWith(
      '/api/workflows',
      expect.objectContaining({ method: 'POST', body: '{"name":"demo"}' }),
    )
    expect(invalidate).toHaveBeenCalledWith({ queryKey: qk.workflows })
  })

  it('useUpdateWorkflow caches the returned workflow', async () => {
    fetchMock.mockResolvedValue(json({ ...WORKFLOW, name: 'renamed' }))
    const { result } = renderHook(() => useUpdateWorkflow(), { wrapper })

    await act(() => result.current.mutateAsync({ id: 'w1', name: 'renamed' }))

    expect(fetchMock).toHaveBeenCalledWith(
      '/api/workflows/w1',
      expect.objectContaining({ method: 'PUT', body: '{"name":"renamed"}' }),
    )
    expect(queryClient.getQueryData<Workflow>(qk.workflow('w1'))?.name).toBe('renamed')
  })

  it('useWorkflow(undefined) does not fetch', async () => {
    const { result } = renderHook(() => useWorkflow(undefined), { wrapper })
    expect(result.current.fetchStatus).toBe('idle')
    await new Promise((r) => setTimeout(r, 20))
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it('useWorkflow(id) fetches the workflow', async () => {
    fetchMock.mockResolvedValue(json(WORKFLOW))
    const { result } = renderHook(() => useWorkflow('w 1'), { wrapper })
    await waitFor(() => expect(result.current.data).toEqual(WORKFLOW))
    expect(fetchMock).toHaveBeenCalledWith('/api/workflows/w%201', { method: 'GET' })
  })
})
