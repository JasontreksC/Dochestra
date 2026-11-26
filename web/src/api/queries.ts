import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import * as api from './endpoints.ts'

export const qk = {
  nodes: ['nodes'],
  workflows: ['workflows'],
  workflow: (id: string) => ['workflows', id],
  runs: (workflowId: string) => ['workflows', workflowId, 'runs'],
  run: (runId: string) => ['runs', runId],
  bundles: ['bundles'],
  bundle: (id: string) => ['bundles', id],
} as const

// ---- queries ----

export const useNodeCatalog = () =>
  useQuery({ queryKey: qk.nodes, queryFn: api.getNodes, staleTime: Infinity })

export const useWorkflows = () => useQuery({ queryKey: qk.workflows, queryFn: api.listWorkflows })

export const useWorkflow = (id?: string) =>
  useQuery({
    queryKey: qk.workflow(id ?? ''),
    queryFn: () => api.getWorkflow(id as string),
    enabled: !!id,
  })

export const useRuns = (workflowId?: string) =>
  useQuery({
    queryKey: qk.runs(workflowId ?? ''),
    queryFn: () => api.listRuns(workflowId as string),
    enabled: !!workflowId,
  })

export const useRun = (runId?: string) =>
  useQuery({
    queryKey: qk.run(runId ?? ''),
    queryFn: () => api.getRun(runId as string),
    enabled: !!runId,
  })

export const useBundles = () => useQuery({ queryKey: qk.bundles, queryFn: api.listBundles })

export const useBundle = (id?: string) =>
  useQuery({
    queryKey: qk.bundle(id ?? ''),
    queryFn: () => api.getBundle(id as string),
    enabled: !!id,
  })

// ---- mutations ----

export function useCreateWorkflow() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: api.createWorkflow,
    onSuccess: () => qc.invalidateQueries({ queryKey: qk.workflows }),
  })
}

export function useUpdateWorkflow() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, ...req }: { id: string } & Parameters<typeof api.updateWorkflow>[1]) =>
      api.updateWorkflow(id, req),
    onSuccess: (workflow) => {
      qc.setQueryData(qk.workflow(workflow.id), workflow)
      return qc.invalidateQueries({ queryKey: qk.workflows, exact: true })
    },
  })
}

export function useDeleteWorkflow() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: api.deleteWorkflow,
    onSuccess: (_void, id) => {
      qc.removeQueries({ queryKey: qk.workflow(id) })
      return qc.invalidateQueries({ queryKey: qk.workflows, exact: true })
    },
  })
}

export const useValidateWorkflow = () => useMutation({ mutationFn: api.validateWorkflow })

export function useStartRun() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: api.startRun,
    onSuccess: (_created, workflowId) =>
      qc.invalidateQueries({ queryKey: qk.runs(workflowId) }),
  })
}

export function useCreateBundle() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: api.createBundle,
    onSuccess: () => qc.invalidateQueries({ queryKey: qk.bundles }),
  })
}

export function useUploadBundleFiles() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ bundleId, files }: { bundleId: string; files: File[] }) =>
      api.uploadBundleFiles(bundleId, files),
    onSuccess: (_files, { bundleId }) =>
      Promise.all([
        qc.invalidateQueries({ queryKey: qk.bundle(bundleId) }),
        qc.invalidateQueries({ queryKey: qk.bundles }),
      ]),
  })
}

export function useDeleteBundle() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: api.deleteBundle,
    onSuccess: (_void, id) => {
      qc.removeQueries({ queryKey: qk.bundle(id) })
      return qc.invalidateQueries({ queryKey: qk.bundles })
    },
  })
}

export function useDeleteBundleFile() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ bundleId, fileId }: { bundleId: string; fileId: string }) =>
      api.deleteBundleFile(bundleId, fileId),
    onSuccess: (_void, { bundleId }) =>
      Promise.all([
        qc.invalidateQueries({ queryKey: qk.bundle(bundleId) }),
        qc.invalidateQueries({ queryKey: qk.bundles }),
      ]),
  })
}
