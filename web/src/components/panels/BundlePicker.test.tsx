import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { useState } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { callsTo, jsonResponse, renderWithProviders, stubApi } from '@/test/utils.tsx'
import BundlePicker from './BundlePicker.tsx'

const B1 = '11111111-1111-4111-8111-111111111111'
const B2 = '22222222-2222-4222-8222-222222222222'
const LIST = [
  { id: B1, name: 'Alpha', fileCount: 2, createdAt: '' },
  { id: B2, name: 'Beta', fileCount: 0, createdAt: '' },
]
const DETAIL = {
  id: B1,
  name: 'Alpha',
  createdAt: '',
  files: [
    { id: 'f1', originalName: 'a.pdf', sizeBytes: 2048, position: 0, createdAt: '' },
    { id: 'f2', originalName: 'b.pdf', sizeBytes: 3 * 1024 * 1024, position: 1, createdAt: '' },
  ],
}

afterEach(() => vi.unstubAllGlobals())

function Harness({ initial, spy }: { initial: string | null; spy?: (v: string | null) => void }) {
  const [v, setV] = useState(initial)
  return (
    <BundlePicker
      value={v}
      onChange={(id) => {
        spy?.(id)
        setV(id)
      }}
    />
  )
}

describe('BundlePicker', () => {
  it('lists bundles and reports the selection', async () => {
    stubApi({ 'GET /api/bundles': jsonResponse(LIST), [`GET /api/bundles/${B2}`]: jsonResponse({ ...DETAIL, id: B2, files: [] }) })
    const spy = vi.fn()
    renderWithProviders(<Harness initial={null} spy={spy} />)
    const select = screen.getByRole('combobox', { name: '번들' })
    expect(await screen.findByRole('option', { name: 'Alpha (2)' })).toBeInTheDocument()
    expect(screen.getByRole('option', { name: '선택 없음' })).toBeInTheDocument()
    await userEvent.selectOptions(select, B2)
    expect(spy).toHaveBeenCalledWith(B2)
    await userEvent.selectOptions(select, '')
    expect(spy).toHaveBeenLastCalledWith(null)
  })

  it('warns when the bundle id is not in the list', async () => {
    stubApi({ 'GET /api/bundles': jsonResponse(LIST) })
    renderWithProviders(<Harness initial="99999999-9999-4999-8999-999999999999" />)
    expect(await screen.findByText('번들을 찾을 수 없습니다')).toBeInTheDocument()
  })

  it('creates a bundle and selects it automatically', async () => {
    const mock = stubApi({
      'GET /api/bundles': jsonResponse(LIST),
      'POST /api/bundles': jsonResponse({ id: B2, name: 'New', createdAt: '', files: [] }, 201),
      [`GET /api/bundles/${B2}`]: jsonResponse({ id: B2, name: 'New', createdAt: '', files: [] }),
    })
    const spy = vi.fn()
    renderWithProviders(<Harness initial={null} spy={spy} />)
    await userEvent.type(screen.getByLabelText('새 번들 이름'), 'New')
    await userEvent.click(screen.getByRole('button', { name: '새 번들' }))
    await waitFor(() => expect(spy).toHaveBeenCalledWith(B2))
    expect(callsTo(mock, 'POST /api/bundles')[0]?.[1]?.body).toBe('{"name":"New"}')
    expect(screen.getByLabelText('새 번들 이름')).toHaveValue('')
  })

  it('shows files with sizes, deletes a file', async () => {
    const mock = stubApi({
      'GET /api/bundles': jsonResponse(LIST),
      [`GET /api/bundles/${B1}`]: jsonResponse(DETAIL),
      [`DELETE /api/bundles/${B1}/files/f1`]: new Response(null, { status: 204 }),
    })
    renderWithProviders(<Harness initial={B1} />)
    expect(await screen.findByText('a.pdf')).toBeInTheDocument()
    expect(screen.getByText('2.0 KB')).toBeInTheDocument()
    expect(screen.getByText('3.0 MB')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'a.pdf 삭제' }))
    await waitFor(() =>
      expect(callsTo(mock, `DELETE /api/bundles/${B1}/files/f1`)).toHaveLength(1),
    )
  })

  it('uploads PDFs as multipart "files"', async () => {
    const mock = stubApi({
      'GET /api/bundles': jsonResponse(LIST),
      [`GET /api/bundles/${B1}`]: jsonResponse(DETAIL),
      [`POST /api/bundles/${B1}/files`]: jsonResponse([], 201),
    })
    renderWithProviders(<Harness initial={B1} />)
    const input = await screen.findByLabelText('PDF 업로드')
    expect(input).toHaveAttribute('accept', 'application/pdf,.pdf')
    const f1 = new File(['%PDF-1'], 'x.pdf', { type: 'application/pdf' })
    const f2 = new File(['%PDF-2'], 'y.pdf', { type: 'application/pdf' })
    await userEvent.upload(input, [f1, f2])
    await waitFor(() => expect(callsTo(mock, `POST /api/bundles/${B1}/files`)).toHaveLength(1))
    const body = callsTo(mock, `POST /api/bundles/${B1}/files`)[0]?.[1]?.body as FormData
    expect((body.getAll('files') as File[]).map((f) => f.name)).toEqual(['x.pdf', 'y.pdf'])
  })

  it('shows the error when upload fails', async () => {
    stubApi({
      'GET /api/bundles': jsonResponse(LIST),
      [`GET /api/bundles/${B1}`]: jsonResponse(DETAIL),
      [`POST /api/bundles/${B1}/files`]: jsonResponse(
        { error: { code: 'BAD_REQUEST', message: 'PDF 가 아닙니다' } },
        400,
      ),
    })
    renderWithProviders(<Harness initial={B1} />)
    await userEvent.upload(
      await screen.findByLabelText('PDF 업로드'),
      new File(['x'], 'x.pdf', { type: 'application/pdf' }),
    )
    expect(await screen.findByText('PDF 가 아닙니다')).toBeInTheDocument()
  })
})
