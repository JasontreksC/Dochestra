import { fireEvent, screen, waitFor } from '@testing-library/react'
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

  const pdf = (name: string) => new File(['%PDF-1'], name, { type: 'application/pdf' })
  const uploadKey = `POST /api/bundles/${B1}/files`

  it('shows a primary upload button and the empty-state hint', async () => {
    stubApi({
      'GET /api/bundles': jsonResponse(LIST),
      [`GET /api/bundles/${B1}`]: jsonResponse({ ...DETAIL, files: [] }),
    })
    renderWithProviders(<Harness initial={B1} />)
    expect(screen.getByRole('button', { name: 'PDF 업로드' })).toBeEnabled()
    expect(
      await screen.findByText(/아직 업로드한 PDF 가 없습니다 — 위 버튼을 누르거나/),
    ).toBeInTheDocument()
  })

  it('uploads to the selected bundle without creating a new one (multipart "files")', async () => {
    const mock = stubApi({
      'GET /api/bundles': jsonResponse(LIST),
      [`GET /api/bundles/${B1}`]: jsonResponse(DETAIL),
      [uploadKey]: jsonResponse([], 201),
    })
    renderWithProviders(<Harness initial={B1} />)
    const input = await screen.findByLabelText('PDF 파일 선택')
    expect(input).toHaveAttribute('accept', 'application/pdf,.pdf')
    await userEvent.upload(input, [pdf('x.pdf'), pdf('y.pdf')])
    await waitFor(() => expect(callsTo(mock, uploadKey)).toHaveLength(1))
    const body = callsTo(mock, uploadKey)[0]?.[1]?.body as FormData
    expect((body.getAll('files') as File[]).map((f) => f.name)).toEqual(['x.pdf', 'y.pdf'])
    expect(callsTo(mock, 'POST /api/bundles')).toHaveLength(0)
  })

  it('auto-creates a bundle named after the first file, selects it, then uploads', async () => {
    const mock = stubApi({
      'GET /api/bundles': jsonResponse(LIST),
      'POST /api/bundles': jsonResponse({ id: B2, name: 'report', createdAt: '', files: [] }, 201),
      [`GET /api/bundles/${B2}`]: jsonResponse({ id: B2, name: 'report', createdAt: '', files: [] }),
      [`POST /api/bundles/${B2}/files`]: jsonResponse([], 201),
    })
    const spy = vi.fn()
    renderWithProviders(<Harness initial={null} spy={spy} />)
    await userEvent.upload(screen.getByLabelText('PDF 파일 선택'), pdf('report.pdf'))
    await waitFor(() => expect(callsTo(mock, `POST /api/bundles/${B2}/files`)).toHaveLength(1))
    expect(callsTo(mock, 'POST /api/bundles')[0]?.[1]?.body).toBe('{"name":"report"}')
    expect(spy).toHaveBeenCalledWith(B2)
    const order = mock.mock.calls
      .map(([u, i]) => `${i?.method ?? 'GET'} ${String(u)}`)
      .filter((k) => k.startsWith('POST'))
    expect(order).toEqual(['POST /api/bundles', `POST /api/bundles/${B2}/files`])
  })

  it('names the bundle "<first> 외 N개" for several files and caps the name at 100 chars', async () => {
    const mock = stubApi({
      'GET /api/bundles': jsonResponse(LIST),
      'POST /api/bundles': jsonResponse({ id: B2, name: 'n', createdAt: '', files: [] }, 201),
      [`GET /api/bundles/${B2}`]: jsonResponse({ id: B2, name: 'n', createdAt: '', files: [] }),
      [`POST /api/bundles/${B2}/files`]: jsonResponse([], 201),
    })
    renderWithProviders(<Harness initial={null} />)
    await userEvent.upload(screen.getByLabelText('PDF 파일 선택'), [
      pdf('a.pdf'),
      pdf('b.pdf'),
      pdf('c.pdf'),
    ])
    await waitFor(() => expect(callsTo(mock, 'POST /api/bundles')).toHaveLength(1))
    expect(callsTo(mock, 'POST /api/bundles')[0]?.[1]?.body).toBe('{"name":"a 외 2개"}')
  })

  it('caps an auto-generated bundle name at 100 characters', async () => {
    const mock = stubApi({
      'GET /api/bundles': jsonResponse(LIST),
      'POST /api/bundles': jsonResponse({ id: B2, name: 'n', createdAt: '', files: [] }, 201),
      [`GET /api/bundles/${B2}`]: jsonResponse({ id: B2, name: 'n', createdAt: '', files: [] }),
      [`POST /api/bundles/${B2}/files`]: jsonResponse([], 201),
    })
    renderWithProviders(<Harness initial={null} />)
    await userEvent.upload(screen.getByLabelText('PDF 파일 선택'), pdf(`${'x'.repeat(150)}.pdf`))
    await waitFor(() => expect(callsTo(mock, 'POST /api/bundles')).toHaveLength(1))
    const sent = JSON.parse(String(callsTo(mock, 'POST /api/bundles')[0]?.[1]?.body)) as { name: string }
    expect(sent.name).toHaveLength(100)
  })

  it('accepts PDFs dropped onto the zone', async () => {
    const mock = stubApi({
      'GET /api/bundles': jsonResponse(LIST),
      [`GET /api/bundles/${B1}`]: jsonResponse(DETAIL),
      [uploadKey]: jsonResponse([], 201),
    })
    renderWithProviders(<Harness initial={B1} />)
    const zone = screen.getByTestId('bundle-dropzone')
    fireEvent.dragOver(zone)
    expect(zone.className).toContain('border-blue-500')
    fireEvent.drop(zone, { dataTransfer: { files: [pdf('dropped.pdf')] } })
    await waitFor(() => expect(callsTo(mock, uploadKey)).toHaveLength(1))
    const body = callsTo(mock, uploadKey)[0]?.[1]?.body as FormData
    expect((body.getAll('files') as File[]).map((f) => f.name)).toEqual(['dropped.pdf'])
  })

  it('does not upload when only non-PDF files are dropped', async () => {
    const mock = stubApi({ 'GET /api/bundles': jsonResponse(LIST) })
    renderWithProviders(<Harness initial={null} />)
    fireEvent.drop(screen.getByTestId('bundle-dropzone'), {
      dataTransfer: { files: [new File(['x'], 'a.txt', { type: 'text/plain' })] },
    })
    expect(await screen.findByRole('alert')).toHaveTextContent('PDF 파일만 올릴 수 있습니다')
    expect(callsTo(mock, 'POST /api/bundles')).toHaveLength(0)
  })

  it('disables the button and shows progress while uploading', async () => {
    let release: (r: Response) => void = () => {}
    stubApi({
      'GET /api/bundles': jsonResponse(LIST),
      [`GET /api/bundles/${B1}`]: jsonResponse(DETAIL),
      [uploadKey]: () => new Promise<Response>((r) => (release = r)) as unknown as Response,
    })
    renderWithProviders(<Harness initial={B1} />)
    await userEvent.upload(await screen.findByLabelText('PDF 파일 선택'), [pdf('a.pdf'), pdf('b.pdf')])
    expect(await screen.findByText('업로드 중… (2개)')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'PDF 업로드' })).toBeDisabled()
    release(jsonResponse([], 201))
    await waitFor(() => expect(screen.queryByText(/업로드 중…/)).not.toBeInTheDocument())
    expect(screen.getByRole('button', { name: 'PDF 업로드' })).toBeEnabled()
  })

  it('shows the error when upload fails', async () => {
    stubApi({
      'GET /api/bundles': jsonResponse(LIST),
      [`GET /api/bundles/${B1}`]: jsonResponse(DETAIL),
      [uploadKey]: jsonResponse({ error: { code: 'BAD_REQUEST', message: 'PDF 가 아닙니다' } }, 400),
    })
    renderWithProviders(<Harness initial={B1} />)
    await userEvent.upload(await screen.findByLabelText('PDF 파일 선택'), pdf('x.pdf'))
    expect(await screen.findByText('PDF 가 아닙니다')).toBeInTheDocument()
  })
})
