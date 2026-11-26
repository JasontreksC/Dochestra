import { screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import App from './App.tsx'
import { jsonResponse, renderWithProviders, stubApi } from './test/utils.tsx'

afterEach(() => vi.unstubAllGlobals())

describe('App routes', () => {
  it('landing page links to the editor', () => {
    renderWithProviders(<App />, { route: '/' })
    expect(screen.getByRole('link', { name: '시작하기' })).toHaveAttribute('href', '/editor')
  })

  it('editor page renders the sidebar', async () => {
    stubApi({ 'GET /api/workflows': jsonResponse([]) })
    const { container } = renderWithProviders(<App />, { route: '/editor' })
    expect(container.querySelector('aside')).toBeInTheDocument()
    expect(await screen.findByText('워크플로가 없습니다')).toBeInTheDocument()
  })

  it('unknown paths redirect to the landing page', () => {
    renderWithProviders(<App />, { route: '/nope' })
    expect(screen.getByRole('heading', { name: 'Dochestra' })).toBeInTheDocument()
  })
})
