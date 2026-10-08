// Exercises per-interaction presenter controls against a running API and PostgreSQL.
//
//   docker compose up -d postgres
//   cargo run --manifest-path services/app/Cargo.toml
//   npm run smoke:interaction-locks
//
// The fixture is seeded directly through psql because creator sign-in requires
// Google Workspace OAuth, which a local smoke run cannot complete.
import assert from 'node:assert/strict'
import { createHash, randomUUID } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import dotenv from 'dotenv'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
dotenv.config({ path: resolve(root, '.env') })

const options = parseArguments(process.argv.slice(2))
const baseUrl = new URL(options.baseUrl)
const databaseUrl = options.databaseUrl
for (const [label, hostname] of [['API', baseUrl.hostname], ['database', new URL(databaseUrl).hostname]]) {
  if (!['127.0.0.1', 'localhost', '::1'].includes(hostname) && process.env.ALLOW_REMOTE_SMOKE !== '1') {
    console.error(`Refusing to run the lock smoke test against remote ${label} host ${hostname}. Set ALLOW_REMOTE_SMOKE=1 only for an isolated environment.`)
    process.exit(2)
  }
}

const deckId = '00000000-0000-4000-8000-000000009001'
const userId = '00000000-0000-4000-8000-000000009002'
const runId = '00000000-0000-4000-8000-000000009003'
const revisionId = '00000000-0000-4000-8000-000000009004'
const joinCode = 'LCK-TST'
const creatorToken = `smoke-lock-${randomUUID()}`
const creatorCookie = `interdeck_session=${creatorToken}`
const pollId = 'lock-poll'
const rankedId = 'lock-rank'
const rankingId = 'priority-order'
const hotspotId = 'focus-hotspot'
const surveyId = 'team-pulse'

const checks = []

try {
  seedFixture()

  const audience = await join()

  await step('a joined audience member can load a private image only through the live room', async () => {
    const pngBytes = Buffer.concat([
      Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
      Buffer.from('smoke'),
    ])
    const form = new FormData()
    form.append('file', new Blob([pngBytes], { type: 'image/png' }), 'smoke.png')
    const uploaded = await fetch(new URL(`/api/decks/${deckId}/assets`, baseUrl), {
      method: 'POST',
      headers: { cookie: creatorCookie },
      body: form,
    })
    assert.equal(uploaded.status, 201)
    const asset = await uploaded.json()

    const audienceImage = await fetch(new URL(`/api/join/${joinCode}/assets/${asset.id}`, baseUrl), {
      headers: { cookie: audience },
    })
    assert.equal(audienceImage.status, 200)
    assert.equal(audienceImage.headers.get('content-type'), 'image/png')
    assert.deepEqual(Buffer.from(await audienceImage.arrayBuffer()), pngBytes)

    const removed = await fetch(new URL(`/api/decks/${deckId}/assets/${asset.id}`, baseUrl), {
      method: 'DELETE',
      headers: { cookie: creatorCookie },
    })
    assert.equal(removed.status, 204)
  })

  await step('an open poll accepts a response', async () => {
    const response = await submit(audience, pollId, { option_id: 'option-a' })
    assert.equal(response.statusCode, 200, response.body)
  })

  await step('complete audience ordering is persisted and aggregated', async () => {
    const response = await submit(audience, rankingId, { ranking: ['speed', 'quality'] })
    assert.equal(response.statusCode, 200, response.body)
    const ranking = await presenterInteraction(rankingId)
    assert.deepEqual(ranking.result.ranking, ['speed', 'quality'])
    assert.equal(ranking.result.scores.speed.average_rank, 1)
  })

  await step('normalized image pins are persisted and aggregated', async () => {
    const response = await submit(audience, hotspotId, { x: 0.25, y: 0.75 })
    assert.equal(response.statusCode, 200, response.body)
    const hotspot = await presenterInteraction(hotspotId)
    assert.equal(hotspot.result.count, 1)
    assert.equal(hotspot.result.average_x, 0.25)
    assert.equal(hotspot.result.average_y, 0.75)
  })

  await step('general and slide-scoped questions follow the live slide', async () => {
    const generalResponse = await post(`/api/join/${joinCode}/questions`, { body: 'General question?', scope: 'general' }, audience)
    const slideResponse = await post(`/api/join/${joinCode}/questions`, { body: 'Slide one question?', scope: 'slide' }, audience)
    assert.equal(generalResponse.statusCode, 201, generalResponse.body)
    assert.equal(slideResponse.statusCode, 201, slideResponse.body)
    const generalId = json(generalResponse).id
    const slideId = json(slideResponse).id

    await patch(`/api/decks/${deckId}/questions/${generalId}`, { moderation_status: 'approved' }, creatorCookie)
    await patch(`/api/decks/${deckId}/questions/${slideId}`, { moderation_status: 'approved', is_pinned: true }, creatorCookie)
    let audienceQuestions = (await audienceState(audience)).questions
    assert.deepEqual(new Set(audienceQuestions.map(question => question.id)), new Set([generalId, slideId]))
    assert.equal(audienceQuestions.find(question => question.id === slideId).slide_number, 1)

    await post(`/api/decks/${deckId}/navigate`, { slide_number: 2, click_step: 0 }, creatorCookie)
    audienceQuestions = (await audienceState(audience)).questions
    assert.deepEqual(audienceQuestions.map(question => question.id), [generalId])
    const presenterQuestions = json(await request(`/api/decks/${deckId}/live-state`, { cookie: creatorCookie })).questions
    assert.ok(presenterQuestions.some(question => question.id === slideId), 'presenter retains cross-slide visibility')

    await patch(`/api/decks/${deckId}/questions/${slideId}`, { lifecycle_status: 'archived' }, creatorCookie)
    const archived = json(await request(`/api/decks/${deckId}/live-state`, { cookie: creatorCookie })).questions.find(question => question.id === slideId)
    assert.equal(archived.lifecycle_status, 'archived')
    assert.equal(archived.is_pinned, false)
    await post(`/api/decks/${deckId}/navigate`, { slide_number: 1, click_step: 0 }, creatorCookie)
  })

  await step('a mixed survey saves atomically and aggregates by question', async () => {
    const response = await submit(audience, surveyId, {
      answers: { confidence: 4, priority: 'quality', comment: 'More focus' },
    })
    assert.equal(response.statusCode, 200, response.body)
    const survey = await presenterInteraction(surveyId)
    assert.equal(survey.result.count, 1)
    assert.equal(survey.result.questions.confidence.average, 4)
    assert.equal(survey.result.questions.priority.counts.quality, 1)
    assert.deepEqual(survey.result.questions.comment.texts, ['More focus'])

    const incomplete = await submit(audience, surveyId, { answers: { confidence: 3 } })
    assert.equal(incomplete.statusCode, 400, incomplete.body)
    assert.equal((await presenterInteraction(surveyId)).result.count, 1, 'an invalid replacement must not alter the saved survey')
  })

  await step('the creator can export current responses for one result epoch', async () => {
    const denied = await request(`/api/decks/${deckId}/results.csv?epoch=1`)
    assert.equal(denied.statusCode, 401, 'audience access to the response export must be denied')

    const exported = await request(`/api/decks/${deckId}/results.csv?epoch=1`, { cookie: creatorCookie })
    assert.equal(exported.statusCode, 200, exported.body)
    assert.match(exported.headers['content-type'] || '', /^text\/csv/)
    assert.match(exported.headers['content-disposition'] || '', /attachment/)
    assert.match(exported.body, /"priority-order"/)
    assert.match(exported.body, /"focus-hotspot"/)
    assert.match(exported.body, /"team-pulse"/)
    assert.match(exported.body, /"Lock smoke"/)
  })

  await step('manual result visibility is presenter-controlled', async () => {
    const hidden = await audienceInteraction(audience, pollId)
    assert.equal(hidden.result.hidden, true)
    assert.equal(hidden.results_revealed, false)

    await control(pollId, 'reveal')
    const revealed = await audienceInteraction(audience, pollId)
    assert.equal(revealed.results_revealed, true)
    assert.equal(revealed.result.count, 1)

    await control(pollId, 'hide')
    assert.equal((await audienceInteraction(audience, pollId)).result.hidden, true)
  })

  await step('the countdown starts, pauses durably, resumes, and resets', async () => {
    await control(pollId, 'timer-reset')
    assert.equal((await presenterInteraction(pollId)).timer_remaining_seconds, 5)

    await control(pollId, 'timer-start')
    const started = await presenterInteraction(pollId)
    assert.equal(started.timer_running, true)
    assert.ok(started.timer_remaining_seconds > 0 && started.timer_remaining_seconds <= 5)

    await new Promise(resolve => setTimeout(resolve, 1100))
    const advanced = await presenterInteraction(pollId)
    assert.ok(advanced.timer_remaining_seconds < started.timer_remaining_seconds)

    await control(pollId, 'timer-pause')
    const paused = await presenterInteraction(pollId)
    assert.equal(paused.timer_running, false)
    await new Promise(resolve => setTimeout(resolve, 1100))
    assert.equal((await presenterInteraction(pollId)).timer_remaining_seconds, paused.timer_remaining_seconds)

    await control(pollId, 'timer-start')
    assert.equal((await presenterInteraction(pollId)).timer_running, true)
    await control(pollId, 'timer-reset')
    const reset = await presenterInteraction(pollId)
    assert.equal(reset.timer_running, false)
    assert.equal(reset.timer_remaining_seconds, 5)
  })

  await step('closing one interaction reports it closed to the presenter', async () => {
    const closed = await control(pollId, 'close')
    assert.equal(closed.statusCode, 200, closed.body)
    assert.equal(json(closed).accepting_responses, false)

    const poll = await presenterInteraction(pollId)
    assert.equal(poll.accepting_responses, false)
    assert.equal(poll.presenter_closed, true)
  })

  await step('a closed interaction rejects new responses', async () => {
    const response = await submit(audience, pollId, { option_id: 'option-b' })
    assert.equal(response.statusCode, 409, `expected a conflict, got ${response.statusCode}: ${response.body}`)
    assert.match(json(response).error?.message ?? response.body, /closed this interaction/i)
  })

  await step('closing one interaction leaves the others open', async () => {
    const ranked = await presenterInteraction(rankedId)
    assert.equal(ranked.accepting_responses, true)
    assert.equal(ranked.presenter_closed, false)

    const response = await submit(audience, rankedId, { votes: { quality: 1 } })
    assert.equal(response.statusCode, 200, response.body)
  })

  await step('the audience can tell a closed interaction from a global freeze', async () => {
    const frozen = await audienceInteraction(audience, rankedId)
    assert.equal(frozen.accepting_responses, true)
    assert.equal(frozen.presenter_closed, false)

    const closed = await audienceInteraction(audience, pollId)
    assert.equal(closed.accepting_responses, false)
    assert.equal(closed.presenter_closed, true)

    await post(`/api/decks/${deckId}/input-control`, { action: 'freeze' }, creatorCookie)
    const globallyFrozen = await audienceInteraction(audience, rankedId)
    assert.equal(globallyFrozen.accepting_responses, false)
    assert.equal(globallyFrozen.presenter_closed, false, 'a global freeze must not read as a per-interaction close')
    await post(`/api/decks/${deckId}/input-control`, { action: 'resume' }, creatorCookie)
  })

  await step('reopening an interaction accepts responses again', async () => {
    const reopened = await control(pollId, 'open')
    assert.equal(reopened.statusCode, 200, reopened.body)
    assert.equal(json(reopened).accepting_responses, true)

    const response = await submit(audience, pollId, { option_id: 'option-b' })
    assert.equal(response.statusCode, 200, response.body)
  })

  await step('closing a plain interaction does not rewind the slide reveal', async () => {
    await post(`/api/decks/${deckId}/navigate`, { slide_number: 1, click_step: 2 }, creatorCookie)
    await control(pollId, 'close')
    assert.equal(await clickStep(), 2, 'closing a poll must not replay the slide reveal')

    await control(rankedId, 'reset')
    assert.equal(await clickStep(), 0, 'resetting a ranked list should replay its reveal')
  })

  await step('a new result epoch reopens every closed interaction', async () => {
    await control(pollId, 'close')
    assert.equal((await presenterInteraction(pollId)).presenter_closed, true)

    const reset = await post(`/api/decks/${deckId}/reset`, {}, creatorCookie)
    assert.equal(reset.statusCode, 200, reset.body)
    assert.equal((await presenterInteraction(pollId)).presenter_closed, false)

    const response = await submit(audience, pollId, { option_id: 'option-a' })
    assert.equal(response.statusCode, 200, response.body)
  })

  await step('join codes rotate only after the live room ends', async () => {
    const blocked = await post(`/api/decks/${deckId}/join-code/rotate`, {}, creatorCookie)
    assert.equal(blocked.statusCode, 409, blocked.body)

    const stopped = await post(`/api/decks/${deckId}/stop`, {}, creatorCookie)
    assert.equal(stopped.statusCode, 200, stopped.body)
    const rotated = await post(`/api/decks/${deckId}/join-code/rotate`, {}, creatorCookie)
    assert.equal(rotated.statusCode, 200, rotated.body)
    const freshCode = json(rotated).join_code
    assert.match(freshCode, /^[2-9A-HJ-NP-Z]{3}-[2-9A-HJ-NP-Z]{3}$/)
    assert.notEqual(freshCode, joinCode)

    assert.equal((await request(`/api/join/${joinCode}/state`, { cookie: audience })).statusCode, 404)
    assert.equal((await request(`/api/join/${freshCode}/state`, { cookie: audience })).statusCode, 200)
  })

  const failed = checks.filter(check => !check.ok)
  console.log(JSON.stringify({ passed: failed.length === 0, checks }, null, 2))
  if (failed.length) process.exitCode = 1
}
finally {
  cleanupFixture()
}

async function step(name, run) {
  try {
    await run()
    checks.push({ name, ok: true })
  }
  catch (error) {
    checks.push({ name, ok: false, error: error.message })
  }
}

function seedFixture() {
  psql(`
BEGIN;
DELETE FROM decks WHERE id = '${deckId}';
DELETE FROM users WHERE id = '${userId}';

INSERT INTO users (id, google_sub, email, display_name)
VALUES ('${userId}', 'interdeck-lock-smoke', 'lock-smoke@localhost', 'Interdeck Lock Smoke');

INSERT INTO auth_sessions (token_hash, user_id, expires_at)
VALUES ('${hashToken(creatorToken)}', '${userId}', now() + interval '1 hour');

INSERT INTO decks (id, owner_id, title, join_code, markdown, revision, result_epoch)
VALUES ('${deckId}', '${userId}', 'Interaction lock smoke', '${joinCode}', $deck$<!-- interdeck-slide: lock-slide -->

# Lock smoke

---
<!-- interdeck-slide: second-slide -->

# Second slide
$deck$, 1, 1);

INSERT INTO deck_revisions (id, deck_id, revision, markdown, created_by)
VALUES ('${revisionId}', '${deckId}', 1, $deck$<!-- interdeck-slide: lock-slide -->

# Lock smoke

---
<!-- interdeck-slide: second-slide -->

# Second slide
$deck$, '${userId}');

INSERT INTO interaction_definitions (
  id, deck_id, interaction_key, slide_key, slide_number, source_order,
  kind, title, config, options, source_revision
) VALUES (
  '00000000-0000-4000-8000-00000000a001', '${deckId}', '${pollId}', 'lock-slide', 1, 0,
  'poll', 'Which option?', '{"results":"manual","timer":"5"}',
  '[{"id":"option-a","label":"Option A"},{"id":"option-b","label":"Option B"}]', 1
), (
  '00000000-0000-4000-8000-00000000a002', '${deckId}', '${rankedId}', 'lock-slide', 1, 1,
  'ranked-list', 'Rank these', '{"results":"on-close"}',
  '[{"id":"quality","label":"Quality"},{"id":"speed","label":"Speed"}]', 1
), (
  '00000000-0000-4000-8000-00000000a003', '${deckId}', '${rankingId}', 'lock-slide', 1, 2,
  'ranking', 'Order these', '{"results":"after-vote"}',
  '[{"id":"quality","label":"Quality"},{"id":"speed","label":"Speed"}]', 1
), (
  '00000000-0000-4000-8000-00000000a004', '${deckId}', '${hotspotId}', 'lock-slide', 1, 3,
  'image-hotspot', 'Place a pin', '{"image":"https://example.com/map.png","alt":"Office map","results":"after-vote"}',
  '[]', 1
), (
  '00000000-0000-4000-8000-00000000a005', '${deckId}', '${surveyId}', 'lock-slide', 1, 4,
  'survey', 'Quick pulse', '{"results":"presenter","questions":[{"id":"confidence","label":"Confidence","type":"rating","required":true,"min":1,"max":5},{"id":"priority","label":"Priority","type":"choice","required":true,"options":[{"id":"quality","label":"Quality"},{"id":"speed","label":"Speed"}]},{"id":"comment","label":"Comment","type":"text","required":false,"max":100}]}',
  '[{"id":"confidence","label":"Confidence"},{"id":"priority","label":"Priority"},{"id":"comment","label":"Comment"}]', 1
);

INSERT INTO result_epochs (deck_id, epoch, created_by, reason)
VALUES ('${deckId}', 1, '${userId}', 'lock-smoke');

INSERT INTO presentation_runs (id, deck_id, source_revision, result_epoch, status, started_by)
VALUES ('${runId}', '${deckId}', 1, 1, 'live', '${userId}');

INSERT INTO live_deck_states (deck_id, run_id, slide_key, slide_number, click_step, sequence)
VALUES ('${deckId}', '${runId}', 'lock-slide', 1, 0, 1);

-- Matches the per-interaction state that starting a presentation seeds.
INSERT INTO interaction_live_states (run_id, interaction_key, phase, accepting_responses)
SELECT '${runId}', interaction_key, 'voting', true
FROM interaction_definitions
WHERE deck_id = '${deckId}' AND is_archived = false;
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

async function join() {
  const response = await post(`/api/join/${joinCode}`, { display_name: 'Lock smoke' })
  assertStatus(response, 200, 'join the audience room')
  const cookie = (Array.isArray(response.headers['set-cookie'])
    ? response.headers['set-cookie'][0]
    : response.headers['set-cookie'])?.split(';', 1)[0]
  if (!cookie) throw new Error('join did not return an audience session cookie')
  return cookie
}

function submit(cookie, interactionId, payload) {
  return request(`/api/join/${joinCode}/interactions/${interactionId}`, {
    method: 'PUT',
    cookie,
    json: { payload, idempotency_key: randomUUID() },
  })
}

function control(interactionId, action) {
  return post(`/api/decks/${deckId}/interactions/${interactionId}/control`, { action }, creatorCookie)
}

async function presenterInteraction(interactionId) {
  const response = await request(`/api/decks/${deckId}/live-state`, { cookie: creatorCookie })
  assertStatus(response, 200, 'read the presenter live state')
  return findInteraction(json(response), interactionId)
}

async function audienceInteraction(cookie, interactionId) {
  return findInteraction(await audienceState(cookie), interactionId)
}

async function audienceState(cookie) {
  const response = await request(`/api/join/${joinCode}/state`, { cookie })
  assertStatus(response, 200, 'read the audience state')
  return json(response)
}

async function clickStep() {
  const response = await request(`/api/decks/${deckId}/live-state`, { cookie: creatorCookie })
  assertStatus(response, 200, 'read the presenter live state')
  return json(response).click_step
}

function findInteraction(state, interactionId) {
  const interaction = state.interactions?.find(item => item.id === interactionId)
  if (!interaction) throw new Error(`interaction ${interactionId} is missing from the live state`)
  return interaction
}

function post(path, body, cookie) {
  return request(path, { method: 'POST', json: body, cookie })
}

function patch(path, body, cookie) {
  return request(path, { method: 'PATCH', json: body, cookie })
}

function request(path, { method = 'GET', cookie, json: body } = {}) {
  const payload = body === undefined ? undefined : JSON.stringify(body)
  return fetch(new URL(path, baseUrl), {
    method,
    headers: {
      accept: 'application/json',
      ...(payload ? { 'content-type': 'application/json' } : {}),
      ...(cookie ? { cookie } : {}),
    },
    body: payload,
  }).then(async response => ({
    statusCode: response.status,
    headers: {
      'set-cookie': response.headers.getSetCookie?.() ?? response.headers.get('set-cookie'),
      'content-type': response.headers.get('content-type'),
      'content-disposition': response.headers.get('content-disposition'),
    },
    body: await response.text(),
  }))
}

function json(response) {
  try { return JSON.parse(response.body) }
  catch { throw new Error(`expected JSON, received: ${response.body.slice(0, 200)}`) }
}

function assertStatus(response, expected, action) {
  if (response.statusCode !== expected)
    throw new Error(`${action} returned ${response.statusCode}: ${response.body.slice(0, 200)}`)
}

function parseArguments(arguments_) {
  const values = Object.fromEntries(arguments_.map(argument => {
    const [key, ...rest] = argument.replace(/^--/, '').split('=')
    return [key, rest.join('=')]
  }))
  return {
    baseUrl: values['base-url'] || process.env.SMOKE_API_BASE || 'http://127.0.0.1:8080',
    databaseUrl: values['database-url'] || process.env.DATABASE_URL || 'postgresql://postgres:postgres@127.0.0.1:55432/interdeck',
  }
}
