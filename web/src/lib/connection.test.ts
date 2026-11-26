import { describe, expect, it } from 'vitest'
import type { NodeKind } from '@/bindings/NodeKind'
import { CATALOG } from '@/test/utils.tsx'
import { checkConnection, type ConnectionCandidate } from './connection.ts'

const nodes: Array<{ id: string; kind: NodeKind }> = [
  { id: 'bundle', kind: 'bundle' },
  { id: 'pick', kind: 'pick_pdf' },
  { id: 'pick2', kind: 'pick_pdf' },
  { id: 'extract', kind: 'extract' },
  { id: 't1', kind: 'translate' },
  { id: 't2', kind: 'translate' },
]

const check = (
  c: Partial<ConnectionCandidate>,
  edges: Array<{ source: string; target: string; targetHandle?: string | null }> = [],
) =>
  checkConnection(
    { source: 'bundle', sourceHandle: 'files', target: 'pick', targetHandle: 'files', ...c },
    nodes,
    edges,
    CATALOG,
  )

describe('checkConnection', () => {
  it('accepts matching types and reports the data type', () => {
    expect(check({})).toEqual({ ok: true, dataType: { scalar: 'PDF', list: true } })
  })

  it('allows fan-out from one output', () => {
    const edges = [{ source: 'bundle', target: 'pick', targetHandle: 'files' }]
    expect(check({ target: 'pick2' }, edges).ok).toBe(true)
  })

  it('rejects missing handles', () => {
    expect(check({ sourceHandle: null })).toEqual({ ok: false, reason: 'missing-handle' })
    expect(check({ targetHandle: null })).toEqual({ ok: false, reason: 'missing-handle' })
  })

  it('rejects unknown nodes', () => {
    expect(check({ source: 'nope' })).toEqual({ ok: false, reason: 'unknown-node' })
    expect(check({ target: 'nope' })).toEqual({ ok: false, reason: 'unknown-node' })
  })

  it('rejects unknown sockets', () => {
    expect(check({ sourceHandle: 'zzz' })).toEqual({ ok: false, reason: 'unknown-socket' })
    expect(check({ targetHandle: 'zzz' })).toEqual({ ok: false, reason: 'unknown-socket' })
    // an input id used as a source handle
    expect(check({ source: 'pick', sourceHandle: 'files', target: 'extract', targetHandle: 'pdf' })).toEqual({
      ok: false,
      reason: 'unknown-socket',
    })
  })

  it('rejects self loops', () => {
    expect(
      check({ source: 't1', sourceHandle: 'md', target: 't1', targetHandle: 'md' }),
    ).toEqual({ ok: false, reason: 'self-loop' })
  })

  it('rejects type mismatches, including LIST[T] vs T (D-5)', () => {
    expect(check({ target: 'extract', targetHandle: 'pdf' })).toEqual({
      ok: false,
      reason: 'type-mismatch',
    })
  })

  it('rejects a second edge into the same input socket', () => {
    const edges = [{ source: 'bundle', target: 'pick', targetHandle: 'files' }]
    expect(check({}, edges)).toEqual({ ok: false, reason: 'input-occupied' })
  })

  it('rejects connections that would create a cycle', () => {
    const edges = [{ source: 't1', target: 't2', targetHandle: 'md' }]
    expect(
      check({ source: 't2', sourceHandle: 'md', target: 't1', targetHandle: 'md' }, edges),
    ).toEqual({ ok: false, reason: 'cycle' })
  })
})
