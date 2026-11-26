import path from 'node:path'
import { expect, test, type APIRequestContext, type Locator, type Page } from '@playwright/test'

const SAMPLE_PDF = path.resolve(import.meta.dirname, '../../extractor/tests/fixtures/sample.pdf')
const RUN_TAG = `pw-${Date.now().toString(36)}`

const node = (page: Page, label: string): Locator =>
  page.locator('.react-flow__node').filter({ hasText: label })
const handle = (n: Locator, id: string, type: 'source' | 'target'): Locator =>
  n.locator(`.react-flow__handle.${type}[data-handleid="${id}"]`)

async function center(l: Locator): Promise<{ x: number; y: number }> {
  const box = await l.boundingBox()
  if (!box) throw new Error('element has no bounding box')
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 }
}

/** Real mouse drag with intermediate moves (React Flow needs pointer movement). */
async function drag(page: Page, from: Locator, to: { x: number; y: number } | Locator) {
  const a = await center(from)
  const b = 'x' in to ? to : await center(to)
  await page.mouse.move(a.x, a.y)
  await page.mouse.down()
  await page.mouse.move((a.x + b.x) / 2, (a.y + b.y) / 2, { steps: 8 })
  await page.mouse.move(b.x, b.y, { steps: 8 })
  await page.mouse.up()
}

async function moveNode(page: Page, n: Locator, dx: number, dy: number) {
  const header = n.locator('span.font-semibold').first()
  const c = await center(header)
  await drag(page, header, { x: c.x + dx, y: c.y + dy })
}

/** Zoom/pan so every node (and its handles) is on screen. */
async function fitView(page: Page) {
  await page.locator('.react-flow__controls-fitview').click()
  await page.waitForTimeout(500) // let the viewport transition settle
}

async function cleanup(request: APIRequestContext) {
  const workflows = (await (await request.get('/api/workflows')).json()) as Array<{
    id: string
    name: string
  }>
  for (const w of workflows.filter((x) => x.name.startsWith(RUN_TAG))) {
    await request.delete(`/api/workflows/${w.id}`)
  }
  const bundles = (await (await request.get('/api/bundles')).json()) as Array<{
    id: string
    name: string
  }>
  for (const b of bundles.filter((x) => x.name.startsWith(RUN_TAG))) {
    await request.delete(`/api/bundles/${b.id}`)
  }
}

test.afterEach(async ({ request }) => {
  await cleanup(request)
})

test('editor: create, wire, save, reload, run, and report issues', async ({ page }) => {
  const wfName = `${RUN_TAG}-flow`
  const bundleName = `${RUN_TAG}-bundle`

  await test.step('1. landing -> editor, create a workflow from the sidebar', async () => {
    await page.goto('/')
    await page.getByRole('link', { name: '시작하기' }).click()
    await expect(page).toHaveURL(/\/editor$/)
    await page.getByLabel('새 워크플로 이름').fill(wfName)
    await page.getByRole('button', { name: '생성' }).click()
    await expect(page).toHaveURL(/\/editor\/[0-9a-f-]{36}$/)
    await expect(page.getByRole('heading', { level: 1, name: wfName })).toBeVisible()
  })

  const bundleNode = node(page, 'PDF 번들')
  const pickNode = node(page, 'PDF 선택')
  const extractNode = node(page, '추출')

  await test.step('2. add three nodes from the palette', async () => {
    const palette = page.getByRole('navigation', { name: '노드 팔레트' })
    await palette.getByRole('button', { name: 'PDF 번들' }).click()
    await moveNode(page, bundleNode, -320, -120)
    await palette.getByRole('button', { name: 'PDF 선택' }).click()
    await palette.getByRole('button', { name: '추출' }).click()
    await moveNode(page, extractNode, 280, 140)
    await expect(page.locator('.react-flow__node')).toHaveCount(3)
    await fitView(page)
  })

  await test.step('3. drag connections: valid ones connect, type mismatch is rejected', async () => {
    const edges = page.locator('.react-flow__edge')
    await expect(edges).toHaveCount(0)

    await drag(page, handle(bundleNode, 'files', 'source'), handle(pickNode, 'files', 'target'))
    await expect(edges).toHaveCount(1)

    // LIST[PDF] -> PDF: rejected (D-5), edge count unchanged
    await drag(page, handle(bundleNode, 'files', 'source'), handle(extractNode, 'pdf', 'target'))
    await expect(edges).toHaveCount(1)

    await drag(page, handle(pickNode, 'pdf', 'source'), handle(extractNode, 'pdf', 'target'))
    await expect(edges).toHaveCount(2)
  })

  await test.step('4. inspector: create + upload a bundle, mark extract as output', async () => {
    await bundleNode.locator('span.font-semibold').click()
    await page.getByLabel('새 번들 이름').fill(bundleName)
    await page.getByRole('button', { name: '새 번들' }).click()
    await expect(page.getByRole('combobox', { name: '번들' })).toContainText(bundleName)
    await page.getByLabel('PDF 업로드').setInputFiles(SAMPLE_PDF)
    await expect(page.getByText('sample.pdf')).toBeVisible()

    await extractNode.locator('span.font-semibold').click()
    await page.getByRole('checkbox', { name: '출력 노드로 지정' }).check()
    await expect(extractNode.getByText('출력')).toBeVisible()
  })

  await test.step('5. save, reload, and see the graph restored', async () => {
    await expect(page.getByText('저장되지 않음')).toBeVisible()
    await page.getByRole('button', { name: '저장' }).click()
    await expect(page.getByText('저장되지 않음')).toBeHidden()

    await page.reload()
    await expect(page.locator('.react-flow__node')).toHaveCount(3)
    await expect(page.locator('.react-flow__edge')).toHaveCount(2)
    await expect(extractNode.getByText('출력')).toBeVisible()
  })

  await test.step('6. run: all nodes end succeeded and the md artifact is linked', async () => {
    await page.getByRole('button', { name: '실행' }).click()
    await expect(page.locator('.react-flow__node [data-status="succeeded"]')).toHaveCount(3, {
      timeout: 180_000,
    })
    await expect(page.getByRole('link', { name: 'md.md' })).toBeVisible()
  })

  await test.step('7. without an output node the run is rejected with NO_OUTPUT_NODE', async () => {
    await extractNode.locator('span.font-semibold').click()
    await page.getByRole('checkbox', { name: '출력 노드로 지정' }).uncheck()
    await page.getByRole('button', { name: '실행' }).click()
    await expect(page.getByRole('alert').filter({ hasText: 'NO_OUTPUT_NODE' })).toContainText(
      '출력 노드가 없습니다',
    )
  })
})
