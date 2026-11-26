import os from 'node:os'
import fs from 'node:fs'
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

async function createWorkflow(page: Page, name: string, origin = '') {
  await page.goto(`${origin}/editor`)
  await page.getByLabel('새 워크플로 이름').fill(name)
  await page.getByRole('button', { name: '생성' }).click()
  await expect(page).toHaveURL(/\/editor\/[0-9a-f-]{36}$/)
  await expect(page.getByRole('heading', { level: 1, name })).toBeVisible()
}

const palette = (page: Page) => page.getByRole('navigation', { name: '노드 팔레트' })
const inspectorHeading = (page: Page, label: string) =>
  page.getByRole('complementary', { name: '인스펙터' }).getByRole('heading', { level: 2, name: label })

test('palette: drag a node onto the canvas places and selects it at the drop point', async ({ page }) => {
  await createWorkflow(page, `${RUN_TAG}-dnd`)
  await expect(palette(page).getByText('클릭하거나 캔버스로 끌어다 놓으세요')).toBeVisible()

  const pane = page.locator('.react-flow__pane')
  const paneBox = await pane.boundingBox()
  if (!paneBox) throw new Error('pane has no bounding box')
  const target = { x: Math.round(paneBox.width * 0.3), y: Math.round(paneBox.height * 0.4) }
  await palette(page).getByRole('button', { name: '추출' }).dragTo(pane, { targetPosition: target })

  const nodes = page.locator('.react-flow__node')
  await expect(nodes).toHaveCount(1)
  const added = node(page, '추출')
  await expect(added).toBeVisible()
  const c = await center(added)
  expect(Math.abs(c.x - (paneBox.x + target.x))).toBeLessThan(120)
  expect(Math.abs(c.y - (paneBox.y + target.y))).toBeLessThan(120)
  await expect(added).toHaveClass(/selected/)
  await expect(inspectorHeading(page, '추출')).toBeVisible()
  await expect(page.getByText('저장되지 않음')).toBeVisible()
})

test('palette: click-adding three nodes yields non-overlapping, visible nodes', async ({ page }) => {
  await createWorkflow(page, `${RUN_TAG}-click`)
  const labels = ['PDF 번들', 'PDF 선택', '추출']
  for (const [i, label] of labels.entries()) {
    await palette(page).getByRole('button', { name: label }).click()
    await expect(page.locator('.react-flow__node')).toHaveCount(i + 1)
    await expect(inspectorHeading(page, label)).toBeVisible()
  }
  await page.waitForTimeout(400) // let any pan transition settle

  const boxes = await Promise.all(labels.map((l) => node(page, l).boundingBox()))
  const viewport = page.viewportSize()
  for (const [i, a] of boxes.entries()) {
    if (!a || !viewport) throw new Error(`node ${labels[i]} has no bounding box`)
    expect(a.x).toBeGreaterThanOrEqual(0)
    expect(a.y).toBeGreaterThanOrEqual(0)
    expect(a.x + a.width).toBeLessThanOrEqual(viewport.width)
    expect(a.y + a.height).toBeLessThanOrEqual(viewport.height)
    for (const b of boxes.slice(i + 1)) {
      if (!b) continue
      const overlap =
        a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
      expect(overlap, `${JSON.stringify(a)} overlaps ${JSON.stringify(b)}`).toBe(false)
    }
  }
  await expect(page.locator('.react-flow__node.selected')).toHaveCount(1)
  await expect(node(page, '추출')).toHaveClass(/selected/)
})

test('editor: create, wire, save, reload, run, and report issues', async ({ page }) => {
  const wfName = `${RUN_TAG}-flow`
  const bundleName = `${RUN_TAG}-sample`

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
    // No bundle yet: a prominent upload button is shown right away and the upload creates one.
    await expect(page.getByText('PDF 를 업로드하면 번들이 자동으로 만들어집니다')).toBeVisible()
    await expect(page.getByRole('button', { name: 'PDF 업로드' })).toBeVisible()
    // The file input is hidden behind the button; the name carries RUN_TAG so cleanup finds the bundle.
    await page.getByLabel('PDF 파일 선택').setInputFiles({
      name: `${bundleName}.pdf`,
      mimeType: 'application/pdf',
      buffer: fs.readFileSync(SAMPLE_PDF),
    })
    await expect(page.getByText(`${bundleName}.pdf`)).toBeVisible()
    await expect(page.getByRole('combobox', { name: '번들' })).toContainText(bundleName)

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

/** Click-add a bundle, drop a PDF picker, and wire them — every step mints ids via newId(). */
async function addDropConnect(page: Page) {
  await palette(page).getByRole('button', { name: 'PDF 번들' }).click()
  await expect(node(page, 'PDF 번들')).toBeVisible()
  const pane = page.locator('.react-flow__pane')
  const paneBox = await pane.boundingBox()
  if (!paneBox) throw new Error('pane has no bounding box')
  await palette(page)
    .getByRole('button', { name: 'PDF 선택' })
    .dragTo(pane, { targetPosition: { x: Math.round(paneBox.width * 0.15), y: Math.round(paneBox.height * 0.3) } })
  await expect(page.locator('.react-flow__node')).toHaveCount(2)
  await expect(inspectorHeading(page, 'PDF 선택')).toBeVisible()
  await drag(page, handle(node(page, 'PDF 번들'), 'files', 'source'), handle(node(page, 'PDF 선택'), 'files', 'target'))
  await expect(page.locator('.react-flow__edge')).toHaveCount(1)
}

test('insecure context: works without crypto.randomUUID', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(Crypto.prototype, 'randomUUID', { value: undefined, configurable: true })
  })
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  await createWorkflow(page, `${RUN_TAG}-nouuid`)
  expect(await page.evaluate(() => typeof crypto.randomUUID)).toBe('undefined')
  await addDropConnect(page)
  expect(errors).toEqual([])
})

/** First non-loopback IPv4 of this host: http://<ip> is not a secure context (unlike localhost). */
function hostIp(): string | undefined {
  return Object.values(os.networkInterfaces())
    .flat()
    .find((a) => a && a.family === 'IPv4' && !a.internal)?.address
}

test('insecure context: real http://<host IP> origin', async ({ page, baseURL }) => {
  const ip = hostIp()
  test.skip(!ip || !baseURL, 'no non-loopback IPv4 address on this host')
  const origin = new URL(baseURL!)
  origin.hostname = ip!
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  await createWorkflow(page, `${RUN_TAG}-ip`, origin.origin)
  expect(await page.evaluate(() => window.isSecureContext)).toBe(false)
  expect(await page.evaluate(() => typeof crypto.randomUUID)).toBe('undefined')
  await addDropConnect(page)
  expect(errors).toEqual([])
})

const REPORT_PDF = path.resolve(import.meta.dirname, '../../fixtures/live/report_en.pdf')

// Real, paid OpenRouter calls (embedding + LLM): opt in with E2E_LIVE=1.
test('rag chat: ask the RAG node about the indexed report from the inspector', async ({ page }) => {
  test.skip(process.env.E2E_LIVE !== '1', 'live OpenRouter test: set E2E_LIVE=1 to run')
  await createWorkflow(page, `${RUN_TAG}-rag`)

  const labels = ['PDF 번들', 'PDF 선택', '추출', 'RAG 챗봇']
  for (const [i, label] of labels.entries()) {
    await palette(page).getByRole('button', { name: label, exact: true }).click()
    await expect(page.locator('.react-flow__node')).toHaveCount(i + 1)
  }
  await fitView(page)
  const [bundleNode, pickNode, extractNode, ragNode] = labels.map((l) => node(page, l))

  const edges = page.locator('.react-flow__edge')
  await drag(page, handle(bundleNode!, 'files', 'source'), handle(pickNode!, 'files', 'target'))
  await drag(page, handle(pickNode!, 'pdf', 'source'), handle(extractNode!, 'pdf', 'target'))
  await drag(page, handle(extractNode!, 'md', 'source'), handle(ragNode!, 'md', 'target'))
  await expect(edges).toHaveCount(3)

  await bundleNode!.locator('span.font-semibold').click()
  await page.getByLabel('PDF 파일 선택').setInputFiles({
    name: `${RUN_TAG}-report.pdf`,
    mimeType: 'application/pdf',
    buffer: fs.readFileSync(REPORT_PDF),
  })
  await expect(page.getByRole('combobox', { name: '번들' })).toContainText(RUN_TAG)

  await ragNode!.locator('span.font-semibold').click()
  await page.getByRole('checkbox', { name: '출력 노드로 지정' }).check()
  const chat = page.getByRole('region', { name: 'RAG 챗봇' })
  await expect(chat.getByText('워크플로를 실행해 RAG 인덱스를 만든 뒤 질문할 수 있습니다')).toBeVisible()

  await page.getByRole('button', { name: '실행' }).click()
  await expect(page.locator('.react-flow__node [data-status="succeeded"]')).toHaveCount(4, {
    timeout: 240_000,
  })

  await ragNode!.locator('span.font-semibold').click()
  await expect(chat.getByText(/인덱스: 청크 \d+개 · 실행 [0-9a-f]{8}/)).toBeVisible()
  await chat.getByRole('textbox', { name: '질문 입력' }).fill("What was Aurora Robotics' total revenue in 2025?")
  await chat.getByRole('button', { name: '보내기' }).click()
  await expect(chat.getByText('답변 생성 중…')).toBeVisible()
  const answer = chat.getByTestId('rag-answer')
  await expect(answer).toBeVisible({ timeout: 120_000 })
  await expect(answer).toContainText('482')

  const sourcesToggle = chat.getByRole('button', { name: /^근거 \d+개$/ })
  await expect(sourcesToggle).toBeVisible()
  await sourcesToggle.click()
  expect(await chat.getByTestId('rag-source').count()).toBeGreaterThanOrEqual(1)
  await page.screenshot({ path: 'node_modules/.tmp/playwright-results/rag-chat.png' })
})
