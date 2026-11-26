import type { Connection, EdgeChangeset, NodeChangeset } from '@xyflow/react'
import { create } from 'zustand'
import type { NodeKind } from '@/bindings/NodeKind'
import type { NodeStatus } from '@/bindings/NodeStatus'
import type { RunStatus } from '@/bindings/RunStatus'
import type { NodeSpec } from '@/bindings/NodeSpec'
import type { Workflow } from '@/bindings/Workflow'
import { checkConnection } from '@/lib/connection.ts'
import {
  toFlow,
  type DochestraFlowNode,
  type TypedFlowEdge,
} from '@/lib/graphMapping.ts'

interface EditorState {
  workflowId: string | null
  nodes: DochestraFlowNode[]
  edges: TypedFlowEdge[]
  catalog: NodeSpec[]
  selectedNodeId: string | null
  dirty: boolean
  /** Nodes referenced by the latest validation issues (highlighted on the canvas). */
  issueNodeIds: ReadonlySet<string>
  /** Per-node status of the run being displayed (empty when no run is shown). */
  runNodeStatus: Readonly<Record<string, NodeStatus>>
  runStatus: RunStatus | null

  load: (workflow: Workflow, catalog: NodeSpec[]) => void
  onNodesChange: (changes: NodeChangeset<DochestraFlowNode>) => void
  onEdgesChange: (changes: EdgeChangeset<TypedFlowEdge>) => void
  onConnect: (connection: Connection) => void
  addNode: (kind: NodeKind, position: { x: number; y: number }) => void
  updateNodeParams: (id: string, params: unknown) => void
  setOutput: (id: string, isOutput: boolean) => void
  removeNode: (id: string) => void
  markSaved: () => void
  setIssues: (nodeIds: ReadonlySet<string>) => void
  clearIssues: () => void
  setRunState: (nodeStatus: Record<string, NodeStatus>, runStatus: RunStatus | null) => void
  clearRun: () => void
  reset: () => void
}

const NO_ISSUES: ReadonlySet<string> = new Set<string>()

const initial = {
  workflowId: null,
  nodes: [],
  edges: [],
  catalog: [],
  selectedNodeId: null,
  dirty: false,
  issueNodeIds: new Set<string>(),
  runNodeStatus: {},
  runStatus: null,
} satisfies Partial<EditorState>

export const useEditorStore = create<EditorState>()((set, get) => ({
  ...initial,

  load: (workflow, catalog) => {
    const { nodes, edges } = toFlow(workflow.graph, catalog)
    set({ workflowId: workflow.id, nodes, edges, catalog, selectedNodeId: null, dirty: false, issueNodeIds: NO_ISSUES, runNodeStatus: {}, runStatus: null })
  },

  onNodesChange: (changeset) =>
    set((s) => {
      const changes = changeset.toArray()
      const removed = new Set(changes.flatMap((c) => (c.type === 'remove' ? [c.id] : [])))
      const dirty = changes.some(
        (c) => c.type === 'remove' || (c.type === 'position' && c.dragging === false),
      )
      let selectedNodeId = s.selectedNodeId
      for (const c of changes) {
        if (c.type !== 'select') continue
        if (c.selected) selectedNodeId = c.id
        else if (selectedNodeId === c.id) selectedNodeId = null
      }
      if (selectedNodeId !== null && removed.has(selectedNodeId)) selectedNodeId = null
      return {
        nodes: changeset.applyTo(s.nodes),
        edges:
          removed.size > 0
            ? s.edges.filter((e) => !removed.has(e.source) && !removed.has(e.target))
            : s.edges,
        selectedNodeId,
        dirty: s.dirty || dirty,
        ...(dirty ? { issueNodeIds: NO_ISSUES } : {}),
      }
    }),

  onEdgesChange: (changeset) =>
    set((s) => {
      const removed = changeset.toArray().some((c) => c.type === 'remove')
      return {
        edges: changeset.applyTo(s.edges),
        dirty: s.dirty || removed,
        ...(removed ? { issueNodeIds: NO_ISSUES } : {}),
      }
    }),

  onConnect: (connection) => {
    const s = get()
    const check = checkConnection(connection, s.nodes.map((n) => ({ id: n.id, kind: n.data.kind })), s.edges, s.catalog)
    if (!check.ok) return
    const edge: TypedFlowEdge = {
      id: `e-${crypto.randomUUID()}`,
      type: 'typed',
      source: connection.source,
      sourceHandle: connection.sourceHandle,
      target: connection.target,
      targetHandle: connection.targetHandle,
      data: { dataType: check.dataType },
    }
    set({ edges: [...s.edges, edge], dirty: true, issueNodeIds: NO_ISSUES })
  },

  addNode: (kind, position) => {
    const spec = get().catalog.find((c) => c.kind === kind)
    if (!spec) return
    const node: DochestraFlowNode = {
      id: crypto.randomUUID(),
      type: 'dochestra',
      position,
      data: { kind, params: structuredClone(spec.defaultParams), isOutput: false },
    }
    set((s) => ({ nodes: [...s.nodes, node], dirty: true, issueNodeIds: NO_ISSUES }))
  },

  updateNodeParams: (id, params) =>
    set((s) => ({
      nodes: s.nodes.map((n) => (n.id === id ? { ...n, data: { ...n.data, params } } : n)),
      dirty: true,
      issueNodeIds: NO_ISSUES,
    })),

  setOutput: (id, isOutput) =>
    set((s) => ({
      nodes: s.nodes.map((n) => (n.id === id ? { ...n, data: { ...n.data, isOutput } } : n)),
      dirty: true,
      issueNodeIds: NO_ISSUES,
    })),

  removeNode: (id) =>
    set((s) => ({
      nodes: s.nodes.filter((n) => n.id !== id),
      edges: s.edges.filter((e) => e.source !== id && e.target !== id),
      selectedNodeId: s.selectedNodeId === id ? null : s.selectedNodeId,
      dirty: true,
      issueNodeIds: NO_ISSUES,
    })),

  markSaved: () => set({ dirty: false }),

  setIssues: (issueNodeIds) => set({ issueNodeIds }),

  clearIssues: () => set({ issueNodeIds: NO_ISSUES }),

  setRunState: (runNodeStatus, runStatus) => set({ runNodeStatus, runStatus }),

  clearRun: () => set({ runNodeStatus: {}, runStatus: null }),

  reset: () => set({ ...initial }),
}))
