import type { ApiErrorBody } from '@/bindings/ApiErrorBody'
import type { ValidationIssue } from '@/bindings/ValidationIssue'

const API_BASE = '/api'

export type HttpMethod = 'GET' | 'POST' | 'PUT' | 'DELETE'

/** Error raised for every non-2xx response and for network failures (status 0). */
export class ApiError extends Error {
  readonly status: number
  readonly code: string
  readonly issues: ValidationIssue[]

  constructor(status: number, code: string, message: string, issues: ValidationIssue[] = []) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
    this.issues = issues
  }
}

export function isValidationError(e: unknown): e is ApiError {
  return e instanceof ApiError && e.code === 'VALIDATION_FAILED'
}

function isApiErrorBody(value: unknown): value is ApiErrorBody {
  if (typeof value !== 'object' || value === null || !('error' in value)) return false
  const err = value.error
  return (
    typeof err === 'object' &&
    err !== null &&
    'code' in err &&
    typeof err.code === 'string' &&
    'message' in err &&
    typeof err.message === 'string'
  )
}

async function toApiError(res: Response): Promise<ApiError> {
  let text = ''
  try {
    text = await res.text()
  } catch {
    // body unreadable: fall through to the generic error
  }
  try {
    const parsed: unknown = JSON.parse(text)
    if (isApiErrorBody(parsed)) {
      const { code, message, issues } = parsed.error
      return new ApiError(res.status, code, message, issues ?? [])
    }
  } catch {
    // not JSON
  }
  return new ApiError(res.status, `HTTP_${res.status}`, (text || res.statusText).slice(0, 200))
}

/** Shared fetch + response/error handling for JSON and multipart requests. */
async function send<T>(method: HttpMethod, path: string, init: RequestInit): Promise<T> {
  let res: Response
  try {
    res = await fetch(`${API_BASE}${path}`, { ...init, method })
  } catch (e) {
    throw new ApiError(0, 'NETWORK', e instanceof Error ? e.message : 'network error')
  }
  if (!res.ok) throw await toApiError(res)
  if (res.status === 204) return undefined as T
  return (await res.json()) as T
}

export function request<T>(method: HttpMethod, path: string, body?: unknown): Promise<T> {
  if (body === undefined) return send<T>(method, path, {})
  return send<T>(method, path, {
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
}

/** multipart/form-data request; the browser sets Content-Type (with boundary) itself. */
export function requestForm<T>(method: HttpMethod, path: string, form: FormData): Promise<T> {
  return send<T>(method, path, { body: form })
}
