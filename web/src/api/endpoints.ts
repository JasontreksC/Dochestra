import type { Bundle } from '@/bindings/Bundle'
import type { BundleFile } from '@/bindings/BundleFile'
import type { BundleSummary } from '@/bindings/BundleSummary'
import type { CreateBundleRequest } from '@/bindings/CreateBundleRequest'
import type { CreateWorkflowRequest } from '@/bindings/CreateWorkflowRequest'
import type { NodeSpec } from '@/bindings/NodeSpec'
import type { RagQueryRequest } from '@/bindings/RagQueryRequest'
import type { RagQueryResponse } from '@/bindings/RagQueryResponse'
import type { RunCreated } from '@/bindings/RunCreated'
import type { RunDetail } from '@/bindings/RunDetail'
import type { RunSummary } from '@/bindings/RunSummary'
import type { UpdateWorkflowRequest } from '@/bindings/UpdateWorkflowRequest'
import type { ValidationReport } from '@/bindings/ValidationReport'
import type { Workflow } from '@/bindings/Workflow'
import type { WorkflowSummary } from '@/bindings/WorkflowSummary'
import { request, requestForm } from './client.ts'

const enc = encodeURIComponent

// catalog
export const getNodes = (): Promise<NodeSpec[]> => request('GET', '/nodes')

// workflows
export const listWorkflows = (): Promise<WorkflowSummary[]> => request('GET', '/workflows')
export const createWorkflow = (req: CreateWorkflowRequest): Promise<Workflow> =>
  request('POST', '/workflows', req)
export const getWorkflow = (id: string): Promise<Workflow> =>
  request('GET', `/workflows/${enc(id)}`)
export const updateWorkflow = (id: string, req: UpdateWorkflowRequest): Promise<Workflow> =>
  request('PUT', `/workflows/${enc(id)}`, req)
export const deleteWorkflow = (id: string): Promise<void> =>
  request('DELETE', `/workflows/${enc(id)}`)
export const validateWorkflow = (id: string): Promise<ValidationReport> =>
  request('POST', `/workflows/${enc(id)}/validate`)

// runs
export const startRun = (workflowId: string): Promise<RunCreated> =>
  request('POST', `/workflows/${enc(workflowId)}/runs`)
export const listRuns = (workflowId: string): Promise<RunSummary[]> =>
  request('GET', `/workflows/${enc(workflowId)}/runs`)
export const getRun = (runId: string): Promise<RunDetail> => request('GET', `/runs/${enc(runId)}`)

// bundles
export const listBundles = (): Promise<BundleSummary[]> => request('GET', '/bundles')
export const createBundle = (req: CreateBundleRequest): Promise<Bundle> =>
  request('POST', '/bundles', req)
export const getBundle = (id: string): Promise<Bundle> => request('GET', `/bundles/${enc(id)}`)
export const deleteBundle = (id: string): Promise<void> => request('DELETE', `/bundles/${enc(id)}`)
export const uploadBundleFiles = (bundleId: string, files: File[]): Promise<BundleFile[]> => {
  const form = new FormData()
  for (const file of files) form.append('files', file)
  return requestForm('POST', `/bundles/${enc(bundleId)}/files`, form)
}
export const deleteBundleFile = (bundleId: string, fileId: string): Promise<void> =>
  request('DELETE', `/bundles/${enc(bundleId)}/files/${enc(fileId)}`)

// rag
export const queryRag = (indexId: string, req: RagQueryRequest): Promise<RagQueryResponse> =>
  request('POST', `/rag/${enc(indexId)}/query`, req)
