import { test, expect } from '@playwright/test'
import { createHash, randomUUID } from 'node:crypto'
import { execFileSync } from 'node:child_process'

const database = process.env.DATABASE_URL || 'postgresql://postgres:postgres@127.0.0.1:55432/interdeck'
if (!['127.0.0.1', 'localhost', '[::1]'].includes(new URL(database).hostname)) throw new Error('Browser fixtures refuse remote databases')
const creator = randomUUID(), token = randomUUID()
const headers = { cookie: `interdeck_session=${token}` }
const sql = query => execFileSync('psql', [database, '-v', 'ON_ERROR_STOP=1', '-q', '-c', query], { stdio: 'pipe' })
const fixtures = []
const source = `---
theme: default
---

<!-- interdeck-slide: welcome -->
# Welcome fixture

---
layout: center
---

<!-- interdeck-slide: middle -->
# Middle fixture

---

<!-- interdeck-slide: closing -->
# Closing fixture
`
let deck
const preview = page => page.frameLocator('iframe[title="Interdeck deck preview"]')

test.beforeAll(() => {
  sql(`INSERT INTO users(id,google_sub,email,display_name) VALUES('${creator}','editor-${creator}','editor-${creator}@example.invalid','Editor fixture'); INSERT INTO auth_sessions(token_hash,user_id,expires_at) VALUES('${createHash('sha256').update(token).digest('base64url')}','${creator}',now()+interval '1 hour');`)
})

test.beforeEach(async ({ request, page, context, baseURL }) => {
  const response = await request.post('/api/decks', { headers, data: { title: 'Editor slide fixture', markdown: source } })
  expect(response.ok()).toBeTruthy()
  deck = await response.json()
  fixtures.push(deck.id)
  await context.addCookies([{ name: 'interdeck_session', value: token, url: baseURL, httpOnly: true, sameSite: 'Lax' }])
  await page.goto(`/decks/${deck.id}`)
  await expect(page.locator('.monaco-editor')).toBeVisible()
  await expect(preview(page).getByRole('heading', { name: 'Welcome fixture', exact: true })).toBeVisible()
})

test.afterAll(async ({ request }) => {
  for (const id of fixtures) expect((await request.delete(`/api/decks/${id}`, { headers })).ok()).toBeTruthy()
  sql(`DELETE FROM users WHERE id='${creator}'`)
})

for (const scope of ['slide', 'markdown']) {
  test(`assistant insertion keeps ${scope === 'slide' ? 'Current slide' : 'All slides'} mode and navigates to the new slide`, async ({ page, request }) => {
    await page.locator('#editor-source-kind').selectOption(scope)
    const proposed = source.replace('---\nlayout: center', '---\n\n<!-- interdeck-slide: assistant-added -->\n# Assistant added fixture\n\n---\nlayout: center')
    let instruction
    await page.route(`**/api/decks/${deck.id}/assistant/propose`, async route => {
      instruction = route.request().postDataJSON()
      await route.fulfill({ json: {
        proposal_id: randomUUID(), action: 'propose_patch', message: 'Add a slide', summary: 'Added a slide after Welcome',
        // Also covers an older server/model returning the previous focus key.
        focus_slide_key: 'welcome', operations: [{ kind: 'replace', old_text: '# Welcome fixture', new_text: '# Welcome fixture\n\n---\n\n# Assistant added fixture' }],
        proposed_markdown: proposed, base_version: instruction.expected_version, source_hash: createHash('sha256').update(source).digest('hex'),
      } })
    })
    let releaseValidation, validationStarted = false
    const validation = new Promise(resolve => { releaseValidation = resolve })
    await page.route('**/_gateway/slidev/preflight', async route => {
      const response = await route.fetch()
      validationStarted = true
      await validation
      await route.fulfill({ response })
    })
    try {
      await page.getByRole('textbox', { name: /^Ask the slide agent/ }).fill('Add a slide after this one')
      await page.getByRole('button', { name: 'Ask Agent', exact: true }).click()
      await page.getByRole('button', { name: 'Apply to deck', exact: true }).click()
      expect(instruction.editor_scope).toBe(scope)
      expect(instruction.current_slide_number).toBe(1)
      await expect.poll(() => validationStarted).toBe(true)
      // A stale acknowledgement from the old iframe must not steal focus.
      await preview(page).locator('body').evaluate(() => window.parent.postMessage({ type: 'interdeck:navigation', commandId: 'editor-organizer-1', slideNumber: 1, slideKey: 'welcome', version: 1 }, window.location.origin))
      await expect(page.locator('#editor-source-kind')).toHaveValue(scope)
      if (scope === 'slide') await expect(page.locator('#editor-source-description')).toContainText('Slide 2 source')
    } finally { releaseValidation() }
    await expect(preview(page).getByRole('heading', { name: 'Assistant added fixture', exact: true })).toBeVisible()
    await expect(page.locator('#editor-source-kind')).toHaveValue(scope)
    await expect(page.locator('.save-status')).toHaveText('Draft saved')
    const saved = await (await request.get(`/api/decks/${deck.id}`, { headers })).json()
    expect(saved.slides.map(slide => slide.key)).toEqual(['welcome', 'assistant-added', 'middle', 'closing'])
    expect(saved.markdown).toContain('layout: center')
  })

  test(`Add slide menu inserts after the current slide in ${scope === 'slide' ? 'Current slide' : 'All slides'} mode`, async ({ page, request }) => {
    await page.locator('#editor-source-kind').selectOption('organizer')
    await page.locator('.organizer-slide-main').filter({ hasText: 'Closing fixture' }).click()
    await expect(preview(page).getByRole('heading', { name: 'Closing fixture', exact: true })).toBeVisible()
    await page.locator('#editor-source-kind').selectOption(scope)
    await page.getByRole('button', { name: 'Add slide content', exact: true }).click()
    const addSlide = page.getByRole('menuitem', { name: /^Add slide/ })
    await expect(addSlide).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(page.getByRole('menu', { name: 'Add slide content' })).toHaveCount(0)
    await expect(preview(page).getByRole('heading', { name: 'New slide', exact: true })).toBeVisible()
    await expect(page.locator('#editor-source-kind')).toHaveValue(scope)
    await expect(page.locator('.save-status')).toHaveText('Draft saved')
    if (scope === 'slide') await expect(page.locator('#editor-source-description')).toContainText('Slide 4 source')
    const saved = await (await request.get(`/api/decks/${deck.id}`, { headers })).json()
    expect(saved.slides.map(slide => slide.key)).toEqual(['welcome', 'middle', 'closing', 'new-slide'])
    expect(saved.markdown).toContain('layout: center')
  })
}
