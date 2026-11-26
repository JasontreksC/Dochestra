import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook } from '@testing-library/react'
import { createElement, type ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { RunDetail } from '@/bindings/RunDetail'
import type { RunEvent } from '@/bindings/RunEvent'
import { applyRunEvent, useRunEvents } from './sse.ts'

const RUN: RunDetail = {
  id: 'r1',
  workflowId: 'w1',
  status: 'running',
  startedAt: '2026-01-01T00:00:00Z',
  finishedAt: null,
  nodes: [
    {
      nodeId: 'a',
      kind: 'bundle',
      status: 'pending',
      error: null,
      outputs: [],
      startedAt: null,
      finishedAt: null,
    },
    {
      nodeId: 'b',
      kind: 'extract',
      status: 'pending',
      error: null,
      outputs: [],
      startedAt: null,
      finishedAt: null,
    },
  ],
}

const OUTPUTS = [
  {
    socketId: 'md',
    dataType: { scalar: 'MD' as const, list: false },
    files: [{ index: 0, fileName: 'md.md', url: '/api/runs/r1/artifacts/b/md?index=0' }],
  },
]

describe('applyRunEvent', () => {
  it('snapshot replaces the run', () => {
    expect(applyRunEvent(null, { type: 'snapshot', run: RUN })).toBe(RUN)
    const other = { ...RUN, id: 'r2' }
    expect(applyRunEvent(RUN, { type: 'snapshot', run: other })).toBe(other)
  })

  it('node_status updates only the matching node, immutably', () => {
    const ev: RunEvent = {
      type: 'node_status',
      nodeId: 'b',
      status: 'succeeded',
      error: null,
      outputs: OUTPUTS,
    }
    const next = applyRunEvent(RUN, ev)
    expect(next).not.toBe(RUN)
    expect(next?.nodes[1]?.status).toBe('succeeded')
    expect(next?.nodes[1]?.outputs).toEqual(OUTPUTS)
    expect(next?.nodes[0]).toBe(RUN.nodes[0])
    expect(RUN.nodes[1]?.status).toBe('pending')
  })

  it('node_status carries the error message', () => {
    const next = applyRunEvent(RUN, {
      type: 'node_status',
      nodeId: 'a',
      status: 'failed',
      error: 'boom',
      outputs: [],
    })
    expect(next?.nodes[0]).toMatchObject({ status: 'failed', error: 'boom' })
  })

  it('node_status for an unknown nodeId is ignored', () => {
    const next = applyRunEvent(RUN, {
      type: 'node_status',
      nodeId: 'zzz',
      status: 'running',
      error: null,
      outputs: [],
    })
    expect(next).toBe(RUN)
  })

  it('node_status / run_finished keep null when there is no run yet', () => {
    expect(
      applyRunEvent(null, {
        type: 'node_status',
        nodeId: 'a',
        status: 'running',
        error: null,
        outputs: [],
      }),
    ).toBeNull()
    expect(applyRunEvent(null, { type: 'run_finished', status: 'failed' })).toBeNull()
  })

  it('run_finished updates only the status', () => {
    const next = applyRunEvent(RUN, { type: 'run_finished', status: 'succeeded' })
    expect(next).toEqual({ ...RUN, status: 'succeeded' })
    expect(next?.finishedAt).toBeNull()
  })
})

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
  emit(ev: RunEvent | string) {
    const data = typeof ev === 'string' ? ev : JSON.stringify(ev)
    this.onmessage?.(new MessageEvent('message', { data }))
  }
}

describe('useRunEvents', () => {
  let queryClient: QueryClient
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client: queryClient }, children)
  const lastSource = () => {
    const s = FakeEventSource.instances.at(-1)
    if (!s) throw new Error('no EventSource created')
    return s
  }

  beforeEach(() => {
    FakeEventSource.instances = []
    queryClient = new QueryClient()
    vi.stubGlobal('EventSource', FakeEventSource)
  })
  afterEach(() => vi.unstubAllGlobals())

  it('is idle and opens nothing without a runId', () => {
    const { result } = renderHook(() => useRunEvents(null), { wrapper })
    expect(result.current).toEqual({ run: null, connection: 'idle' })
    expect(FakeEventSource.instances).toHaveLength(0)
  })

  it('connects, folds snapshot + node_status, and closes on run_finished', () => {
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries')
    const { result } = renderHook(() => useRunEvents('r/1'), { wrapper })

    const source = lastSource()
    expect(source.url).toBe('/api/runs/r%2F1/events')
    expect(result.current).toEqual({ run: null, connection: 'connecting' })

    act(() => source.onopen?.())
    expect(result.current.connection).toBe('open')

    act(() => source.emit({ type: 'snapshot', run: RUN }))
    expect(result.current.run).toEqual(RUN)

    act(() =>
      source.emit({
        type: 'node_status',
        nodeId: 'a',
        status: 'running',
        error: null,
        outputs: [],
      }),
    )
    expect(result.current.run?.nodes[0]?.status).toBe('running')
    expect(source.closed).toBe(false)

    act(() => source.emit({ type: 'run_finished', status: 'succeeded' }))
    expect(source.closed).toBe(true)
    expect(result.current.connection).toBe('closed')
    expect(result.current.run?.status).toBe('succeeded')
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['runs', 'r/1'] })
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['workflows', 'w1', 'runs'] })
  })

  it('ignores unparsable messages with a warning', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const { result } = renderHook(() => useRunEvents('r1'), { wrapper })
    const source = lastSource()
    act(() => source.emit({ type: 'snapshot', run: RUN }))
    act(() => source.emit('not json'))
    expect(warn).toHaveBeenCalled()
    expect(result.current.run).toEqual(RUN)
    warn.mockRestore()
  })

  it('goes back to "connecting" on error before the run finished', () => {
    const { result } = renderHook(() => useRunEvents('r1'), { wrapper })
    const source = lastSource()
    act(() => source.onopen?.())
    act(() => source.onerror?.())
    expect(result.current.connection).toBe('connecting')
    expect(source.closed).toBe(false)
  })

  it('closes the source on unmount and when runId changes', () => {
    const { rerender, unmount } = renderHook(({ id }) => useRunEvents(id), {
      wrapper,
      initialProps: { id: 'r1' as string | null },
    })
    const first = lastSource()

    rerender({ id: 'r2' })
    expect(first.closed).toBe(true)
    const second = lastSource()
    expect(second.url).toBe('/api/runs/r2/events')
    expect(second.closed).toBe(false)

    unmount()
    expect(second.closed).toBe(true)
  })
})
