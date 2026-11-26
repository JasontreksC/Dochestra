import { afterEach, describe, expect, it, vi } from 'vitest'
import type { WorkflowGraph } from '@/bindings/WorkflowGraph'
import { CATALOG } from '@/test/utils.tsx'
import { toFlow, toGraph } from './graphMapping.ts'

const GRAPH: WorkflowGraph = {
  nodes: [
    { id: 'a', kind: 'bundle', position: { x: 10.5, y: 20.25 }, params: { bundleId: 'b1' }, isOutput: false },
    { id: 'b', kind: 'pick_pdf', position: { x: 300, y: -40 }, params: { index: 2 }, isOutput: false },
    { id: 'c', kind: 'extract', position: { x: 600, y: 0 }, params: {}, isOutput: true },
  ],
  edges: [
    { id: 'e1', source: 'a', sourceHandle: 'files', target: 'b', targetHandle: 'files' },
    { id: 'e2', source: 'b', sourceHandle: 'pdf', target: 'c', targetHandle: 'pdf' },
  ],
}

afterEach(() => vi.restoreAllMocks())

describe('graphMapping', () => {
  it('round-trips a graph unchanged', () => {
    const { nodes, edges } = toFlow(GRAPH, CATALOG)
    expect(toGraph(nodes, edges)).toEqual(GRAPH)
  })

  it('sets node/edge types and the edge data type from the source socket', () => {
    const { nodes, edges } = toFlow(GRAPH, CATALOG)
    expect(nodes.every((n) => n.type === 'dochestra')).toBe(true)
    expect(edges[0]).toMatchObject({ type: 'typed', data: { dataType: { scalar: 'PDF', list: true } } })
    expect(edges[1]?.data?.dataType).toEqual({ scalar: 'PDF', list: false })
  })

  it('drops edges whose source socket is not in the catalog', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const graph: WorkflowGraph = {
      ...GRAPH,
      edges: [...GRAPH.edges, { id: 'bad', source: 'a', sourceHandle: 'nope', target: 'b', targetHandle: 'files' }],
    }
    const { edges } = toFlow(graph, CATALOG)
    expect(edges.map((e) => e.id)).toEqual(['e1', 'e2'])
    expect(warn).toHaveBeenCalled()
  })

  it('toGraph skips edges with a null handle', () => {
    const { nodes, edges } = toFlow(GRAPH, CATALOG)
    const broken = [{ ...edges[0]!, targetHandle: null }, edges[1]!]
    expect(toGraph(nodes, broken).edges.map((e) => e.id)).toEqual(['e2'])
  })
})
