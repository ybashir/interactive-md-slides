import { defineConfig } from '@playwright/test'

const baseURL = process.env.BROWSER_BASE_URL || 'http://127.0.0.1:3000'
if (!['127.0.0.1', 'localhost', '[::1]'].includes(new URL(baseURL).hostname)) throw new Error('Browser regressions require a disposable local installation')
export default defineConfig({
  testDir: './scripts/browser',
  workers: 1,
  timeout: 60000,
  retries: 0,
  reporter: 'list',
  use: { baseURL, browserName: 'chromium', headless: true, trace: 'retain-on-failure' },
})
