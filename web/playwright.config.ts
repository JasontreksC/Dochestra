import { defineConfig } from '@playwright/test'

// No default on purpose: the user's own stack must never be hit by accident.
// Run through ../scripts/verify-stack.sh --ui, which provides an isolated stack.
const baseURL = process.env.E2E_BASE_URL
if (!baseURL) {
  throw new Error('E2E_BASE_URL is required (use ./scripts/verify-stack.sh --ui for an isolated stack)')
}

export default defineConfig({
  testDir: './e2e',
  timeout: 300_000,
  expect: { timeout: 15_000 },
  workers: 1,
  retries: 0,
  reporter: [['list']],
  use: {
    baseURL,
    viewport: { width: 1600, height: 1000 },
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  outputDir: './node_modules/.tmp/playwright-results',
  projects: [{ name: 'chromium', use: { browserName: 'chromium' } }],
})
