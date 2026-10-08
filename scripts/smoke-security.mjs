import assert from 'node:assert/strict'
import { createHash, randomUUID } from 'node:crypto'
import { execFileSync } from 'node:child_process'

const base = process.env.SMOKE_BASE_URL || 'http://127.0.0.1:8080'
const database = process.env.DATABASE_URL || 'postgresql://postgres:postgres@127.0.0.1:55432/interdeck'
for (const target of [base, database]) assert.ok(['127.0.0.1', 'localhost', '[::1]'].includes(new URL(target).hostname), 'security smoke refuses remote targets')
const userId = randomUUID()
const creatorToken = randomUUID()
const cookie = `interdeck_session=${creatorToken}`
const hash = createHash('sha256').update(creatorToken).digest('base64url')
const psql = sql => execFileSync('psql', [database, '-v', 'ON_ERROR_STOP=1', '-q', '-c', sql], { stdio: 'pipe' })
let deck
async function api(path, method = 'GET', body, auth = cookie) {
  const response = await fetch(new URL(path, base), { method, headers: { ...(auth ? { cookie: auth } : {}), ...(body === undefined ? {} : { 'content-type': 'application/json' }) }, body: body === undefined ? undefined : JSON.stringify(body) })
  assert.ok(response.ok, `request failed: ${method} ${path} ${response.status}`)
  return response.status === 204 ? null : response.json()
}
async function snapshot(path) {
  const abort = new AbortController()
  const timer = setTimeout(() => abort.abort(), 5000)
  try {
    const response = await fetch(new URL(path, base), { signal: abort.signal })
    const reader = response.body.pipeThrough(new TextDecoderStream()).getReader()
    let buffer = ''
    while (true) {
      const { value, done } = await reader.read()
      assert.ok(!done, 'SSE ended before its snapshot')
      buffer += value
      const lines = buffer.split('\n'); buffer = lines.pop()
      for (const line of lines) if (line.startsWith('data: {')) return JSON.parse(line.slice(6)).state
    }
  } finally { clearTimeout(timer); abort.abort() }
}
try {
  psql(`INSERT INTO users(id,google_sub,email,display_name) VALUES('${userId}','security-${userId}','security@example.invalid','Security fixture'); INSERT INTO auth_sessions(token_hash,user_id,expires_at) VALUES('${hash}','${userId}',now()+interval '1 hour');`)
  const markdown = ['---\ntheme: default\n---\n# Privacy regression', ...['manual', 'hidden', 'after-vote'].map(policy => `:::interact{type="poll" id="${policy}" results="${policy}"}\n# Choose\n- [one] One\n- [two] Two\n:::`)].join('\n\n')
  deck = await api('/api/decks', 'POST', { title: 'Security fixture', markdown })
  const uploads = await Promise.all(Array.from({ length: 8 }, async () => {
    const form = new FormData()
    form.set('file', new Blob([Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6ZAAAAABJRU5ErkJggg==', 'base64')], { type: 'image/png' }), 'fixture.png')
    const response = await fetch(new URL(`/api/decks/${deck.id}/assets`, base), { method: 'POST', headers: { cookie }, body: form })
    assert.ok(response.ok, `concurrent duplicate upload failed: ${response.status}`)
    return response.json()
  }))
  assert.equal(new Set(uploads.map(item => item.id)).size, 1, 'duplicates must resolve to one stored asset')
  const build = await fetch(new URL(`/internal/decks/${deck.id}/builds`, base), { method: 'POST', headers: { 'content-type': 'application/json', 'x-interdeck-internal-token': process.env.INTERNAL_SERVICE_TOKEN || 'development-internal-token' }, body: JSON.stringify({ source_version: 1, status: 'succeeded', slidev_version: '52.19.1', theme: 'default' }) })
  assert.ok(build.ok)
  await api(`/api/decks/${deck.id}/present`, 'POST')
  async function join() {
    const response = await fetch(new URL(`/api/join/${deck.join_code}`, base), { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}' })
    assert.ok(response.ok); return response.headers.get('set-cookie').split(';')[0]
  }
  const voter = await join(), observer = await join()
  for (const policy of ['manual', 'hidden', 'after-vote']) await api(`/api/join/${deck.join_code}/interactions/${policy}`, 'PUT', { idempotency_key: randomUUID(), payload: { option_id: 'one' } }, voter)
  await new Promise(resolve => setTimeout(resolve, 300))
  const shared = await snapshot(`/api/join/${deck.join_code}/events`)
  for (const interaction of shared.interactions) assert.deepEqual(interaction.result, { hidden: true }, 'shared SSE must contain no protected tallies')
  const mine = await api(`/api/join/${deck.join_code}/state`, 'GET', undefined, voter)
  const other = await api(`/api/join/${deck.join_code}/state`, 'GET', undefined, observer)
  assert.equal(mine.interactions.find(item => item.id === 'after-vote').result.count, 1)
  assert.deepEqual(other.interactions.find(item => item.id === 'after-vote').result, { hidden: true })
  await api(`/api/decks/${deck.id}/interactions/manual/control`, 'POST', { action: 'reveal' })
  const revealed = await api(`/api/join/${deck.join_code}/state`, 'GET', undefined, observer)
  assert.equal(revealed.interactions.find(item => item.id === 'manual').result.count, 1)
  psql(`UPDATE participant_sessions SET expires_at = now() - interval '1 second' WHERE deck_id = '${deck.id}'`)
  const expired = await api(`/api/join/${deck.join_code}/state`, 'GET', undefined, voter)
  assert.equal(expired.audience_session_active, false)
  assert.deepEqual(expired.interactions.find(item => item.id === 'after-vote').result, { hidden: true })
  const start = await fetch(new URL('/api/auth/google/start', base), { redirect: 'manual' })
  assert.equal(start.status, 307)
  assert.match(start.headers.get('set-cookie'), /interdeck_oauth=.*HttpOnly; SameSite=Lax/)
  const state = new URL(start.headers.get('location')).searchParams.get('state')
  const foreign = await fetch(new URL(`/api/auth/google/callback?state=${state}&code=fixture`, base), { redirect: 'manual' })
  assert.equal(foreign.status, 400, 'callback without browser binding must fail before calling Google')
  console.log(JSON.stringify({ ok: true, sse_privacy: true, after_vote_authorization: true, manual_reveal: true, session_expiry: true, oauth_browser_binding: true, concurrent_upload_deduplication: true }))
} finally {
  if (deck) psql(`BEGIN; INSERT INTO asset_gc(storage_key) SELECT storage_key FROM deck_assets WHERE deck_id='${deck.id}' ON CONFLICT DO NOTHING; DELETE FROM decks WHERE id = '${deck.id}'; COMMIT;`)
  psql(`DELETE FROM users WHERE id = '${userId}'`)
}
