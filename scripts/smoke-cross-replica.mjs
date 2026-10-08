// Proves that a state change handled by one API replica reaches an SSE client
// connected to a different replica through PostgreSQL LISTEN/NOTIFY.
//
//   docker compose up -d postgres
//   API_BIND_ADDR=127.0.0.1:18088 cargo run --manifest-path services/app/Cargo.toml
//   API_BIND_ADDR=127.0.0.1:18089 cargo run --manifest-path services/app/Cargo.toml
//   npm run smoke:cross-replica

import assert from 'node:assert/strict'
import { createHash, randomUUID } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import dotenv from 'dotenv'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
dotenv.config({ path: resolve(root, '.env') })

const options = parseArguments(process.argv.slice(2))
const writerUrl = new URL(options.writerUrl)
const listenerUrl = new URL(options.listenerUrl)
const databaseUrl = options.databaseUrl
const internalToken = process.env.INTERNAL_SERVICE_TOKEN || 'development-internal-token'
for (const [label, hostname] of [
  ['writer API', writerUrl.hostname],
  ['listener API', listenerUrl.hostname],
  ['database', new URL(databaseUrl).hostname],
]) {
  if (!['127.0.0.1', 'localhost', '::1'].includes(hostname) && process.env.ALLOW_REMOTE_SMOKE !== '1') {
    console.error(`Refusing to run the cross-replica smoke test against remote ${label} host ${hostname}.`)
    process.exit(2)
  }
}

assert.notEqual(writerUrl.origin, listenerUrl.origin, 'writer and listener must be different API instances')

const deckId = '00000000-0000-4000-8000-000000009101'
const userId = '00000000-0000-4000-8000-000000009102'
const runId = '00000000-0000-4000-8000-000000009103'
const revisionId = '00000000-0000-4000-8000-000000009104'
const joinCode = 'XRP-TST'
const creatorToken = `smoke-replica-${randomUUID()}`
const creatorCookie = `interdeck_session=${creatorToken}`

try {
  seedFixture()
  const abort = new AbortController()
  const eventPromise = waitForRemoteState(abort.signal)

  // Wait until replica B has flushed its SSE response. Only then mutate through A.
  const ready = await eventPromise.ready
  assert.equal(ready, 'connected')
  const connectedMetrics = await metrics(listenerUrl)
  assert.equal(connectedMetrics.event_listener_ready, true)
  assert.equal(connectedMetrics.active_sse_connections, 1)

  const response = await fetch(new URL(`/api/decks/${deckId}/navigate`, writerUrl), {
    method: 'POST',
    headers: {
      accept: 'application/json',
      'content-type': 'application/json',
      cookie: creatorCookie,
    },
    body: JSON.stringify({ slide_number: 2, click_step: 0 }),
  })
  assert.equal(response.status, 200, await response.text())

  const sequence = await eventPromise.state
  abort.abort()
  assert.ok(Number(sequence) >= 2, `expected a state sequence, received ${sequence}`)
  await waitFor(async () => (await metrics(listenerUrl)).active_sse_connections === 0)

  console.log(JSON.stringify({
    passed: true,
    writer: writerUrl.origin,
    listener: listenerUrl.origin,
    event: { type: 'state', sequence: Number(sequence) },
  }, null, 2))
}
finally {
  cleanupFixture()
}

async function metrics(baseUrl) {
  const response = await fetch(new URL('/internal/metrics', baseUrl), {
    headers: { 'x-interdeck-internal-token': internalToken },
  })
  if (!response.ok) throw new Error(`metrics returned ${response.status}: ${await response.text()}`)
  return response.json()
}

async function waitFor(predicate) {
  const deadline = Date.now() + 2_000
  while (Date.now() < deadline) {
    if (await predicate()) return
    await new Promise(resolve => setTimeout(resolve, 25))
  }
  throw new Error('timed out waiting for the operational metric to converge')
}

function waitForRemoteState(signal) {
  let resolveReady
  let resolveState
  let rejectReady
  let rejectState
  const ready = new Promise((resolve, reject) => { resolveReady = resolve; rejectReady = reject })
  const state = new Promise((resolve, reject) => { resolveState = resolve; rejectState = reject })
  const timeout = setTimeout(() => {
    const error = new Error('timed out waiting for a cross-replica state event')
    rejectReady(error)
    rejectState(error)
  }, 5_000)

  fetch(new URL(`/api/join/${joinCode}/events`, listenerUrl), {
    headers: { accept: 'text/event-stream' },
    signal,
  }).then(async response => {
    if (!response.ok || !response.body) throw new Error(`SSE endpoint returned ${response.status}`)
    const reader = response.body.pipeThrough(new TextDecoderStream()).getReader()
    let buffer = ''
    let event = 'message'
    while (true) {
      const { value, done } = await reader.read()
      if (done) break
      buffer += value
      const records = buffer.split('\n\n')
      buffer = records.pop() ?? ''
      for (const record of records) {
        let data = ''
        event = 'message'
        for (const line of record.split('\n')) {
          if (line.startsWith('event:')) event = line.slice(6).trim()
          if (line.startsWith('data:')) data += line.slice(5).trim()
        }
        if (event === 'connected') resolveReady('connected')
        if (event === 'state') {
          let snapshot
          try { snapshot = JSON.parse(data) } catch {}
          if (snapshot?.type === 'snapshot' && snapshot.state?.slide_number === 2) {
            clearTimeout(timeout)
            resolveState(String(snapshot.sequence))
            return
          }
        }
      }
    }
    throw new Error('SSE stream ended before a state event')
  }).catch(error => {
    if (error.name === 'AbortError') return
    clearTimeout(timeout)
    rejectReady(error)
    rejectState(error)
  })

  return { ready, state }
}

function seedFixture() {
  psql(`
BEGIN;
DELETE FROM decks WHERE id = '${deckId}';
DELETE FROM users WHERE id = '${userId}';

INSERT INTO users (id, google_sub, email, display_name)
VALUES ('${userId}', 'interdeck-replica-smoke', 'replica-smoke@localhost', 'Replica Smoke');

INSERT INTO auth_sessions (token_hash, user_id, expires_at)
VALUES ('${hashToken(creatorToken)}', '${userId}', now() + interval '1 hour');

INSERT INTO decks (id, owner_id, title, join_code, markdown, revision, result_epoch)
VALUES ('${deckId}', '${userId}', 'Replica smoke', '${joinCode}', $deck$<!-- interdeck-slide: first -->

# First

---
<!-- interdeck-slide: second -->

# Second
$deck$, 1, 1);

INSERT INTO deck_revisions (id, deck_id, revision, markdown, created_by)
VALUES ('${revisionId}', '${deckId}', 1, $deck$<!-- interdeck-slide: first -->

# First

---
<!-- interdeck-slide: second -->

# Second
$deck$, '${userId}');

INSERT INTO result_epochs (deck_id, epoch, created_by, reason)
VALUES ('${deckId}', 1, '${userId}', 'replica-smoke');

INSERT INTO presentation_runs (id, deck_id, source_revision, result_epoch, status, started_by)
VALUES ('${runId}', '${deckId}', 1, 1, 'live', '${userId}');

INSERT INTO live_deck_states (deck_id, run_id, slide_key, slide_number, click_step, sequence)
VALUES ('${deckId}', '${runId}', 'first', 1, 0, 1);
COMMIT;
`)
}

function cleanupFixture() {
  psql(`DELETE FROM decks WHERE id = '${deckId}'; DELETE FROM users WHERE id = '${userId}';`)
}

function psql(sql) {
  const result = spawnSync('psql', [databaseUrl, '-v', 'ON_ERROR_STOP=1', '-q', '-c', sql], {
    stdio: ['ignore', 'ignore', 'inherit'],
  })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`psql exited with ${result.status}`)
}

function hashToken(token) {
  return createHash('sha256').update(token).digest('base64url')
}

function parseArguments(arguments_) {
  const values = Object.fromEntries(arguments_.map(argument => {
    const [key, ...rest] = argument.replace(/^--/, '').split('=')
    return [key, rest.join('=')]
  }))
  return {
    writerUrl: values['writer-url'] || process.env.SMOKE_WRITER_API || 'http://127.0.0.1:18088',
    listenerUrl: values['listener-url'] || process.env.SMOKE_LISTENER_API || 'http://127.0.0.1:18089',
    databaseUrl: values['database-url'] || process.env.DATABASE_URL || 'postgresql://postgres:postgres@127.0.0.1:55432/interdeck',
  }
}
