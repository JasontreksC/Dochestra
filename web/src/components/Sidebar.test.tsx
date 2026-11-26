import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Route, Routes, useLocation } from 'react-router'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { callsTo, jsonResponse, renderWithProviders, stubApi } from '@/test/utils.tsx'
import Sidebar from './Sidebar.tsx'

const LIST = [
  { id: 'w2', name: 'Second', updatedAt: '2026-01-02T00:00:00Z' },
  { id: 'w1', name: 'First', updatedAt: '2026-01-01T00:00:00Z' },
]

function Location() {
  return <div data-testid="loc">{useLocation().pathname}</div>
}

function renderSidebar(route = '/editor') {
  return renderWithProviders(
    <>
      <Routes>
        <Route path="*" element={<Sidebar />} />
      </Routes>
      <Location />
    </>,
    { route },
  )
}

afterEach(() => vi.unstubAllGlobals())

describe('Sidebar', () => {
  it('renders the workflow list in the given order', async () => {
    stubApi({ 'GET /api/workflows': jsonResponse(LIST) })
    renderSidebar()
    const links = await screen.findAllByRole('link')
    expect(links.map((l) => l.textContent)).toEqual(['Second', 'First'])
    expect(links[0]).toHaveAttribute('href', '/editor/w2')
  })

  it('marks the active workflow with aria-current', async () => {
    stubApi({ 'GET /api/workflows': jsonResponse(LIST) })
    renderSidebar('/editor/w1')
    expect(await screen.findByRole('link', { name: 'First' })).toHaveAttribute(
      'aria-current',
      'page',
    )
    expect(screen.getByRole('link', { name: 'Second' })).not.toHaveAttribute('aria-current')
  })

  it('shows a loading state, then the empty message', async () => {
    stubApi({ 'GET /api/workflows': jsonResponse([]) })
    renderSidebar()
    expect(screen.getByText('불러오는 중…')).toBeInTheDocument()
    expect(await screen.findByText('워크플로가 없습니다')).toBeInTheDocument()
  })

  it('shows an error with a retry button', async () => {
    let calls = 0
    stubApi({
      'GET /api/workflows': () => {
        calls += 1
        return calls === 1
          ? jsonResponse({ error: { code: 'INTERNAL', message: 'db down' } }, 500)
          : jsonResponse(LIST)
      },
    })
    renderSidebar()
    expect(await screen.findByRole('alert')).toHaveTextContent('db down')
    await userEvent.click(screen.getByRole('button', { name: '다시 시도' }))
    expect(await screen.findByRole('link', { name: 'Second' })).toBeInTheDocument()
  })

  it('disables the create button for a blank name', async () => {
    stubApi({ 'GET /api/workflows': jsonResponse([]) })
    renderSidebar()
    const button = screen.getByRole('button', { name: '생성' })
    expect(button).toBeDisabled()
    await userEvent.type(screen.getByLabelText('새 워크플로 이름'), '   ')
    expect(button).toBeDisabled()
    await userEvent.type(screen.getByLabelText('새 워크플로 이름'), 'x')
    expect(button).toBeEnabled()
  })

  it('creates a workflow, clears the input and navigates to it', async () => {
    const mock = stubApi({
      'GET /api/workflows': jsonResponse([]),
      'POST /api/workflows': jsonResponse(
        {
          id: 'new1',
          name: 'Demo',
          graph: { nodes: [], edges: [] },
          createdAt: '2026-01-01T00:00:00Z',
          updatedAt: '2026-01-01T00:00:00Z',
        },
        201,
      ),
    })
    renderSidebar()
    const input = screen.getByLabelText('새 워크플로 이름')
    await userEvent.type(input, 'Demo')
    await userEvent.click(screen.getByRole('button', { name: '생성' }))

    await waitFor(() => expect(screen.getByTestId('loc')).toHaveTextContent('/editor/new1'))
    expect(input).toHaveValue('')
    const init = callsTo(mock, 'POST /api/workflows')[0]?.[1]
    expect(init?.body).toBe('{"name":"Demo"}')
  })

  it('shows the error message when creation fails', async () => {
    stubApi({
      'GET /api/workflows': jsonResponse([]),
      'POST /api/workflows': jsonResponse(
        { error: { code: 'BAD_REQUEST', message: '이름이 올바르지 않습니다' } },
        400,
      ),
    })
    renderSidebar()
    await userEvent.type(screen.getByLabelText('새 워크플로 이름'), 'Demo')
    await userEvent.click(screen.getByRole('button', { name: '생성' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('이름이 올바르지 않습니다')
    expect(screen.getByTestId('loc')).toHaveTextContent('/editor')
    expect(screen.getByTestId('loc')).not.toHaveTextContent('/editor/')
  })

  it('does not delete when confirm is cancelled', async () => {
    const mock = stubApi({ 'GET /api/workflows': jsonResponse(LIST) })
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false)
    renderSidebar()
    await userEvent.click(await screen.findByRole('button', { name: 'First 삭제' }))
    expect(confirm).toHaveBeenCalledWith("'First' 워크플로를 삭제할까요?")
    expect(callsTo(mock, 'DELETE /api/workflows/w1')).toHaveLength(0)
    confirm.mockRestore()
  })

  it('deletes after confirmation and stays put for another workflow', async () => {
    const mock = stubApi({
      'GET /api/workflows': jsonResponse(LIST),
      'DELETE /api/workflows/w1': new Response(null, { status: 204 }),
    })
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true)
    renderSidebar('/editor/w2')
    await userEvent.click(await screen.findByRole('button', { name: 'First 삭제' }))
    await waitFor(() => expect(callsTo(mock, 'DELETE /api/workflows/w1')).toHaveLength(1))
    expect(screen.getByTestId('loc')).toHaveTextContent('/editor/w2')
    confirm.mockRestore()
  })

  it('navigates to /editor when the open workflow is deleted', async () => {
    stubApi({
      'GET /api/workflows': jsonResponse(LIST),
      'DELETE /api/workflows/w1': new Response(null, { status: 204 }),
    })
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true)
    renderSidebar('/editor/w1')
    await userEvent.click(await screen.findByRole('button', { name: 'First 삭제' }))
    await waitFor(() => expect(screen.getByTestId('loc').textContent).toBe('/editor'))
    confirm.mockRestore()
  })
})
