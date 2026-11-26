import { describe, expect, it, vi } from 'vitest'
import type { NodeStatus } from '@/bindings/NodeStatus'
import type { RunDetail } from '@/bindings/RunDetail'
import type { RunSummary } from '@/bindings/RunSummary'
import { MAX_CANDIDATE_RUNS, parseRagApi, resolveRagIndex, type ResolveDeps } from './ragIndex.ts'

const IDX = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`

const apiJson = (indexId: string, chunkCount = 7) =>
  JSON.stringify({
    type: 'rag_api',
    indexId,
    method: 'POST',
    endpoint: `/api/rag/${indexId}/query`,
    request: { question: 'string', topK: 'number' },
    chunkCount,
    embeddingModel: 'baai/bge-m3',
  })

const url = (runId: string) => `/api/runs/${runId}/artifacts/r/api?index=0`

function run(id: string, ragStatus: NodeStatus | null): RunDetail {
  return {
    id,
    workflowId: 'w',
    status: 'succeeded',
    startedAt: '',
    finishedAt: null,
    nodes:
      ragStatus === null
        ? []
        : [
            {
              nodeId: 'r',
              kind: 'rag',
              status: ragStatus,
              error: null,
              startedAt: null,
              finishedAt: null,
              outputs:
                ragStatus === 'succeeded'
                  ? [
                      {
                        socketId: 'api',
                        dataType: { scalar: 'JSON', list: false },
                        files: [{ index: 0, fileName: 'api.json', url: url(id) }],
                      },
                    ]
                  : [],
            },
          ],
  }
}

const summary = (id: string): RunSummary => ({ id, status: 'succeeded', startedAt: '', finishedAt: null })

function deps(runs: RunDetail[], files: Record<string, string> = {}): ResolveDeps & {
  getRun: ReturnType<typeof vi.fn>
} {
  const byId = new Map(runs.map((r) => [r.id, r]))
  return {
    listRuns: vi.fn(async () => runs.map((r) => summary(r.id))),
    getRun: vi.fn(async (id: string) => {
      const r = byId.get(id)
      if (!r) throw new Error(`no run ${id}`)
      return r
    }),
    fetchText: vi.fn(async (u: string) => {
      const f = files[u]
      if (f === undefined) throw new Error(`no file ${u}`)
      return f
    }),
  }
}

describe('parseRagApi', () => {
  it('accepts the rag node output', () => {
    expect(parseRagApi(apiJson(IDX(1), 12))).toEqual({
      indexId: IDX(1),
      chunkCount: 12,
      embeddingModel: 'baai/bge-m3',
    })
  })
  it('rejects invalid JSON, wrong type and non-uuid ids', () => {
    expect(() => parseRagApi('{nope')).toThrow('올바른 JSON 이 아닙니다')
    expect(() => parseRagApi('[]')).toThrow('형식')
    expect(() => parseRagApi(JSON.stringify({ type: 'other', indexId: IDX(1) }))).toThrow('rag_api')
    expect(() => parseRagApi(JSON.stringify({ type: 'rag_api', indexId: 'x' }))).toThrow('id')
  })
})

describe('resolveRagIndex', () => {
  it('prefers the selected run when the node succeeded there', async () => {
    const d = deps([run('new', 'succeeded'), run('sel', 'succeeded')], {
      [url('new')]: apiJson(IDX(1)),
      [url('sel')]: apiJson(IDX(2), 3),
    })
    expect(await resolveRagIndex('w', 'r', 'sel', d)).toEqual({
      indexId: IDX(2),
      chunkCount: 3,
      embeddingModel: 'baai/bge-m3',
      runId: 'sel',
    })
    expect(d.listRuns).not.toHaveBeenCalled()
  })

  it('falls back to the most recent run where the node succeeded', async () => {
    const d = deps(
      [run('sel', 'failed'), run('a', null), run('b', 'skipped'), run('c', 'succeeded'), run('d', 'succeeded')],
      { [url('c')]: apiJson(IDX(3)), [url('d')]: apiJson(IDX(4)) },
    )
    const hit = await resolveRagIndex('w', 'r', 'sel', d)
    expect(hit?.runId).toBe('c')
    expect(hit?.indexId).toBe(IDX(3))
    // lazy: stops at the first hit, never refetches the selected run, never touches 'd'
    expect(d.getRun.mock.calls.map((c) => c[0])).toEqual(['sel', 'a', 'b', 'c'])
  })

  it('uses the latest run when nothing is selected', async () => {
    const d = deps([run('x', 'succeeded')], { [url('x')]: apiJson(IDX(5)) })
    expect((await resolveRagIndex('w', 'r', null, d))?.runId).toBe('x')
  })

  it('returns null when no run built the index (and checks at most 10 runs)', async () => {
    const runs = Array.from({ length: 12 }, (_, i) => run(`r${i}`, i === 11 ? 'succeeded' : 'failed'))
    const d = deps(runs, { [url('r11')]: apiJson(IDX(6)) })
    expect(await resolveRagIndex('w', 'r', null, d)).toBeNull()
    expect(d.getRun).toHaveBeenCalledTimes(MAX_CANDIDATE_RUNS)
    expect(await resolveRagIndex('w', 'r', null, deps([]))).toBeNull()
  })

  it('reports an invalid api artifact as an error', async () => {
    const d = deps([run('x', 'succeeded')], { [url('x')]: 'not json' })
    await expect(resolveRagIndex('w', 'r', null, d)).rejects.toThrow('올바른 JSON 이 아닙니다')
  })
})
