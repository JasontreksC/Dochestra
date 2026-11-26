import { EdgeChangeset, NodeChangeset, type NodeChange } from '@xyflow/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Workflow } from '@/bindings/Workflow'
import type { DochestraFlowNode, TypedFlowEdge } from '@/lib/graphMapping.ts'
import { CATALOG } from '@/test/utils.tsx'
import { useEditorStore } from './editorStore.ts'

const WORKFLOW: Workflow = {
  id: 'w1',
  name: 'demo',
  createdAt: '',
  updatedAt: '',
  graph: {
    nodes: [
      { id: 'a', kind: 'bundle', position: { x: 0, y: 0 }, params: { bundleId: null }, isOutput: false },
      { id: 'b', kind: 'pick_pdf', position: { x: 200, y: 0 }, params: { index: 0 }, isOutput: false },
    ],
    edges: [{ id: 'e1', source: 'a', sourceHandle: 'files', target: 'b', targetHandle: 'files' }],
  },
}

const store = () => useEditorStore.getState()
const nodeChanges = (...changes: NodeChange<DochestraFlowNode>[]) =>
  new NodeChangeset<DochestraFlowNode>(changes)

beforeEach(() => {
  store().reset()
  store().load(WORKFLOW, CATALOG)
})

describe('editorStore', () => {
  it('load maps the graph and starts clean', () => {
    expect(store().workflowId).toBe('w1')
    expect(store().nodes).toHaveLength(2)
    expect(store().edges).toHaveLength(1)
    expect(store().dirty).toBe(false)
  })

  it('addNode clones default params and marks dirty', () => {
    store().addNode('bundle', { x: 5, y: 6 })
    const added = store().nodes.at(-1)!
    expect(added.position).toEqual({ x: 5, y: 6 })
    expect(added.data).toMatchObject({ kind: 'bundle', isOutput: false, params: { bundleId: null } })
    expect(store().dirty).toBe(true)

    ;(added.data.params as { bundleId: string | null }).bundleId = 'mutated'
    expect(CATALOG[0]?.defaultParams).toEqual({ bundleId: null })
  })

  it('addNode makes the new node the only selected node', () => {
    store().onNodesChange(nodeChanges({ type: 'select', id: 'a', selected: true }))
    expect(store().nodes.find((n) => n.id === 'a')?.selected).toBe(true)
    const id = store().addNode('extract', { x: 0, y: 0 })
    expect(id).not.toBeNull()
    expect(store().selectedNodeId).toBe(id)
    expect(store().nodes.filter((n) => n.selected).map((n) => n.id)).toEqual([id])
  })

  it('addNode ignores kinds missing from the catalog', () => {
    expect(store().addNode('voice', { x: 0, y: 0 })).toBeNull()
    expect(store().nodes).toHaveLength(2)
    expect(store().dirty).toBe(false)
  })

  it('onConnect adds a typed edge for a valid connection', () => {
    store().addNode('pick_pdf', { x: 0, y: 100 })
    const target = store().nodes.at(-1)!
    store().markSaved()
    store().onConnect({ source: 'a', sourceHandle: 'files', target: target.id, targetHandle: 'files' })
    expect(store().edges).toHaveLength(2)
    const edge = store().edges.at(-1)!
    expect(edge.id).toMatch(/^e-/)
    expect(edge).toMatchObject({ type: 'typed', data: { dataType: { scalar: 'PDF', list: true } } })
    expect(store().dirty).toBe(true)
  })

  it('onConnect ignores invalid connections', () => {
    // input already occupied
    store().onConnect({ source: 'a', sourceHandle: 'files', target: 'b', targetHandle: 'files' })
    // type mismatch
    store().addNode('extract', { x: 0, y: 0 })
    const extract = store().nodes.at(-1)!
    store().markSaved()
    store().onConnect({ source: 'a', sourceHandle: 'files', target: extract.id, targetHandle: 'pdf' })
    expect(store().edges).toHaveLength(1)
    expect(store().dirty).toBe(false)
  })

  it('dirty only changes for real edits, not selection or drag-in-progress', () => {
    store().onNodesChange(nodeChanges({ type: 'select', id: 'a', selected: true }))
    expect(store().dirty).toBe(false)
    expect(store().selectedNodeId).toBe('a')

    store().onNodesChange(nodeChanges({ type: 'position', id: 'a', position: { x: 5, y: 5 }, dragging: true }))
    expect(store().dirty).toBe(false)

    store().onNodesChange(nodeChanges({ type: 'position', id: 'a', position: { x: 9, y: 9 }, dragging: false }))
    expect(store().dirty).toBe(true)
    expect(store().nodes[0]?.position).toEqual({ x: 9, y: 9 })

    store().markSaved()
    expect(store().dirty).toBe(false)
  })

  it('deselecting clears selectedNodeId', () => {
    store().onNodesChange(nodeChanges({ type: 'select', id: 'a', selected: true }))
    store().onNodesChange(nodeChanges({ type: 'select', id: 'a', selected: false }))
    expect(store().selectedNodeId).toBeNull()
  })

  it('removing a node also removes its edges', () => {
    store().onNodesChange(nodeChanges({ type: 'remove', id: 'a' }))
    expect(store().nodes.map((n) => n.id)).toEqual(['b'])
    expect(store().edges).toHaveLength(0)
    expect(store().dirty).toBe(true)
  })

  it('removing an edge marks dirty', () => {
    store().onEdgesChange(new EdgeChangeset<TypedFlowEdge>([{ type: 'remove', id: 'e1' }]))
    expect(store().edges).toHaveLength(0)
    expect(store().dirty).toBe(true)
  })

  it('updateNodeParams and setOutput update the node and mark dirty', () => {
    store().updateNodeParams('b', { index: 3 })
    store().setOutput('b', true)
    expect(store().nodes[1]?.data).toMatchObject({ params: { index: 3 }, isOutput: true })
    expect(store().dirty).toBe(true)
  })
})

describe('editorStore issues and run state', () => {
  it('stores issue node ids and clears them on graph edits', () => {
    store().setIssues(new Set(['a']))
    expect(store().issueNodeIds.has('a')).toBe(true)
    store().updateNodeParams('b', { index: 2 })
    expect(store().issueNodeIds.size).toBe(0)

    store().setIssues(new Set(['a']))
    store().onNodesChange(nodeChanges({ type: 'select', id: 'a', selected: true }))
    expect(store().issueNodeIds.size).toBe(1) // selection is not an edit

    store().onNodesChange(nodeChanges({ type: 'remove', id: 'b' }))
    expect(store().issueNodeIds.size).toBe(0)

    store().setIssues(new Set(['a']))
    store().clearIssues()
    expect(store().issueNodeIds.size).toBe(0)
  })

  it('tracks and clears run state; load and reset also clear it', () => {
    store().setRunState({ a: 'running' }, 'running')
    expect(store().runNodeStatus).toEqual({ a: 'running' })
    expect(store().runStatus).toBe('running')
    store().clearRun()
    expect(store().runNodeStatus).toEqual({})
    expect(store().runStatus).toBeNull()

    store().setRunState({ a: 'failed' }, 'failed')
    store().setIssues(new Set(['a']))
    store().load(WORKFLOW, CATALOG)
    expect(store().runNodeStatus).toEqual({})
    expect(store().issueNodeIds.size).toBe(0)
  })
})

describe('editorStore without crypto.randomUUID (insecure http context)', () => {
  const UUID_V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/
  beforeEach(() => {
    const real = globalThis.crypto
    vi.stubGlobal('crypto', {
      getRandomValues: real.getRandomValues.bind(real),
    })
  })
  afterEach(() => vi.unstubAllGlobals())

  it('addNode and onConnect still work', () => {
    expect(typeof crypto.randomUUID).toBe('undefined')
    const id = store().addNode('pick_pdf', { x: 0, y: 100 })
    expect(id).toMatch(UUID_V4)
    store().onConnect({ source: 'a', sourceHandle: 'files', target: id!, targetHandle: 'files' })
    expect(store().edges).toHaveLength(2)
    expect(store().edges.at(-1)!.id).toMatch(/^e-[0-9a-f-]{36}$/)
  })
})
