import http from 'node:http'
import https from 'node:https'
import { randomUUID } from 'node:crypto'
import { performance } from 'node:perf_hooks'

const options = parseArguments(process.argv.slice(2))
if (options.questions > options.participants) {
  console.error('Question count cannot exceed participant count.')
  process.exit(2)
}
const baseUrl = new URL(options.baseUrl)
if (!['127.0.0.1', 'localhost', '::1'].includes(baseUrl.hostname) && process.env.ALLOW_REMOTE_LOAD_TEST !== '1') {
  console.error(`Refusing to load-test remote host ${baseUrl.hostname}. Set ALLOW_REMOTE_LOAD_TEST=1 only for an isolated environment.`)
  process.exit(2)
}

const transport = baseUrl.protocol === 'https:' ? https : http
const agent = new transport.Agent({
  keepAlive: true,
  maxSockets: options.participants * 2 + 100,
  maxFreeSockets: options.participants + 100,
})
const runId = randomUUID()
const joinPath = `/api/join/${encodeURIComponent(options.joinCode)}`
const interactionPath = `${joinPath}/interactions/${encodeURIComponent(options.interactionId)}`
const questionPath = `${joinPath}/questions`
const eventPath = `${joinPath}/events`

const report = {
  target: `${baseUrl.origin}${joinPath}`,
  participants: options.participants,
  join: {},
  sse: {},
  votes: {},
  idempotency: {},
  updates: {},
  persistence: {},
  questions: {},
  reconnect: {},
}

const sseClients = []

try {
  console.error(`Joining ${options.participants} participants…`)
  const joinStartedAt = performance.now()
  const participants = await mapConcurrent(
    Array.from({ length: options.participants }, (_, index) => index),
    options.joinConcurrency,
    async index => {
      const startedAt = performance.now()
      const response = await request(joinPath, {
        method: 'POST',
        json: {
          display_name: index < options.questions && index % 2 === 0
            ? `Load attendee ${String(index + 1).padStart(4, '0')}`
            : null,
        },
      })
      assertStatus(response, 200, `join participant ${index}`)
      const cookie = firstCookie(response.headers['set-cookie'])
      if (!cookie) throw new Error(`join participant ${index} did not receive a session cookie`)
      return { index, cookie, joinMs: performance.now() - startedAt }
    },
  )
  report.join = summarize(participants.map(item => item.joinMs), joinStartedAt)

  console.error(`Opening ${options.participants} persistent SSE connections…`)
  const sseStartedAt = performance.now()
  const opened = await mapConcurrent(participants, options.sseConcurrency, async participant => {
    const client = await openSseWithRetry(eventPath, participant.cookie, participant.index)
    sseClients.push(client)
    return performance.now() - sseStartedAt
  })
  report.sse.open = summarize(opened, sseStartedAt)
  report.sse.connected = sseClients.length

  console.error(`Launching ${options.participants} votes inside ${options.voteWindowMs}ms…`)
  const burstStartedAt = performance.now()
  const votes = await Promise.all(participants.map((participant, index) => new Promise(resolve => {
    const delay = Math.floor(index * options.voteWindowMs / options.participants)
    setTimeout(async () => {
      const launchedAt = performance.now()
      const idempotencyKey = `load-${runId}-${index}`
      try {
        const response = await request(interactionPath, {
          method: 'PUT',
          cookie: participant.cookie,
          json: {
            payload: { option_id: index % 2 === 0 ? 'option-a' : 'option-b' },
            idempotency_key: idempotencyKey,
          },
        })
        const body = parseJson(response.body, `vote participant ${index}`)
        resolve({
          index,
          idempotencyKey,
          status: response.statusCode,
          duplicate: body.duplicate,
          sequence: body.sequence,
          launchedAt,
          completedAt: performance.now(),
        })
      }
      catch (error) {
        resolve({ index, idempotencyKey, status: 0, error: error.message, launchedAt, completedAt: performance.now() })
      }
    }, delay)
  })))

  const successfulVotes = votes.filter(item => item.status === 200)
  const finalSequence = Math.max(0, ...successfulVotes.map(item => Number(item.sequence) || 0))
  const voteLatencies = votes.map(item => item.completedAt - item.launchedAt)
  const launchSpanMs = Math.max(...votes.map(item => item.launchedAt)) - burstStartedAt
  report.votes = {
    ...summarize(voteLatencies, burstStartedAt),
    successful: successfulVotes.length,
    errors: votes.length - successfulVotes.length,
    launch_span_ms: round(launchSpanMs),
    final_sequence: finalSequence,
    status_counts: countBy(votes, item => String(item.status)),
    error_counts: countBy(votes.filter(item => item.error), item => item.error),
    error_samples: [...new Set(votes.filter(item => item.error).map(item => item.error))].slice(0, 5),
  }

  const convergenceDeadline = performance.now() + options.convergenceTimeoutMs
  while (
    sseClients.filter(client => client.lastSequence >= finalSequence).length < sseClients.length
    && performance.now() < convergenceDeadline
  ) await wait(25)
  const converged = sseClients.filter(client => client.lastSequence >= finalSequence)
  const lastVoteCompletedAt = Math.max(...votes.map(item => item.completedAt))
  report.sse.convergence = {
    clients: converged.length,
    percent: round(converged.length * 100 / options.participants),
    p50_ms_from_burst: percentile(converged.map(client => client.lastSequenceAt - burstStartedAt), 50),
    p95_ms_from_burst: percentile(converged.map(client => client.lastSequenceAt - burstStartedAt), 95),
    p99_ms_from_burst: percentile(converged.map(client => client.lastSequenceAt - burstStartedAt), 99),
    max_ms_from_burst: round(Math.max(0, ...converged.map(client => client.lastSequenceAt - burstStartedAt))),
    p50_ms_after_last_ack: percentile(converged.map(client => Math.max(0, client.lastSequenceAt - lastVoteCompletedAt)), 50),
    p95_ms_after_last_ack: percentile(converged.map(client => Math.max(0, client.lastSequenceAt - lastVoteCompletedAt)), 95),
    p99_ms_after_last_ack: percentile(converged.map(client => Math.max(0, client.lastSequenceAt - lastVoteCompletedAt)), 99),
    max_ms_after_last_ack: round(Math.max(0, ...converged.map(client => client.lastSequenceAt - lastVoteCompletedAt))),
    events_per_client_p50: percentile(converged.map(client => client.events), 50),
    events_per_client_p95: percentile(converged.map(client => client.events), 95),
    events_per_client_max: Math.max(0, ...converged.map(client => client.events)),
    snapshots_per_client_p50: percentile(converged.map(client => client.snapshots), 50),
    snapshots_per_client_p95: percentile(converged.map(client => client.snapshots), 95),
    aggregate_converged_percent: round(converged.filter(client => client.latestResultCount === options.participants).length * 100 / options.participants),
  }

  console.error(`Replaying ${options.duplicateSamples} idempotency keys…`)
  const duplicateResults = await mapConcurrent(
    participants.slice(0, options.duplicateSamples),
    Math.min(options.duplicateSamples, 50),
    async participant => {
      const original = votes[participant.index]
      const response = await request(interactionPath, {
        method: 'PUT',
        cookie: participant.cookie,
        json: {
          payload: { option_id: participant.index % 2 === 0 ? 'option-a' : 'option-b' },
          idempotency_key: original.idempotencyKey,
        },
      })
      return { status: response.statusCode, ...parseJson(response.body, 'duplicate vote') }
    },
  )
  report.idempotency = {
    samples: duplicateResults.length,
    duplicates: duplicateResults.filter(item => item.status === 200 && item.duplicate === true).length,
  }

  console.error(`Submitting ${options.duplicateSamples} replacement votes…`)
  const updateResults = await mapConcurrent(
    participants.slice(0, options.duplicateSamples),
    Math.min(options.duplicateSamples, 50),
    async participant => {
      const response = await request(interactionPath, {
        method: 'PUT',
        cookie: participant.cookie,
        json: {
          payload: { option_id: participant.index % 2 === 0 ? 'option-b' : 'option-a' },
          idempotency_key: `update-${runId}-${participant.index}`,
        },
      })
      return { status: response.statusCode, ...parseJson(response.body, 'replacement vote') }
    },
  )
  report.updates = {
    samples: updateResults.length,
    successful: updateResults.filter(item => item.status === 200 && item.duplicate === false).length,
  }

  const stateStartedAt = performance.now()
  const stateResponse = await request(`${joinPath}/state`, { cookie: participants[0].cookie })
  assertStatus(stateResponse, 200, 'audience aggregate')
  const audienceState = parseJson(stateResponse.body, 'audience aggregate')
  const interaction = audienceState.interactions?.find(item => item.id === options.interactionId)
  report.persistence = {
    expected_voters: options.participants,
    actual_voters: interaction?.result?.count ?? null,
    read_ms: round(performance.now() - stateStartedAt),
  }

  if (options.questions > 0) {
    console.error(`Submitting ${options.questions} questions inside ${options.questionWindowMs}ms…`)
    const questionStartedAt = performance.now()
    const questionResults = await Promise.all(participants.slice(0, options.questions).map((participant, index) => new Promise(resolve => {
      const delay = Math.floor(index * options.questionWindowMs / options.questions)
      setTimeout(async () => {
        const launchedAt = performance.now()
        try {
          const response = await request(questionPath, {
            method: 'POST',
            cookie: participant.cookie,
            json: {
              body: loadQuestion(index),
              scope: index % 3 === 0 ? 'slide' : 'general',
            },
          })
          const body = parseJson(response.body, `question participant ${index}`)
          resolve({
            index,
            id: body.id,
            status: response.statusCode,
            analysisStatus: body.analysis_status,
            launchedAt,
            completedAt: performance.now(),
          })
        }
        catch (error) {
          resolve({ index, status: 0, error: error.message, launchedAt, completedAt: performance.now() })
        }
      }, delay)
    })))
    const acceptedQuestions = questionResults.filter(item => item.status === 201 && item.id)
    const questionLaunchSpanMs = Math.max(...questionResults.map(item => item.launchedAt)) - questionStartedAt
    const moderationDeadline = performance.now() + options.questionObservationMs
    while (performance.now() < moderationDeadline) {
      const resolved = acceptedQuestions.filter(item => {
        const state = sseClients[item.index]?.questions.get(item.id)
        return state && state.status !== 'pending'
      })
      if (resolved.length === acceptedQuestions.length) break
      await wait(100)
    }
    const moderationResults = acceptedQuestions.map(item => ({
      ...item,
      moderation: sseClients[item.index]?.questions.get(item.id),
    }))
    const resolvedModeration = moderationResults.filter(item => item.moderation?.status !== undefined && item.moderation.status !== 'pending')
    report.questions = {
      requested: options.questions,
      accepted: acceptedQuestions.length,
      errors: questionResults.length - acceptedQuestions.length,
      named: Math.ceil(options.questions / 2),
      anonymous: Math.floor(options.questions / 2),
      slide_scoped: Math.ceil(options.questions / 3),
      deck_wide: options.questions - Math.ceil(options.questions / 3),
      submission: {
        ...summarize(questionResults.map(item => item.completedAt - item.launchedAt), questionStartedAt),
        elapsed_ms: round(Math.max(...questionResults.map(item => item.completedAt)) - questionStartedAt),
        launch_span_ms: round(questionLaunchSpanMs),
        status_counts: countBy(questionResults, item => String(item.status)),
        error_counts: countBy(questionResults.filter(item => item.error), item => item.error),
        error_samples: [...new Set(questionResults.filter(item => item.error).map(item => item.error))].slice(0, 5),
      },
      visible_decisions: {
        resolved: resolvedModeration.length,
        approved: moderationResults.filter(item => item.moderation?.status === 'approved').length,
        rejected: moderationResults.filter(item => item.moderation?.status === 'rejected').length,
        pending: moderationResults.filter(item => !item.moderation || item.moderation.status === 'pending').length,
        elapsed_ms: round(performance.now() - questionStartedAt),
        p50_ms_from_submit: percentile(resolvedModeration.map(item => item.moderation.seenAt - item.completedAt), 50),
        p95_ms_from_submit: percentile(resolvedModeration.map(item => item.moderation.seenAt - item.completedAt), 95),
        p99_ms_from_submit: percentile(resolvedModeration.map(item => item.moderation.seenAt - item.completedAt), 99),
        max_ms_from_submit: round(Math.max(0, ...resolvedModeration.map(item => item.moderation.seenAt - item.completedAt))),
      },
    }
  }

  if (options.holdBeforeReconnectMs > 0) {
    console.error(`Holding ${options.holdBeforeReconnectMs}ms before reconnect for external failure injection…`)
    await wait(options.holdBeforeReconnectMs)
  }
  console.error(`Reconnecting ${options.participants} SSE clients…`)
  sseClients.splice(0).forEach(client => client.close())
  const reconnectStartedAt = performance.now()
  const reconnectClients = await mapConcurrent(participants, options.sseConcurrency, async participant => (
    openSseWithRetry(eventPath, participant.cookie, participant.index)
  ))
  sseClients.push(...reconnectClients)
  report.reconnect = {
    connected: reconnectClients.length,
    elapsed_ms: round(performance.now() - reconnectStartedAt),
  }

  const passed = (
    participants.length === options.participants
    && sseClients.length === options.participants
    && successfulVotes.length === options.participants
    && launchSpanMs <= options.voteWindowMs + 250
    && report.sse.convergence.percent >= 99
    && report.sse.convergence.aggregate_converged_percent >= 99
    && report.sse.convergence.snapshots_per_client_p50 >= 1
    && report.sse.convergence.events_per_client_p95 <= 100
    && report.idempotency.duplicates === report.idempotency.samples
    && report.updates.successful === report.updates.samples
    && report.persistence.actual_voters === report.persistence.expected_voters
    && (options.questions === 0 || (
      report.questions.accepted === options.questions
    ))
  )
  report.passed = passed
  console.log(JSON.stringify(report, null, 2))
  if (!passed) process.exitCode = 1
}
finally {
  sseClients.forEach(client => client.close())
  agent.destroy()
}

function request(path, { method = 'GET', cookie, json } = {}) {
  const body = json === undefined ? undefined : JSON.stringify(json)
  return new Promise((resolve, reject) => {
    const request = transport.request(new URL(path, baseUrl), {
      agent,
      method,
      headers: {
        accept: 'application/json',
        ...(body ? { 'content-type': 'application/json', 'content-length': Buffer.byteLength(body) } : {}),
        ...(cookie ? { cookie } : {}),
      },
    }, response => {
      const chunks = []
      response.on('data', chunk => chunks.push(chunk))
      response.on('end', () => resolve({
        statusCode: response.statusCode,
        headers: response.headers,
        body: Buffer.concat(chunks).toString('utf8'),
      }))
    })
    request.setTimeout(options.requestTimeoutMs, () => request.destroy(new Error(`request timed out after ${options.requestTimeoutMs}ms`)))
    request.on('error', reject)
    if (body) request.write(body)
    request.end()
  })
}

function openSse(path, cookie, index) {
  return new Promise((resolve, reject) => {
    let settled = false
    const request = transport.request(new URL(path, baseUrl), {
      agent,
      headers: { accept: 'text/event-stream', cookie },
    })
    const timer = setTimeout(() => {
      request.destroy()
      if (!settled) reject(new Error(`SSE participant ${index} timed out while connecting`))
    }, options.requestTimeoutMs)
    request.on('response', response => {
      if (response.statusCode !== 200) {
        clearTimeout(timer)
        response.resume()
        return reject(new Error(`SSE participant ${index} returned ${response.statusCode}`))
      }
      clearTimeout(timer)
      settled = true
      const client = {
        index,
        lastSequence: 0,
        lastSequenceAt: 0,
        events: 0,
        snapshots: 0,
        latestResultCount: 0,
        questions: new Map(),
        close() {
          response.destroy()
          request.destroy()
        },
      }
      let buffer = ''
      response.setEncoding('utf8')
      response.on('data', chunk => {
        buffer += chunk.replaceAll('\r\n', '\n')
        let boundary
        while ((boundary = buffer.indexOf('\n\n')) >= 0) {
          const block = buffer.slice(0, boundary)
          buffer = buffer.slice(boundary + 2)
          const event = parseSseBlock(block)
          if (event.event !== 'state') continue
          let sequence = Number(event.data)
          try {
            const envelope = JSON.parse(event.data)
            if (envelope?.type === 'snapshot') {
              sequence = Number(envelope.sequence)
              client.snapshots += 1
              const interaction = envelope.state?.interactions?.find(item => item.id === options.interactionId)
              client.latestResultCount = Number(interaction?.result?.count || 0)
              for (const question of envelope.state?.questions || []) {
                const existing = client.questions.get(question.id)
                if (existing?.status === question.moderation_status) continue
                client.questions.set(question.id, { status: question.moderation_status, seenAt: performance.now() })
              }
            }
          }
          catch {}
          if (!Number.isFinite(sequence)) continue
          client.events += 1
          if (sequence >= client.lastSequence) {
            client.lastSequence = sequence
            client.lastSequenceAt = performance.now()
          }
        }
      })
      response.on('error', () => {})
      resolve(client)
    })
    request.on('error', error => {
      clearTimeout(timer)
      if (!settled) reject(error)
    })
    request.end()
  })
}

async function openSseWithRetry(path, cookie, index) {
  let lastError
  for (let attempt = 1; attempt <= options.sseConnectAttempts; attempt += 1) {
    try {
      return await openSse(path, cookie, index)
    }
    catch (error) {
      lastError = error
      if (attempt < options.sseConnectAttempts) await wait(attempt * 250)
    }
  }
  throw lastError
}

function parseSseBlock(block) {
  const event = { event: 'message', data: '' }
  for (const line of block.split('\n')) {
    if (line.startsWith('event:')) event.event = line.slice(6).trim()
    if (line.startsWith('data:')) event.data += line.slice(5).trim()
  }
  return event
}

async function mapConcurrent(items, concurrency, operation) {
  const results = new Array(items.length)
  let cursor = 0
  async function worker() {
    while (cursor < items.length) {
      const index = cursor++
      results[index] = await operation(items[index], index)
    }
  }
  await Promise.all(Array.from({ length: Math.min(concurrency, items.length) }, worker))
  return results
}

function summarize(values, startedAt) {
  return {
    completed: values.length,
    elapsed_ms: round(performance.now() - startedAt),
    p50_ms: percentile(values, 50),
    p95_ms: percentile(values, 95),
    p99_ms: percentile(values, 99),
    max_ms: round(Math.max(0, ...values)),
  }
}

function percentile(values, percent) {
  if (!values.length) return null
  const sorted = [...values].sort((a, b) => a - b)
  return round(sorted[Math.min(sorted.length - 1, Math.ceil(percent / 100 * sorted.length) - 1)])
}

function countBy(items, key) {
  return items.reduce((counts, item) => {
    const value = key(item)
    counts[value] = (counts[value] || 0) + 1
    return counts
  }, {})
}

function firstCookie(header) {
  const value = Array.isArray(header) ? header[0] : header
  return value?.split(';', 1)[0]
}

function assertStatus(response, expected, action) {
  if (response.statusCode !== expected)
    throw new Error(`${action} returned ${response.statusCode}: ${response.body.slice(0, 200)}`)
}

function parseJson(body, action) {
  try { return JSON.parse(body) }
  catch { throw new Error(`${action} returned invalid JSON: ${body.slice(0, 200)}`) }
}

function round(value) {
  return Math.round(value * 10) / 10
}

function wait(ms) {
  return new Promise(resolve => setTimeout(resolve, ms))
}

function parseArguments(arguments_) {
  const values = Object.fromEntries(arguments_.map(argument => {
    const [key, ...rest] = argument.replace(/^--/, '').split('=')
    return [key, rest.join('=')]
  }))
  return {
    baseUrl: values['base-url'] || 'http://127.0.0.1:3000',
    joinCode: (values['join-code'] || 'LOD-TST').toUpperCase(),
    interactionId: values['interaction-id'] || 'load-poll',
    participants: positiveInteger(values.participants, 1000),
    joinConcurrency: positiveInteger(values['join-concurrency'], 100),
    sseConcurrency: positiveInteger(values['sse-concurrency'], 250),
    voteWindowMs: positiveInteger(values['vote-window-ms'], 2000),
    convergenceTimeoutMs: positiveInteger(values['convergence-timeout-ms'], 10000),
    requestTimeoutMs: positiveInteger(values['request-timeout-ms'], 30000),
    duplicateSamples: positiveInteger(values['duplicate-samples'], 50),
    questions: nonNegativeInteger(values.questions, 0),
    questionWindowMs: positiveInteger(values['question-window-ms'], 5000),
    questionObservationMs: positiveInteger(values['question-observation-ms'], 30000),
    sseConnectAttempts: positiveInteger(values['sse-connect-attempts'], 4),
    holdBeforeReconnectMs: nonNegativeInteger(values['hold-before-reconnect-ms'], 0),
  }
}

function loadQuestion(index) {
  const questions = [
    'What is the most important company priority for the next quarter?',
    'How will we measure whether the current strategy is working?',
    'What should teams stop doing to create more focus?',
    'Where do you see the biggest opportunity to improve customer outcomes?',
    'How are we investing in developer experience this year?',
    'What is one lesson leadership learned during the last quarter?',
    'How can employees contribute to the next phase of the strategy?',
    'Which operational risk deserves more attention right now?',
    'How will the company protect time for learning and development?',
    'What evidence would cause us to change the current plan?',
    'How should teams balance delivery speed with product quality?',
    'What customer feedback has had the greatest influence on our roadmap?',
    'Where can teams collaborate more effectively across departments?',
    'What is the clearest sign that the organization is making progress?',
    'How are priorities communicated when circumstances change?',
    'What can managers do to make decision making faster?',
    'Which capability should we build internally rather than outsource?',
    'How will we keep the strategy understandable as the company grows?',
    'What experiment would you most like the company to run next?',
    'What will success look like by the next all-hands meeting?',
  ]
  return `${questions[index % questions.length]} (load scenario ${index + 1})`
}

function positiveInteger(value, fallback) {
  const parsed = Number.parseInt(value, 10)
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback
}

function nonNegativeInteger(value, fallback) {
  const parsed = Number.parseInt(value, 10)
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : fallback
}
