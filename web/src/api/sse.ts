import { useEffect, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import type { RunDetail } from '@/bindings/RunDetail'
import type { RunEvent } from '@/bindings/RunEvent'
import { qk } from './queries.ts'

export type RunConnection = 'idle' | 'connecting' | 'open' | 'closed'

/** Pure reducer: folds one SSE `RunEvent` into the current run state. */
export function applyRunEvent(run: RunDetail | null, ev: RunEvent): RunDetail | null {
  switch (ev.type) {
    case 'snapshot':
      return ev.run
    case 'node_status': {
      if (!run) return null
      if (!run.nodes.some((n) => n.nodeId === ev.nodeId)) return run
      return {
        ...run,
        nodes: run.nodes.map((n) =>
          n.nodeId === ev.nodeId
            ? { ...n, status: ev.status, error: ev.error, outputs: ev.outputs }
            : n,
        ),
      }
    }
    case 'run_finished':
      return run ? { ...run, status: ev.status } : null
  }
}

interface RunEventsState {
  run: RunDetail | null
  connection: RunConnection
}

interface Tracked extends RunEventsState {
  runId: string
}

/**
 * Subscribes to `/api/runs/{runId}/events` (SSE, unnamed events). The server sends a snapshot
 * first, including after the browser's automatic reconnects.
 */
export function useRunEvents(runId: string | null): RunEventsState {
  const queryClient = useQueryClient()
  const [tracked, setTracked] = useState<Tracked | null>(null)

  useEffect(() => {
    if (runId === null) return

    let latest: RunDetail | null = null
    let finished = false
    const publish = (connection: RunConnection) => setTracked({ runId, run: latest, connection })

    const source = new EventSource(`/api/runs/${encodeURIComponent(runId)}/events`)

    source.onopen = () => {
      if (!finished) publish('open')
    }
    source.onmessage = (msg: MessageEvent<string>) => {
      let ev: RunEvent
      try {
        ev = JSON.parse(msg.data) as RunEvent
      } catch (e) {
        console.warn('useRunEvents: ignoring unparsable SSE message', e)
        return
      }
      latest = applyRunEvent(latest, ev)
      if (ev.type === 'run_finished') {
        finished = true
        source.close()
        publish('closed')
        void queryClient.invalidateQueries({ queryKey: qk.run(runId) })
        if (latest) void queryClient.invalidateQueries({ queryKey: qk.runs(latest.workflowId) })
      } else {
        publish('open')
      }
    }
    source.onerror = () => {
      // The browser reconnects on its own; the server then re-sends a snapshot.
      if (!finished) publish('connecting')
    }

    return () => {
      finished = true
      source.close()
    }
  }, [runId, queryClient])

  if (runId === null) return { run: null, connection: 'idle' }
  if (tracked?.runId !== runId) return { run: null, connection: 'connecting' }
  return { run: tracked.run, connection: tracked.connection }
}
