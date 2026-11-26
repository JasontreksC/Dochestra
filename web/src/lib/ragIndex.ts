import { useQuery } from '@tanstack/react-query'
import type { RunDetail } from '@/bindings/RunDetail'
import type { RunSummary } from '@/bindings/RunSummary'
import * as api from '@/api/endpoints.ts'
import { useEditorStore } from '@/store/editorStore.ts'

/** How many of the most recent runs are inspected when looking for a built index. */
export const MAX_CANDIDATE_RUNS = 10

export interface RagIndexInfo {
  indexId: string
  chunkCount: number
  embeddingModel: string
  runId: string
}

export type RagIndexState =
  | { status: 'loading' }
  | { status: 'none' }
  | { status: 'error'; message: string }
  | ({ status: 'ready' } & RagIndexInfo)

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

/** Parses the rag node's `api` artifact (CONTEXT §4.3); throws with a readable message. */
export function parseRagApi(text: string): Omit<RagIndexInfo, 'runId'> {
  let value: unknown
  try {
    value = JSON.parse(text)
  } catch {
    throw new Error('RAG API 정보가 올바른 JSON 이 아닙니다')
  }
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error('RAG API 정보 형식이 올바르지 않습니다')
  }
  const rec = value as Record<string, unknown>
  if (rec.type !== 'rag_api') throw new Error('RAG API 정보가 아닙니다 (type ≠ rag_api)')
  if (typeof rec.indexId !== 'string' || !UUID_RE.test(rec.indexId)) {
    throw new Error('RAG 인덱스 id 가 올바르지 않습니다')
  }
  return {
    indexId: rec.indexId,
    chunkCount: typeof rec.chunkCount === 'number' ? rec.chunkCount : 0,
    embeddingModel: typeof rec.embeddingModel === 'string' ? rec.embeddingModel : '',
  }
}

/** URL of the node's `api` artifact when the node succeeded in this run, else null. */
export function ragApiUrl(run: RunDetail, nodeId: string): string | null {
  const node = run.nodes.find((n) => n.nodeId === nodeId)
  if (!node || node.status !== 'succeeded') return null
  return node.outputs.find((o) => o.socketId === 'api')?.files[0]?.url ?? null
}

export interface ResolveDeps {
  listRuns: (workflowId: string) => Promise<RunSummary[]>
  getRun: (runId: string) => Promise<RunDetail>
  fetchText: (url: string) => Promise<string>
}

export const defaultDeps: ResolveDeps = {
  listRuns: api.listRuns,
  getRun: api.getRun,
  fetchText: async (url) => {
    const res = await fetch(url)
    if (!res.ok) throw new Error(`RAG API 정보를 불러오지 못했습니다 (HTTP ${res.status})`)
    return res.text()
  },
}

/**
 * Picks the run whose index the chat talks to: the run selected in the run panel when this node
 * succeeded there, else the most recent run (of at most MAX_CANDIDATE_RUNS, newest first) in
 * which it succeeded. Run details are fetched one at a time and only as far as needed.
 * Returns null when no run built an index.
 */
export async function resolveRagIndex(
  workflowId: string,
  nodeId: string,
  selectedRunId: string | null,
  deps: ResolveDeps = defaultDeps,
): Promise<RagIndexInfo | null> {
  const found = async (run: RunDetail): Promise<RagIndexInfo | null> => {
    const url = ragApiUrl(run, nodeId)
    if (!url) return null
    return { ...parseRagApi(await deps.fetchText(url)), runId: run.id }
  }

  if (selectedRunId) {
    const hit = await found(await deps.getRun(selectedRunId))
    if (hit) return hit
  }
  const runs = (await deps.listRuns(workflowId)).slice(0, MAX_CANDIDATE_RUNS)
  for (const summary of runs) {
    if (summary.id === selectedRunId) continue
    const hit = await found(await deps.getRun(summary.id))
    if (hit) return hit
  }
  return null
}

/**
 * Index of the rag node `nodeId`. Re-resolves when the selected run changes or when this node
 * reaches a new status in the displayed run (e.g. a live run finishing).
 */
export function useRagIndex(
  workflowId: string | null,
  nodeId: string,
  selectedRunId: string | null,
): RagIndexState {
  const nodeStatus = useEditorStore((s) => s.runNodeStatus[nodeId] ?? null)
  const query = useQuery({
    queryKey: ['rag-index', workflowId, nodeId, selectedRunId, nodeStatus],
    queryFn: () => resolveRagIndex(workflowId as string, nodeId, selectedRunId),
    enabled: !!workflowId,
    refetchOnWindowFocus: false,
    placeholderData: (prev) => prev,
  })
  if (query.isPending) return { status: 'loading' }
  if (query.isError) return { status: 'error', message: query.error.message }
  if (!query.data) return { status: 'none' }
  return { status: 'ready', ...query.data }
}
