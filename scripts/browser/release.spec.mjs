import { test, expect } from '@playwright/test'
import AxeBuilder from '@axe-core/playwright'
import { createHash, randomUUID } from 'node:crypto'
import { execFileSync } from 'node:child_process'

const database = process.env.DATABASE_URL || 'postgresql://postgres:postgres@127.0.0.1:55432/interdeck'
if (!['127.0.0.1', 'localhost', '[::1]'].includes(new URL(database).hostname)) throw new Error('Browser fixtures refuse remote databases')
const creator = randomUUID(), token = randomUUID()
const sql = query => execFileSync('psql', [database, '-v', 'ON_ERROR_STOP=1', '-q', '-c', query], { stdio: 'pipe' })
const headers = { cookie: `interdeck_session=${token}` }
let deck

async function accessibility(page) {
  const results = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa']).analyze()
  expect(results.violations.map(item => ({ id: item.id, nodes: item.nodes.map(node => ({ target: node.target, summary: node.failureSummary })) }))).toEqual([])
}

test.beforeAll(async ({ request }) => {
  sql(`INSERT INTO users(id,google_sub,email,display_name) VALUES('${creator}','browser-${creator}','browser-${creator}@example.invalid','Browser fixture'); INSERT INTO auth_sessions(token_hash,user_id,expires_at) VALUES('${createHash('sha256').update(token).digest('base64url')}','${creator}',now()+interval '1 hour');`)
  const response = await request.post('/api/decks', { headers, data: { title: 'Browser fixture', markdown: '---\ntheme: default\n---\n# Browser fixture\n\n:::interact{type="poll" id="browser-poll" results="after-vote"}\n# Choose a color\n- [blue] Blue\n- [green] Green\n:::' } })
  expect(response.ok()).toBeTruthy()
  deck = await response.json()
  const build = await request.post(`/_gateway/slidev/preflight`, { data: { deck_id: deck.id, access: new URL((await (await request.get(`/api/decks/${deck.id}/slidev-access?mode=preview`, { headers })).json()).url, 'http://localhost').searchParams.get('access') } })
  expect(build.ok()).toBeTruthy()
})

test.afterAll(async ({ request }) => {
  if (deck) {
    await request.post(`/api/decks/${deck.id}/stop`, { headers })
    await request.delete(`/api/decks/${deck.id}`, { headers })
  }
  sql(`DELETE FROM users WHERE id='${creator}'`)
})

test('login and installation privacy are accessible without organization-specific branding', async ({ page }) => {
  await page.goto('/login')
  await expect(page.getByRole('link', { name: 'Continue with Google' })).toBeVisible()
  await accessibility(page)
  await page.getByRole('link', { name: 'Privacy and your data' }).click()
  await expect(page.getByRole('heading', { name: 'Privacy and your data' })).toBeVisible()
  await expect(page.getByText('AI question moderation is disabled here.')).toBeVisible()
  await accessibility(page)
})

test('creator dashboard and editor load a verified deck', async ({ page, context, baseURL }) => {
  await context.addCookies([{ name: 'interdeck_session', value: token, url: baseURL, httpOnly: true, sameSite: 'Lax' }])
  await page.goto('/')
  await expect(page.getByRole('heading', { name: 'Your decks' })).toBeVisible()
  await accessibility(page)
  await page.goto(`/decks/${deck.id}`)
  await expect(page.getByRole('textbox', { name: 'Deck title', exact: true })).toHaveValue('Browser fixture')
  await expect(page.locator('.monaco-editor').first()).toBeVisible()
  await accessibility(page)
})

test('the browser player sanitizes malformed nested tags and event handlers', async ({ page, request, context, baseURL }) => {
  await context.addCookies([{ name: 'interdeck_session', value: token, url: baseURL, httpOnly: true, sameSite: 'Lax' }])
  const response = await request.post('/api/decks', { headers, data: {
    title: 'Sanitization fixture',
    markdown: `# <b>Sanitization fixture</b> <scr<script>ipt>window.__interdeckXss = true</script>

<img src="/missing-fixture-image" onerror="window.__interdeckXss = true">
<svg onload="window.__interdeckXss = true"><g></g></svg>
<sty<style>le>body { color: red; }</style>
<style scoped>.slidev-layout h1 { color: rgb(10, 20, 30); }</style>
`,
  } })
  expect(response.ok()).toBeTruthy()
  const fixture = await response.json()
  try {
    await page.goto(`/decks/${fixture.id}/player`)
    await expect(page.getByRole('heading', { name: /Sanitization fixture/ })).toBeVisible()
    await expect(page.locator('.slidev-layout script, .slidev-layout style, .slidev-layout [onerror], .slidev-layout [onload]')).toHaveCount(0)
    await page.waitForTimeout(200)
    expect(await page.evaluate(() => Boolean(window.__interdeckXss))).toBe(false)
  } finally {
    expect((await request.delete(`/api/decks/${fixture.id}`, { headers })).ok()).toBeTruthy()
  }
})

test('a mobile audience votes by keyboard and after-vote tallies stay private to voters', async ({ browser, request, baseURL }) => {
  expect((await request.post(`/api/decks/${deck.id}/present`, { headers })).ok()).toBeTruthy()
  const voterContext = await browser.newContext({ baseURL, viewport: { width: 390, height: 844 } })
  const observerContext = await browser.newContext({ baseURL, viewport: { width: 390, height: 844 } })
  try {
    const voter = await voterContext.newPage(), observer = await observerContext.newPage()
    for (const page of [voter, observer]) {
      await page.goto(`/j/${deck.join_code}`)
      await page.getByRole('button', { name: 'Continue anonymously' }).click()
      await expect(page.getByRole('heading', { name: 'Choose a color' })).toBeVisible()
    }
    const choice = voter.getByRole('button', { name: /^Blue/ })
    await choice.focus()
    await voter.keyboard.press('Enter')
    await expect(voter.getByText('1 response', { exact: true })).toBeVisible()
    await expect(choice).toHaveAttribute('aria-pressed', 'true')
    await expect(observer.getByText('Results appear after you respond', { exact: true })).toBeVisible()
    await accessibility(voter)
    await voter.getByRole('button', { name: /^Q&A/ }).click()
    await voter.getByRole('textbox', { name: 'Ask the presenter' }).fill('Can we discuss this example?')
    await voter.getByRole('button', { name: 'Submit', exact: true }).click()
    await expect(voter.getByText('Your question is awaiting moderation.')).toBeVisible()
    await accessibility(voter)
  } finally {
    await voterContext.close(); await observerContext.close()
  }
})

test('the restricted Slidev exporter renders a PDF and its PPTX library writes an image deck', async ({ page, request, context, baseURL }) => {
  await context.addCookies([{ name: 'interdeck_session', value: token, url: baseURL, httpOnly: true, sameSite: 'Lax' }])
  const access = await (await request.get(`/api/decks/${deck.id}/slidev-access?mode=export`, { headers })).json()
  const exportURL = new URL(access.url, baseURL)
  exportURL.pathname += 'export'
  await page.goto(exportURL.toString())
  await expect(page.locator('.print-slide-container').first()).toBeVisible({ timeout: 45000 })
  await page.emulateMedia({ media: 'print' })
  await expect(page.locator('.print-slide-container').getByRole('heading', { name: 'Browser fixture', exact: true }).first()).toBeVisible({ timeout: 45000 })
  await page.evaluate(async () => {
    await document.fonts.ready
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))
  })
  const pdf = await page.pdf({ printBackground: true, preferCSSPageSize: true })
  expect(pdf.subarray(0, 5).toString()).toBe('%PDF-')
  expect(pdf.length).toBeGreaterThan(5000)
  const moduleURL = new URL(`${new URL(access.url, baseURL).pathname}@id/pptxgenjs`, baseURL).toString()
  const output = await page.evaluate(async url => {
    const { default: PptxGenJS } = await import(url)
    const pptx = new PptxGenJS()
    const slide = pptx.addSlide()
    slide.addText('Browser export fixture', { x: 1, y: 1, w: 5, h: 1 })
    slide.addImage({ data: 'image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6ZAAAAABJRU5ErkJggg==', x: 1, y: 2, w: 1, h: 1 })
    return pptx.write({ outputType: 'base64' })
  }, moduleURL)
  const pptx = Buffer.from(output, 'base64')
  expect(pptx.subarray(0, 2).toString()).toBe('PK')
  expect(pptx.length).toBeGreaterThan(5000)
})
