import { createHmac, timingSafeEqual } from 'node:crypto'
import { existsSync } from 'node:fs'
import { createServer } from 'node:http'
import { fileURLToPath } from 'node:url'
import { dirname, join, resolve } from 'node:path'

import dotenv from 'dotenv'
import express from 'express'
import httpProxy from 'http-proxy'
import { parseSync } from '@slidev/parser'

import { slidevCatalog, validateSlidevCompatibility } from './slidev-catalog.mjs'
import { SlidevManager } from './slidev-manager.mjs'
import { runtimeConfig } from './runtime-config.mjs'
import { transformMarkdown } from './transform.mjs'

const here = dirname(fileURLToPath(import.meta.url))
dotenv.config({ path: resolve(here, '../../../.env') })

const port = Number(process.env.PORT || 3000)
const apiBase = (process.env.INTERNAL_API_BASE || 'http://127.0.0.1:8080').replace(/\/$/, '')
const { appEnv, internalToken, slidevSecret } = runtimeConfig(process.env)
const webDist = resolve(here, process.env.WEB_DIST_DIR || '../../../apps/web/dist')
const internalFetchTimeoutMs = Math.max(1_000, Number(process.env.INTERNAL_FETCH_TIMEOUT_MS || 10_000))

const app = express()
const server = createServer(app)
app.disable('x-powered-by')
app.use((_request, response, next) => {
  response.setHeader('X-Content-Type-Options', 'nosniff')
  response.setHeader('Referrer-Policy', 'same-origin')
  response.setHeader('Permissions-Policy', 'camera=(), microphone=(), geolocation=()')
  response.setHeader('X-Frame-Options', 'SAMEORIGIN')
  response.setHeader('Content-Security-Policy', "frame-ancestors 'self'; base-uri 'self'")
  if (appEnv === 'production')
    response.setHeader('Strict-Transport-Security', 'max-age=31536000; includeSubDomains')
  next()
})
// Upstream services are private implementation details. Rewrite the Host header
// to the selected upstream so Vite validates `localhost`, while `xfwd` preserves
// the public host for any application code that needs the original request.
const proxy = httpProxy.createProxyServer({ changeOrigin: true, ws: true, xfwd: true })
const slidev = new SlidevManager({
  apiBase,
  internalToken,
  idleTtlSeconds: process.env.SLIDEV_IDLE_TTL_SECONDS || 600,
  maxServers: process.env.SLIDEV_MAX_SERVERS || 4,
})

proxy.on('error', (error, request, response) => {
  console.error('Proxy error', error)
  if (response && 'writeHead' in response && !response.headersSent) {
    response.writeHead(502, { 'content-type': 'application/json' })
    response.end(JSON.stringify({ error: { code: 'proxy_error', message: 'The upstream service is unavailable' } }))
  }
  else if (response && 'destroy' in response) {
    response.destroy()
  }
})

app.get('/_gateway/health', (_request, response) => {
  response.json({
    status: 'ok',
    renderer: 'universal',
    active_slidev_servers: slidev.servers.size,
    starting_slidev_servers: slidev.starting.size,
    slidev_server_limit: slidev.maxServers,
  })
})

app.get('/_gateway/slidev/catalog', (_request, response) => {
  response.json({
    slidev_version: slidevCatalog.slidevVersion,
    themes: slidevCatalog.themes.map(({ id, label, description }) => ({ id, label, description })),
    addons: slidevCatalog.addons.map(({ id, label, description }) => ({ id, label, description })),
    core_features: slidevCatalog.coreFeatures,
    policy: 'Themes and addons must be reviewed, pinned, and installed in the Interdeck worker image.',
  })
})

app.post('/_gateway/slidev/validate-source', express.json({ limit: '2mb' }), (request, response) => {
  if (!hasInternalAccess(request))
    return response.status(401).json({ error: { code: 'unauthorized', message: 'Internal access is required' } })

  const deckId = String(request.body?.deck_id || '').toLowerCase()
  const markdown = request.body?.markdown
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(deckId))
    return response.status(400).json({ error: { code: 'invalid_deck_id', message: 'A valid deck ID is required' } })
  if (typeof markdown !== 'string' || markdown.length > 2_000_000)
    return response.status(400).json({ error: { code: 'invalid_markdown', message: 'Valid deck Markdown is required' } })

  try {
    const compatibility = validateSlidevCompatibility(markdown)
    const transformed = transformMarkdown(markdown, {
      deckId,
      joinUrl: 'https://interdeck.invalid/j/VALIDATE',
    })
    const parsed = parseSync(transformed, 'slides.md')
    response.json({ ok: true, slide_count: parsed.slides.length, theme: compatibility.theme })
  }
  catch (error) {
    response.status(error?.statusCode || 422).json({
      error: {
        code: error?.code || 'invalid_slidev_markdown',
        message: error?.publicMessage || 'Slidev could not parse the proposed deck Markdown.',
      },
    })
  }
})

app.post('/_gateway/slidev/preflight', express.json({ limit: '16kb' }), async (request, response) => {
  const startedAt = Date.now()
  let buildContext
  let compileSucceeded = false
  try {
    const deckId = String(request.body?.deck_id || '').toLowerCase()
    if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(deckId))
      return response.status(400).json({ error: { code: 'invalid_deck_id', message: 'A valid deck ID is required' } })

    const payload = verifySlidevToken(request.body?.access, { deckId, mode: 'preview' }, slidevSecret)
    buildContext = { deckId, version: payload.version }
    const source = await fetchDeckSource(deckId, payload.version)
    const compatibility = validateSlidevCompatibility(source.markdown)
    const transformed = transformMarkdown(source.markdown, {
      deckId,
      joinUrl: source.join_url,
    })
    const parsed = parseSync(transformed, 'slides.md')
    if (!parsed.slides.length) {
      const error = new Error('The deck does not contain any slides')
      error.statusCode = 422
      error.code = 'empty_deck'
      error.publicMessage = 'Add at least one slide before previewing this deck.'
      throw error
    }
    compileSucceeded = true
    const startupMs = Date.now() - startedAt
    await recordDeckBuild({
      deckId,
      sourceVersion: payload.version,
      status: 'succeeded',
      slidevVersion: slidevCatalog.slidevVersion,
      theme: compatibility?.theme || 'default',
      startupMs,
    })
    response.json({
      ok: true,
      version: payload.version,
      slidev_version: slidevCatalog.slidevVersion,
      theme: compatibility?.theme || 'default',
      renderer: 'universal',
      startup_ms: startupMs,
    })
  }
  catch (error) {
    if (error.code === 'slidev_version_superseded') {
      return response.json({
        ok: false,
        superseded: true,
        version: error.latestVersion || buildContext?.version,
      })
    }
    if (error.code === 'slidev_restart_required') {
      return response.json({
        ok: false,
        restart_required: true,
        version: buildContext?.version,
      })
    }
    console.error('Slidev preflight failed', error)
    const shouldRecordFailure = buildContext
      && !compileSucceeded
      && error.statusCode !== 503
      && !['slidev_version_superseded', 'slidev_restart_required'].includes(error.code)
    if (shouldRecordFailure) {
      try {
        await recordDeckBuild({
          deckId: buildContext.deckId,
          sourceVersion: buildContext.version,
          status: 'failed',
          slidevVersion: slidevCatalog.slidevVersion,
          startupMs: Date.now() - startedAt,
          diagnosticCode: error.code || 'slidev_preflight_failed',
          diagnosticMessage: error.publicMessage || 'Slidev could not compile this version.',
        })
      }
      catch (recordError) {
        console.error('Could not persist failed Slidev build', recordError)
      }
    }
    response.status(error.statusCode || 422).json({
      error: {
        code: error.code || 'slidev_preflight_failed',
        message: error.publicMessage || 'Slidev could not compile this version.',
      },
    })
  }
})

async function fetchDeckSource(deckId, version) {
  let result
  try {
    result = await fetch(`${apiBase}/internal/decks/${deckId}/source?version=${version}`, {
      headers: { 'x-interdeck-internal-token': internalToken },
      signal: AbortSignal.timeout(internalFetchTimeoutMs),
    })
  }
  catch (cause) {
    const error = new Error('Deck source request timed out or was unavailable', { cause })
    error.statusCode = 504
    error.code = 'deck_source_timeout'
    error.publicMessage = 'Preview validation could not reach the deck service in time. Retry the preview.'
    throw error
  }
  if (result.ok) return result.json()
  const error = new Error(`Deck source request failed with ${result.status}`)
  error.statusCode = result.status === 404 ? 409 : 502
  error.code = result.status === 404 ? 'slidev_version_superseded' : 'deck_source_unavailable'
  error.latestVersion = version
  error.publicMessage = result.status === 404
    ? 'This deck version has been replaced. Reload the editor to use the latest saved version.'
    : 'The deck source is temporarily unavailable.'
  throw error
}

async function recordDeckBuild({
  deckId,
  sourceVersion,
  status,
  slidevVersion,
  theme,
  startupMs,
  diagnosticCode,
  diagnosticMessage,
}) {
  let result
  try {
    result = await fetch(`${apiBase}/internal/decks/${deckId}/builds`, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        'x-interdeck-internal-token': internalToken,
      },
      body: JSON.stringify({
        source_version: sourceVersion,
        status,
        slidev_version: slidevVersion,
        theme,
        startup_ms: startupMs,
        diagnostic_code: diagnosticCode,
        diagnostic_message: diagnosticMessage,
      }),
      signal: AbortSignal.timeout(internalFetchTimeoutMs),
    })
  }
  catch (cause) {
    const error = new Error('Build record request timed out or was unavailable', { cause })
    error.statusCode = 504
    error.code = 'build_record_timeout'
    error.publicMessage = 'The deck compiled, but validation could not be recorded in time. Retry the preview.'
    throw error
  }
  if (result.ok) return result.json()
  const details = await result.text()
  let upstreamError
  try {
    upstreamError = JSON.parse(details)?.error
  }
  catch {
    upstreamError = null
  }
  const error = new Error(`Build record request failed with ${result.status}: ${details.slice(0, 500)}`)
  if (result.status === 422 && upstreamError?.code === 'response_integrity') {
    error.statusCode = 422
    error.code = upstreamError.code
    error.publicMessage = upstreamError.message
    throw error
  }
  error.statusCode = 502
  error.code = 'build_record_failed'
  error.publicMessage = 'The deck compiled, but its verified build record could not be saved. Retry the preview check.'
  throw error
}

app.use((request, response, next) => {
  if (!request.url.startsWith('/api/')) return next()
  response.setHeader('Cache-Control', 'no-store')
  proxy.web(request, response, { target: apiBase })
})

app.use('/slidev', async (request, response) => {
  try {
    if (!['GET', 'HEAD'].includes(request.method))
      return response.status(405).setHeader('Allow', 'GET, HEAD').send('Export routes are read-only')
    const path = request.originalUrl
    const parsed = parseSlidevPath(path)
    if (!parsed) return response.status(404).send('Slidev route not found')

    const requestUrl = new URL(path, 'http://gateway.local')
    const access = requestUrl.searchParams.get('access')
    if (access) {
      const payload = verifySlidevToken(access, parsed, slidevSecret)
      setSlidevCookie(response, access, payload, parsed.deckId, parsed.mode)
      requestUrl.searchParams.delete('access')
      return response.redirect(302, `${requestUrl.pathname}${requestUrl.search}`)
    }

    const token = parseCookies(request.headers.cookie).interdeck_slidev
    const payload = verifySlidevToken(token, parsed, slidevSecret)
    const target = await slidev.ensure({ ...parsed, version: payload.version })
    // Express removes the mounted `/slidev` prefix from request.url. Slidev is
    // configured with that full base path, so forward the original URL.
    request.url = request.originalUrl
    proxy.web(request, response, { target: target.target })
  }
  catch (error) {
    console.error('Slidev request failed', error)
    if (!response.headersSent) {
      if (error.statusCode === 401)
        response.status(401).send('Slide access expired')
      else
        response.status(error.statusCode || 422).type('html').send(renderSlidevError(error.publicMessage))
    }
  }
})

if (existsSync(webDist)) {
  app.use(express.static(webDist, { index: false, maxAge: process.env.APP_ENV === 'production' ? '1h' : 0 }))
  app.get('*splat', (request, response, next) => {
    if (request.path.startsWith('/api/') || request.path.startsWith('/slidev/')) return next()
    response.sendFile(join(webDist, 'index.html'))
  })
}
else {
  app.get('/', (_request, response) => {
    response.status(503).json({
      error: {
        code: 'web_not_built',
        message: 'The web application has not been built. Use the Vite development server on port 5173.',
      },
    })
  })
}

server.on('upgrade', async (request, socket, head) => {
  try {
    if (!request.url?.startsWith('/slidev/')) return socket.destroy()
    const parsed = parseSlidevPath(request.url)
    if (!parsed) return socket.destroy()
    const token = parseCookies(request.headers.cookie).interdeck_slidev
    const payload = verifySlidevToken(token, parsed, slidevSecret)
    const target = await slidev.ensure({ ...parsed, version: payload.version })
    proxy.ws(request, socket, head, { target: target.target })
  }
  catch (error) {
    const pathname = request.url ? new URL(request.url, 'http://gateway.local').pathname : 'unknown'
    console.error('Slidev WebSocket upgrade failed', {
      path: pathname,
      code: error?.code || 'slidev_websocket_upgrade_failed',
      message: error instanceof Error ? error.message : String(error),
    })
    socket.destroy()
  }
})

const bindAddress = process.env.GATEWAY_BIND_ADDR || (appEnv === 'production' ? '0.0.0.0' : '127.0.0.1')
server.listen(port, bindAddress, () => {
  console.log(`Interdeck gateway listening on ${bindAddress}:${port}`)
})

function parseSlidevPath(path) {
  const match = path.match(/^\/slidev\/(preview|present)\/([0-9a-f-]{36})(?:\/|\?|$)/i)
  return match ? { mode: match[1], deckId: match[2].toLowerCase() } : null
}

function verifySlidevToken(token, expected, secret) {
  if (!token) throw unauthorized()
  const [payloadPart, signaturePart, extra] = token.split('.')
  if (!payloadPart || !signaturePart || extra) throw unauthorized()
  const expectedSignature = createHmac('sha256', secret).update(payloadPart).digest()
  let suppliedSignature
  try {
    suppliedSignature = Buffer.from(signaturePart, 'base64url')
  }
  catch {
    throw unauthorized()
  }
  if (suppliedSignature.length !== expectedSignature.length || !timingSafeEqual(suppliedSignature, expectedSignature))
    throw unauthorized()
  let payload
  try {
    payload = JSON.parse(Buffer.from(payloadPart, 'base64url').toString('utf8'))
  }
  catch {
    throw unauthorized()
  }
  if (payload.exp <= Math.floor(Date.now() / 1000) || payload.deck_id !== expected.deckId || payload.mode !== expected.mode)
    throw unauthorized()
  return payload
}

function hasInternalAccess(request) {
  const supplied = Buffer.from(String(request.get('x-interdeck-internal-token') || ''))
  const expected = Buffer.from(internalToken)
  return supplied.length === expected.length && timingSafeEqual(supplied, expected)
}

function parseCookies(header = '') {
  return Object.fromEntries(header.split(';').flatMap(pair => {
    const index = pair.indexOf('=')
    return index > 0 ? [[pair.slice(0, index).trim(), pair.slice(index + 1).trim()]] : []
  }))
}

function setSlidevCookie(response, token, payload, deckId, mode) {
  response.cookie('interdeck_slidev', token, {
    httpOnly: true,
    sameSite: 'lax',
    secure: process.env.APP_ENV === 'production',
    maxAge: Math.max(1, payload.exp * 1000 - Date.now()),
    path: `/slidev/${mode}/${deckId}/`,
  })
}

function unauthorized() {
  return Object.assign(new Error('Slide access token is invalid or expired'), { statusCode: 401 })
}

function renderSlidevError(message) {
  const detail = escapeHtml(message || 'Slidev could not compile this version.')
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Slide preview unavailable</title>
  <style>
    :root { color-scheme: light; font-family: Inter, ui-sans-serif, system-ui, sans-serif; color: #172033; background: #f5f2eb; }
    body { min-height: 100vh; display: grid; place-items: center; margin: 0; }
    main { width: min(620px, calc(100% - 64px)); padding: 34px; border: 1px solid #ded8cc; border-radius: 18px; background: white; box-shadow: 0 18px 60px #26324a14; }
    small { color: #6b58d9; font-size: 12px; font-weight: 800; letter-spacing: .12em; }
    h1 { margin: 12px 0 10px; font-size: 28px; letter-spacing: -.03em; }
    p { margin: 0; color: #626978; font-size: 15px; line-height: 1.55; }
  </style>
</head>
<body><main><small>SLIDEV COMPATIBILITY CHECK</small><h1>Preview unavailable</h1><p>${detail}</p></main></body>
</html>`
}

function escapeHtml(value) {
  return String(value)
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#039;')
}

function shutdown() {
  slidev.close()
  server.close(() => process.exit(0))
  setTimeout(() => process.exit(1), 10_000).unref()
}

process.on('SIGTERM', shutdown)
process.on('SIGINT', shutdown)
