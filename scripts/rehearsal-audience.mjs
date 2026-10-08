import http from 'node:http'
import https from 'node:https'
import { randomUUID } from 'node:crypto'
import { lookup as resolveHost } from 'node:dns/promises'
import { performance } from 'node:perf_hooks'

const options = parseArguments(process.argv.slice(2))
const baseUrl = new URL(options.baseUrl)
if (!['127.0.0.1', 'localhost', '::1'].includes(baseUrl.hostname) && process.env.ALLOW_REMOTE_REHEARSAL !== '1') {
  console.error(`Refusing to simulate an audience on remote host ${baseUrl.hostname}. Set ALLOW_REMOTE_REHEARSAL=1 only for an isolated Test presentation.`)
  process.exit(2)
}
if (!options.joinCode) {
  console.error('A join code is required: --join-code=ABC-123')
  process.exit(2)
}

const transport = baseUrl.protocol === 'https:' ? https : http
// Opening 1,000 sockets at once must not turn into 1,000 DNS requests. macOS's
// resolver can transiently return ENOTFOUND under that fan-out even after the
// join requests succeeded, so resolve once and let the connection pool reuse it.
const resolvedHost = await resolveHost(baseUrl.hostname)
const agent = new transport.Agent({
  keepAlive: true,
  maxSockets: options.participants * 2 + 100,
  maxFreeSockets: options.participants + 100,
  lookup(hostname, lookupOptions, callback) {
    if (hostname !== baseUrl.hostname) return callback(new Error(`Unexpected rehearsal host ${hostname}`))
    if (lookupOptions?.all) return callback(null, [resolvedHost])
    callback(null, resolvedHost.address, resolvedHost.family)
  },
})
const joinPath = `/api/join/${encodeURIComponent(options.joinCode)}`
const eventPath = `${joinPath}/events`
const report = {
  target: `${baseUrl.origin}${joinPath}`,
  participants: options.participants,
  joined: 0,
  streams: 0,
  dropped_streams: 0,
  reconnected_streams: 0,
  failed_reconnects: 0,
  interactions: {},
  questions: {},
  started_at: new Date().toISOString(),
}
const streams = []
const responseTasks = new Set()
const scheduledInteractions = new Set()
let running = true
let presentationSeen = false

process.once('SIGINT', stop)
process.once('SIGTERM', stop)

try {
  const preflight = await audienceState()
  if (!preflight.live || preflight.run_mode !== 'rehearsal') {
    throw new Error('The room is not running an isolated Test presentation. Start Test presentation from deck settings before launching the simulator.')
  }
  presentationSeen = true

  console.error(`Joining ${options.participants} synthetic rehearsal attendees…`)
  const participants = await mapConcurrent(
    Array.from({ length: options.participants }, (_, index) => index),
    options.joinConcurrency,
    async index => {
      const response = await request(joinPath, {
        method: 'POST',
        json: {
          display_name: index % 4 === 0 ? `Rehearsal attendee ${String(index + 1).padStart(4, '0')}` : null,
        },
      })
      assertStatus(response, 200, `join attendee ${index}`)
      const cookie = firstCookie(response.headers['set-cookie'])
      if (!cookie) throw new Error(`join attendee ${index} did not receive a session cookie`)
      return { index, cookie }
    },
  )
  report.joined = participants.length

  console.error(`Opening ${options.participants} persistent audience connections…`)
  const opened = await mapConcurrent(participants, options.sseConcurrency, async participant => {
    const stream = await openSse(eventPath, participant.cookie, participant.index)
    streams.push(stream)
    return stream
  })
  report.streams = opened.length
  console.error('Audience ready. Present normally; responses will arrive as each interaction becomes available.')

  if (options.questions > 0) scheduleQuestions(participants)

  while (running) {
    const state = await audienceState(participants[0].cookie)
    if (!state.live) {
      if (presentationSeen) break
      await wait(options.pollIntervalMs)
      continue
    }
    presentationSeen = true
    if (state.run_mode !== 'rehearsal') {
      throw new Error('The room changed out of Test presentation mode. Stopping before live results can be affected.')
    }
    for (const interaction of state.interactions || []) {
      if (!interaction.accepting_responses) continue
      if (interaction.kind === 'ranked-list' && interaction.phase !== 'voting') continue
      if (scheduledInteractions.has(interaction.id)) continue
      const example = buildPayload(interaction, 0)
      if (!example) {
        scheduledInteractions.add(interaction.id)
        console.error(`Skipping unsupported non-response component ${interaction.id} (${interaction.kind}).`)
        continue
      }
      scheduledInteractions.add(interaction.id)
      const task = simulateInteraction(participants, interaction)
        .finally(() => responseTasks.delete(task))
      responseTasks.add(task)
    }
    await wait(options.pollIntervalMs)
  }

  console.error('Presentation ended. Waiting for scheduled audience actions to settle…')
  await Promise.allSettled(responseTasks)
}
catch (error) {
  report.error = error instanceof Error ? error.message : String(error)
  process.exitCode = 1
}
finally {
  running = false
  await Promise.allSettled(responseTasks)
  streams.forEach(stream => stream.close())
  agent.destroy()
  report.finished_at = new Date().toISOString()
  console.log(JSON.stringify(report, null, 2))
}

function stop() {
  running = false
}

async function audienceState(cookie) {
  let lastError
  for (let attempt = 1; attempt <= 5; attempt += 1) {
    try {
      const response = await request(`${joinPath}/state`, { cookie })
      assertStatus(response, 200, 'audience state')
      return parseJson(response.body, 'audience state')
    }
    catch (error) {
      lastError = error
      if (attempt < 5) await wait(attempt * 250)
    }
  }
  throw lastError
}

function simulateInteraction(participants, interaction) {
  console.error(`Responding to ${interaction.id} (${interaction.kind}) over ${options.responseWindowMs}ms…`)
  const startedAt = performance.now()
  return Promise.all(participants.map((participant, index) => new Promise(resolve => {
    const delay = Math.floor(index * options.responseWindowMs / participants.length)
    setTimeout(async () => {
      const launchedAt = performance.now()
      const payload = buildPayload(interaction, index)
      if (!payload) return resolve({ status: 0, latency: 0, skipped: true })
      try {
        const response = await retryIdempotentRequest(`${joinPath}/interactions/${encodeURIComponent(interaction.id)}`, {
          method: 'PUT',
          cookie: participant.cookie,
          json: { payload, idempotency_key: `rehearsal-${interaction.id}-${participant.index}-${randomUUID()}` },
        })
        resolve({ status: response.statusCode, latency: performance.now() - launchedAt })
      }
      catch (error) {
        resolve({ status: 0, latency: performance.now() - launchedAt, error: error.message })
      }
    }, delay)
  }))).then(results => {
    const latencies = results.filter(item => item.status === 200).map(item => item.latency)
    report.interactions[interaction.id] = {
      kind: interaction.kind,
      attempted: results.length,
      saved: results.filter(item => item.status === 200).length,
      unavailable: results.filter(item => item.status === 409).length,
      rate_limited: results.filter(item => item.status === 429).length,
      errors: results.filter(item => ![200, 409, 429].includes(item.status)).length,
      p50_ms: percentile(latencies, 50),
      p95_ms: percentile(latencies, 95),
      p99_ms: percentile(latencies, 99),
      elapsed_ms: round(performance.now() - startedAt),
    }
    console.error(`${interaction.id}: ${report.interactions[interaction.id].saved}/${results.length} saved, p95 ${report.interactions[interaction.id].p95_ms}ms.`)
  })
}

function scheduleQuestions(participants) {
  const count = Math.min(options.questions, participants.length)
  const task = Promise.all(participants.slice(0, count).map((participant, index) => new Promise(resolve => {
    const delay = options.questionDelayMs + Math.floor(index * options.questionWindowMs / count)
    setTimeout(async () => {
      const launchedAt = performance.now()
      try {
        const response = await request(`${joinPath}/questions`, {
          method: 'POST',
          cookie: participant.cookie,
          json: {
            body: rehearsalQuestion(index),
            scope: index % 3 === 0 ? 'slide' : 'general',
          },
        })
        resolve({ status: response.statusCode, latency: performance.now() - launchedAt })
      }
      catch (error) {
        resolve({ status: 0, latency: performance.now() - launchedAt, error: error.message })
      }
    }, delay)
  }))).then(results => {
    const latencies = results.filter(item => item.status === 201).map(item => item.latency)
    report.questions = {
      attempted: count,
      accepted: results.filter(item => item.status === 201).length,
      errors: results.filter(item => item.status !== 201).length,
      p95_ms: percentile(latencies, 95),
    }
  })
  responseTasks.add(task)
  task.finally(() => responseTasks.delete(task))
}

function buildPayload(interaction, index) {
  const optionsList = interaction.options || []
  const config = interaction.config || {}
  const option = optionsList[index % Math.max(1, optionsList.length)]
  switch (interaction.kind) {
    case 'poll': {
      const multiple = config.multiple === 'true' || config.max != null
      if (!multiple) return option ? { option_id: option.id } : null
      const max = Math.max(1, Math.min(Number(config.max || 2), optionsList.length))
      return { option_ids: rotate(optionsList.map(item => item.id), index).slice(0, max) }
    }
    case 'quiz':
    case 'reaction':
    case 'image-choice':
      return option ? { option_id: option.id } : null
    case 'vote':
    case 'updown':
      return { value: index % 4 === 0 ? -1 : 1 }
    case 'rating': {
      const min = numberConfig(config, 'min', 1)
      const max = numberConfig(config, 'max', 5)
      return { value: min + (index % (Math.floor(max - min) + 1)) }
    }
    case 'number': {
      const min = numberConfig(config, 'min', 0)
      const max = numberConfig(config, 'max', 100)
      return { value: min + ((index % 101) / 100) * (max - min) }
    }
    case 'allocation': {
      if (!optionsList.length) return null
      const total = Math.trunc(numberConfig(config, 'total', 100))
      const base = Math.floor(total / optionsList.length)
      let remainder = total - base * optionsList.length
      const ids = rotate(optionsList.map(item => item.id), index)
      return { allocations: Object.fromEntries(ids.map(id => [id, base + (remainder-- > 0 ? 1 : 0)])) }
    }
    case 'matrix': {
      const xMin = numberConfig(config, 'x-min', 0)
      const xMax = numberConfig(config, 'x-max', 10)
      const yMin = numberConfig(config, 'y-min', 0)
      const yMax = numberConfig(config, 'y-max', 10)
      return { x: xMin + ((index * 37) % 101) / 100 * (xMax - xMin), y: yMin + ((index * 61) % 101) / 100 * (yMax - yMin) }
    }
    case 'ranking':
      return optionsList.length ? { ranking: rotate(optionsList.map(item => item.id), index) } : null
    case 'ranked-list': {
      const revealed = Math.max(0, Number(interaction.revealed_count || optionsList.length))
      const ids = optionsList.slice(0, revealed).map(item => item.id)
      return ids.length ? { votes: Object.fromEntries(ids.map((id, voteIndex) => [id, (index + voteIndex) % 5 === 0 ? -1 : 1])) } : null
    }
    case 'image-hotspot':
      return { x: ((index * 37) % 101) / 100, y: ((index * 61) % 101) / 100 }
    case 'word-cloud': {
      const words = ['focused', 'hopeful', 'curious', 'energized', 'confident', 'ambitious', 'connected', 'thoughtful', 'optimistic', 'ready']
      return { texts: [words[index % words.length]] }
    }
    case 'free-text':
      return { text: `Rehearsal response ${index + 1}` }
    case 'survey':
      return { answers: buildSurveyAnswers(config.questions || [], index) }
    default:
      return null
  }
}

function buildSurveyAnswers(questions, index) {
  return Object.fromEntries(questions.map(question => {
    if (question.type === 'choice') {
      const choice = question.options?.[index % Math.max(1, question.options?.length || 0)]
      return [question.id, choice?.id || '']
    }
    if (question.type === 'rating') {
      const min = Number(question.min ?? 1)
      const max = Number(question.max ?? 5)
      return [question.id, min + (index % (max - min + 1))]
    }
    return [question.id, `Rehearsal answer ${index + 1}`]
  }))
}

function rehearsalQuestion(index) {
  const questions = [
    'What is the most important priority after this all-hands?',
    'How will we measure progress next quarter?',
    'What should teams stop doing to create more focus?',
    'Where is our biggest opportunity to improve customer outcomes?',
    'How can employees contribute to the strategy?',
    'Which customer problem deserves the most attention this quarter?',
    'What is the biggest risk to achieving the plan?',
    'How will leadership communicate progress between all-hands meetings?',
    'Which capability should we invest in developing internally?',
    'What trade-off was hardest when setting these priorities?',
    'How will this strategy affect day-to-day decisions for delivery teams?',
    'What evidence would cause us to change direction?',
    'Where can teams collaborate more effectively across departments?',
    'Which initiative is most dependent on customer feedback?',
    'How are we balancing short-term delivery with long-term investment?',
    'What does success look like for employees by the next all-hands?',
    'Which assumption in the plan has the greatest uncertainty?',
    'How can managers help their teams connect their work to the strategy?',
    'What is one action everyone can take immediately after this session?',
    'Where should we expect the first visible signs of progress?',
  ]
  return `${questions[index % questions.length]} (rehearsal ${index + 1})`
}

function openSse(path, cookie, index) {
  const stream = {
    closed: false,
    request: null,
    response: null,
    reconnectTimer: null,
    close() {
      this.closed = true
      if (this.reconnectTimer) clearTimeout(this.reconnectTimer)
      this.response?.destroy()
      this.request?.destroy()
    },
  }
  return connectSse(stream, path, cookie, index).then(() => stream)
}

function connectSse(stream, path, cookie, index) {
  return new Promise((resolve, reject) => {
    let settled = false
    let disconnected = false
    const request_ = transport.request(new URL(path, baseUrl), { agent, headers: { accept: 'text/event-stream', cookie } })
    stream.request = request_
    const timer = setTimeout(() => {
      request_.destroy()
      if (!settled) reject(new Error(`SSE attendee ${index} timed out`))
    }, options.requestTimeoutMs)
    request_.on('response', response => {
      if (response.statusCode !== 200) {
        clearTimeout(timer)
        response.resume()
        return reject(new Error(`SSE attendee ${index} returned ${response.statusCode}`))
      }
      clearTimeout(timer)
      settled = true
      stream.response = response
      response.resume()
      const reconnect = () => {
        if (disconnected || stream.closed || !running) return
        disconnected = true
        report.dropped_streams += 1
        scheduleSseReconnect(stream, path, cookie, index, 1)
      }
      response.once('close', reconnect)
      response.once('error', reconnect)
      resolve()
    })
    request_.once('error', error => {
      clearTimeout(timer)
      if (!settled) reject(error)
    })
    request_.end()
  })
}

function scheduleSseReconnect(stream, path, cookie, index, attempt) {
  if (stream.closed || !running) return
  stream.reconnectTimer = setTimeout(async () => {
    try {
      await connectSse(stream, path, cookie, index)
      report.reconnected_streams += 1
    }
    catch {
      report.failed_reconnects += 1
      scheduleSseReconnect(stream, path, cookie, index, Math.min(attempt + 1, 8))
    }
  }, Math.min(5000, 250 * 2 ** (attempt - 1)))
}

async function retryIdempotentRequest(path, requestOptions) {
  let lastError
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    try { return await request(path, requestOptions) }
    catch (error) {
      lastError = error
      if (attempt < 3) await wait(attempt * 150)
    }
  }
  throw lastError
}

function request(path, { method = 'GET', cookie, json } = {}) {
  const body = json === undefined ? undefined : JSON.stringify(json)
  return new Promise((resolve, reject) => {
    const request_ = transport.request(new URL(path, baseUrl), {
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
      response.on('end', () => resolve({ statusCode: response.statusCode, headers: response.headers, body: Buffer.concat(chunks).toString('utf8') }))
    })
    request_.setTimeout(options.requestTimeoutMs, () => request_.destroy(new Error(`request timed out after ${options.requestTimeoutMs}ms`)))
    request_.once('error', reject)
    if (body) request_.write(body)
    request_.end()
  })
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

function parseArguments(arguments_) {
  const values = Object.fromEntries(arguments_.map(argument => {
    const [key, ...rest] = argument.replace(/^--/, '').split('=')
    return [key, rest.join('=')]
  }))
  return {
    baseUrl: values['base-url'] || 'http://127.0.0.1:3000',
    joinCode: (values['join-code'] || '').toUpperCase(),
    participants: positiveInteger(values.participants, 1000),
    joinConcurrency: positiveInteger(values['join-concurrency'], 100),
    sseConcurrency: positiveInteger(values['sse-concurrency'], 250),
    responseWindowMs: positiveInteger(values['response-window-ms'], 8000),
    pollIntervalMs: positiveInteger(values['poll-interval-ms'], 500),
    requestTimeoutMs: positiveInteger(values['request-timeout-ms'], 30000),
    questions: nonNegativeInteger(values.questions, 20),
    questionDelayMs: nonNegativeInteger(values['question-delay-ms'], 15000),
    questionWindowMs: positiveInteger(values['question-window-ms'], 60000),
  }
}

function firstCookie(value) {
  const header = Array.isArray(value) ? value[0] : value
  return header?.split(';', 1)[0] || ''
}

function assertStatus(response, expected, action) {
  if (response.statusCode !== expected) throw new Error(`${action} returned ${response.statusCode}: ${response.body.slice(0, 200)}`)
}

function parseJson(body, action) {
  try { return JSON.parse(body) }
  catch { throw new Error(`${action} returned invalid JSON: ${body.slice(0, 200)}`) }
}

function rotate(items, index) {
  if (!items.length) return []
  const offset = index % items.length
  return [...items.slice(offset), ...items.slice(0, offset)]
}

function numberConfig(config, key, fallback) {
  const value = Number(config[key])
  return Number.isFinite(value) ? value : fallback
}

function percentile(values, percentage) {
  if (!values.length) return null
  const ordered = [...values].sort((left, right) => left - right)
  return round(ordered[Math.min(ordered.length - 1, Math.ceil(ordered.length * percentage / 100) - 1)])
}

function positiveInteger(value, fallback) {
  const parsed = Number.parseInt(value, 10)
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback
}

function nonNegativeInteger(value, fallback) {
  const parsed = Number.parseInt(value, 10)
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : fallback
}

function round(value) {
  return Math.round(value * 10) / 10
}

function wait(ms) {
  return new Promise(resolve => setTimeout(resolve, ms))
}
