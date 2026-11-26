import { afterEach, describe, expect, it, vi } from 'vitest'
import { ApiError, isValidationError, request } from './client.ts'
import { uploadBundleFiles } from './endpoints.ts'

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

function stubFetch(impl: (...args: Parameters<typeof fetch>) => Promise<Response>) {
  const fn = vi.fn(impl)
  vi.stubGlobal('fetch', fn)
  return fn
}

afterEach(() => vi.unstubAllGlobals())

describe('request', () => {
  it('returns parsed JSON on success and uses the /api base', async () => {
    const fetchMock = stubFetch(async () => jsonResponse({ status: 'ok' }))
    await expect(request('GET', '/health')).resolves.toEqual({ status: 'ok' })
    expect(fetchMock).toHaveBeenCalledWith('/api/health', { method: 'GET' })
  })

  it('serializes a body as JSON with a Content-Type header', async () => {
    const fetchMock = stubFetch(async () => jsonResponse({ id: '1' }, 201))
    await request('POST', '/workflows', { name: 'x' })
    const [, init] = fetchMock.mock.calls[0] ?? []
    expect(init?.method).toBe('POST')
    expect(init?.body).toBe('{"name":"x"}')
    expect(init?.headers).toEqual({ 'Content-Type': 'application/json' })
  })

  it('returns undefined for 204', async () => {
    stubFetch(async () => new Response(null, { status: 204 }))
    await expect(request('DELETE', '/workflows/1')).resolves.toBeUndefined()
  })

  it('maps a standard error body (with issues) to ApiError', async () => {
    const issues = [{ code: 'NO_OUTPUT_NODE' }, { code: 'DUPLICATE_ID', id: 'n1' }]
    stubFetch(async () =>
      jsonResponse(
        { error: { code: 'VALIDATION_FAILED', message: 'invalid graph', issues } },
        422,
      ),
    )
    const err = await request('PUT', '/workflows/1', {}).catch((e: unknown) => e)
    expect(err).toBeInstanceOf(ApiError)
    expect(isValidationError(err)).toBe(true)
    const apiErr = err as ApiError
    expect(apiErr.status).toBe(422)
    expect(apiErr.code).toBe('VALIDATION_FAILED')
    expect(apiErr.message).toBe('invalid graph')
    expect(apiErr.issues).toEqual(issues)
  })

  it('defaults issues to [] when the error body has none', async () => {
    stubFetch(async () => jsonResponse({ error: { code: 'NOT_FOUND', message: 'nope' } }, 404))
    const err = (await request('GET', '/workflows/x').catch((e: unknown) => e)) as ApiError
    expect(err.code).toBe('NOT_FOUND')
    expect(err.issues).toEqual([])
    expect(isValidationError(err)).toBe(false)
  })

  it('maps a non-JSON error response to HTTP_<status> with truncated text', async () => {
    stubFetch(async () => new Response('x'.repeat(500), { status: 500 }))
    const err = (await request('GET', '/boom').catch((e: unknown) => e)) as ApiError
    expect(err).toBeInstanceOf(ApiError)
    expect(err.status).toBe(500)
    expect(err.code).toBe('HTTP_500')
    expect(err.message).toHaveLength(200)
  })

  it('maps fetch rejection to status 0 / NETWORK', async () => {
    stubFetch(async () => {
      throw new TypeError('Failed to fetch')
    })
    const err = (await request('GET', '/x').catch((e: unknown) => e)) as ApiError
    expect(err).toBeInstanceOf(ApiError)
    expect(err.status).toBe(0)
    expect(err.code).toBe('NETWORK')
  })

  it('isValidationError rejects non-ApiError values', () => {
    expect(isValidationError(new Error('VALIDATION_FAILED'))).toBe(false)
    expect(isValidationError(null)).toBe(false)
  })
})

describe('uploadBundleFiles', () => {
  it('sends a FormData with repeated "files" fields and no manual Content-Type', async () => {
    const fetchMock = stubFetch(async () => jsonResponse([], 201))
    const a = new File(['%PDF-1'], 'a.pdf', { type: 'application/pdf' })
    const b = new File(['%PDF-2'], 'b.pdf', { type: 'application/pdf' })
    await uploadBundleFiles('bundle/1', [a, b])

    const [url, init] = fetchMock.mock.calls[0] ?? []
    expect(url).toBe('/api/bundles/bundle%2F1/files')
    expect(init?.method).toBe('POST')
    const form = init?.body
    expect(form).toBeInstanceOf(FormData)
    const files = (form as FormData).getAll('files') as File[]
    expect(files.map((f) => f.name)).toEqual(['a.pdf', 'b.pdf'])
    expect(init?.headers).toBeUndefined()
  })

  it('shares ApiError handling with request', async () => {
    stubFetch(async () =>
      jsonResponse({ error: { code: 'PAYLOAD_TOO_LARGE', message: 'too big' } }, 413),
    )
    const err = (await uploadBundleFiles('b', []).catch((e: unknown) => e)) as ApiError
    expect(err.status).toBe(413)
    expect(err.code).toBe('PAYLOAD_TOO_LARGE')
  })
})
