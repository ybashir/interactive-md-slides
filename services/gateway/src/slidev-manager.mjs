import { spawn } from 'node:child_process'
import { createRequire } from 'node:module'
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import getPort from 'get-port'
import { parseSync } from '@slidev/parser'
import { parseDocument } from 'yaml'

import { validateSlidevCompatibility } from './slidev-catalog.mjs'
import {
  audienceCountComponent,
  chartComponent,
  elementVoteComponent,
  interactionComponent,
  liveQaComponent,
  liveSummaryComponent,
  liveStateComposable,
  navigationBridgeComponent,
  qrComponent,
  transformMarkdown,
} from './transform.mjs'

const require = createRequire(import.meta.url)
const slidevCli = require.resolve('@slidev/cli/bin/slidev.mjs')
const here = dirname(fileURLToPath(import.meta.url))

// Slidev's browser exporter relies on print-prefixed utility classes to undo
// its fixed, clipped on-screen shell. When that reset is missed by the browser,
// Chrome paginates the viewport instead of the slides: the tail of one slide is
// repeated above a clipped following slide on every PDF page. Keep this small
// print-only guard under Interdeck's control so user CSS, preview, and presenting
// are unaffected.
const interdeckPrintStyles = `
@media print {
  html,
  body,
  #app {
    width: auto !important;
    height: auto !important;
    min-height: 0 !important;
    overflow: visible !important;
  }

  body:has(#export-container) #app > div {
    position: static !important;
    inset: auto !important;
    display: block !important;
    width: auto !important;
    height: auto !important;
    min-height: 0 !important;
    overflow: visible !important;
  }

  #export-container,
  #export-content {
    position: static !important;
    display: block !important;
    width: auto !important;
    height: auto !important;
    max-width: none !important;
    max-height: none !important;
    margin: 0 !important;
    padding: 0 !important;
    overflow: visible !important;
    transform: none !important;
  }

  #export-content .print-slide-container {
    position: relative !important;
    flex: none !important;
    margin: 0 !important;
    overflow: hidden !important;
    transform: none !important;
    break-inside: avoid !important;
    break-after: page !important;
    page-break-after: always !important;
  }

  #export-content .print-slide-container:last-child {
    break-after: auto !important;
    page-break-after: auto !important;
  }
}
`.trim()

function workspaceStyles(css) {
  return [css?.trimEnd(), interdeckPrintStyles].filter(Boolean).join('\n\n')
}

export class SlidevManager {
  constructor({ apiBase, internalToken, idleTtlSeconds = 600, maxServers = 4 }) {
    this.apiBase = apiBase.replace(/\/$/, '')
    this.internalToken = internalToken
    this.idleTtlMs = Number(idleTtlSeconds) * 1000
    this.maxServers = Math.max(1, Number(maxServers) || 4)
    this.servers = new Map()
    this.starting = new Map()
    // Each worker gets an independent workspace outside the repository. Slidev
    // treats the nearest monorepo root as readable, including sibling decks.
    this.root = join(tmpdir(), 'interdeck-slidev')
    this.cleanupTimer = setInterval(() => this.cleanup(), 60_000)
    this.cleanupTimer.unref()
  }

  async ensure({ deckId, version, mode, allowRestart = true }) {
    const key = mode === 'preview' ? `preview:${deckId}` : `${mode}:${deckId}:${version}`
    const existing = this.servers.get(key)
    if (existing) {
      existing.lastUsedAt = Date.now()
      if (mode !== 'preview' || version <= existing.version) return existing

      const pending = this.starting.get(key)
      if (pending) {
        const target = await pending
        if (target.version >= version) return target
        return this.ensure({ deckId, version, mode, allowRestart })
      }
      const update = this.updatePreview(existing, version, allowRestart)
      this.starting.set(key, update)
      try {
        return await update
      }
      finally {
        if (this.starting.get(key) === update) this.starting.delete(key)
      }
    }

    const pending = this.starting.get(key)
    if (pending) {
      const target = await pending
      if (mode !== 'preview' || target.version >= version) return target
      return this.ensure({ deckId, version, mode, allowRestart })
    }

    const superseding = [...this.servers.values()].find(server => (
      server.ready
      && server.mode === mode
      && server.deckId === deckId
      && server.version > version
    ))
    if (superseding) {
      const error = new Error(`Version ${version} has been superseded by version ${superseding.version}`)
      error.statusCode = 409
      error.code = 'slidev_version_superseded'
      error.latestVersion = superseding.version
      error.publicMessage = 'This deck version has been replaced. Reload the editor to use the latest saved version.'
      throw error
    }

    this.evictIfNeeded({ deckId, mode })
    const startup = Promise.resolve().then(() => this.start({ deckId, version, mode, key }))
    this.starting.set(key, startup)
    try {
      return await startup
    }
    finally {
      if (this.starting.get(key) === startup)
        this.starting.delete(key)
    }
  }

  async updatePreview(server, version, allowRestart) {
    const source = await this.fetchSource(server.deckId, version)
    const css = source.css || ''
    const compatibility = validateSlidevCompatibility(source.markdown)
    const transformed = lockDownExportSource(transformMarkdown(source.markdown, {
      deckId: server.deckId,
      joinUrl: source.join_url,
    }))
    const parsed = parseSync(transformed, 'slides.md')
    const compatibilityChanged = !sameCompatibility(server.compatibility, compatibility)
    const cssChanged = css !== server.css
    const slideCountChanged = parsed.slides.length !== server.parsed.slides.length
    if (slideCountChanged && !compatibilityChanged && !cssChanged) {
      if (server.structuralRestartVersion === version) {
        if (!allowRestart)
          throw restartRequiredError('Slidev could not apply this structural change through HMR')
        return this.replacePreview(server, version)
      }
      return this.updateStructuralPreview(server, version, {
        compatibility,
        css,
        transformed,
        parsed,
      }, allowRestart)
    }

    const cannotHotUpdate = compatibilityChanged
      || cssChanged
      || styleBlocks(transformed) !== styleBlocks(server.transformed)
      || parsed.slides.some((slide, index) => (
        slide.content !== server.parsed.slides[index].content && slide.content.length === 0
      ))
    if (cannotHotUpdate) {
      if (!allowRestart) {
        throw restartRequiredError('This change requires a fresh Slidev worker')
      }
      return this.replacePreview(server, version)
    }

    const changedSlides = changedSlideIndexes(parsed.slides, server.parsed.slides)
    try {
      await updateSlidevSlides(server, parsed.slides, changedSlides)
      if (changedSlides.length === 0) {
        server.version = version
        server.compatibility = compatibility
        server.css = css
        server.transformed = transformed
        server.parsed = parsed
        server.slideCount = parsed.slides.length
        server.lastUsedAt = Date.now()
        return server
      }
      const moduleSignature = await verifySlideModules(
        server,
        parsed.slides.length,
        true,
        server.moduleSignature,
      )
      server.moduleSignature = moduleSignature
    }
    catch (error) {
      // A failed HMR update must not destroy the last preview that compiled.
      // Put the previous source back and make sure the worker recovered before
      // returning the compile error to the editor.
      let recovered = true
      try {
        await updateSlidevSlides(server, server.parsed.slides, changedSlides)
        server.moduleSignature = await verifySlideModules(server, server.slideCount)
      }
      catch {
        recovered = false
        this.stop(server)
      }
      if (/did not refresh (?:its )?(?:compiled )?slide modules in time/i.test(error?.message || '')) {
        if (!allowRestart)
          throw restartRequiredError('Slidev could not apply this change through HMR')
        return recovered
          ? this.replacePreview(server, version)
          : this.start({
              deckId: server.deckId,
              version,
              mode: 'preview',
              key: server.key,
            })
      }
      throw error
    }
    server.version = version
    server.compatibility = compatibility
    server.css = css
    server.transformed = transformed
    server.parsed = parsed
    server.slideCount = parsed.slides.length
    server.lastUsedAt = Date.now()
    return server
  }

  async updateStructuralPreview(server, version, next, allowRestart) {
    const previous = {
      transformed: server.transformed,
      slideCount: server.parsed.slides.length,
    }
    try {
      await writeFile(join(server.workspace, 'slides.md'), next.transformed)
      await waitForSlideRegistry(server, next.parsed.slides.length)
      server.moduleSignature = await verifySlideModules(
        server,
        next.parsed.slides.length,
        false,
        server.moduleSignature,
      )
    }
    catch (error) {
      const hmrUnavailable = error?.code === 'slidev_structural_hmr_unavailable'
        || /did not refresh (?:its )?(?:compiled )?slide modules in time/i.test(error?.message || '')
      let recovered = true
      try {
        await writeFile(join(server.workspace, 'slides.md'), previous.transformed)
        await waitForSlideRegistry(server, previous.slideCount)
        server.moduleSignature = await verifySlideModules(server, previous.slideCount, false)
      }
      catch {
        recovered = false
        this.stop(server)
      }
      if (hmrUnavailable) {
        server.structuralRestartVersion = version
        if (!allowRestart)
          throw restartRequiredError('Slidev could not apply this structural change through HMR')
        return recovered
          ? this.replacePreview(server, version)
          : this.start({
              deckId: server.deckId,
              version,
              mode: 'preview',
              key: server.key,
            })
      }
      throw error
    }

    server.version = version
    server.compatibility = next.compatibility
    server.css = next.css
    server.transformed = next.transformed
    server.parsed = next.parsed
    server.slideCount = next.parsed.slides.length
    server.lastUsedAt = Date.now()
    server.structuralRestartVersion = null
    return server
  }

  async replacePreview(previous, version) {
    const temporaryKey = `${previous.key}:replacement:${version}:${Date.now()}`
    const replacement = await this.start({
      deckId: previous.deckId,
      version,
      mode: 'preview',
      key: temporaryKey,
      retireExisting: false,
    })
    if (this.servers.get(temporaryKey) === replacement)
      this.servers.delete(temporaryKey)
    replacement.key = previous.key
    this.servers.set(previous.key, replacement)
    this.retireSuperseded(replacement)

    // Existing iframe requests and HMR sockets may still be attached to the
    // previous worker. Give them a short drain window after the replacement is
    // healthy so they reload against the new target instead of seeing a 502.
    const drainTimer = setTimeout(() => this.stop(previous), 5_000)
    drainTimer.unref()
    return replacement
  }

  async start({ deckId, version, mode, key, retireExisting = true }) {
    const source = await this.fetchSource(deckId, version)
    const compatibility = validateSlidevCompatibility(source.markdown)
    const transformed = lockDownExportSource(transformMarkdown(source.markdown, {
      deckId,
      joinUrl: source.join_url,
    }))
    const parsed = parseSync(transformed, 'slides.md')
    const slideCount = parsed.slides.length
    await mkdir(this.root, { recursive: true })
    const workspace = await mkdtemp(join(this.root, `${mode}-${deckId}-${version}-`))
    const components = join(workspace, 'components')
    const composables = join(workspace, 'composables')
    const chartSpec = await readFile(resolve(here, '../../../apps/web/src/chart-spec.mjs'), 'utf8')
    await Promise.all([
      mkdir(components, { recursive: true }),
      mkdir(composables, { recursive: true }),
      mkdir(join(workspace, '.home'), { recursive: true }),
      mkdir(join(workspace, '.tmp'), { recursive: true }),
      symlink(resolve(here, '../../../node_modules'), join(workspace, 'node_modules'), 'dir'),
    ])
    await Promise.all([
      writeFile(join(workspace, 'slides.md'), transformed),
      writeFile(join(workspace, 'style.css'), workspaceStyles(source.css)),
      writeFile(join(workspace, 'package.json'), JSON.stringify({ private: true, type: 'module' }, null, 2)),
      // Slidev otherwise treats the CLI as globally installed and places Vite's
      // dependency cache under root-owned node_modules in the production image.
      writeFile(
        join(workspace, 'vite.config.mjs'),
        viteConfig(process.env.NODE_ENV === 'production' || process.env.APP_ENV === 'production', workspace),
      ),
      writeFile(join(components, 'InterdeckInteraction.vue'), interactionComponent),
      writeFile(join(components, 'InterdeckElementVote.vue'), elementVoteComponent),
      writeFile(join(components, 'InterdeckLiveQA.vue'), liveQaComponent),
      writeFile(join(components, 'InterdeckLiveSummary.vue'), liveSummaryComponent),
      writeFile(join(components, 'AudienceQR.vue'), qrComponent),
      writeFile(join(components, 'AudienceCount.vue'), audienceCountComponent),
      writeFile(join(components, 'InterdeckChart.vue'), chartComponent),
      writeFile(join(composables, 'useInterdeckLive.js'), liveStateComposable),
      writeFile(join(composables, 'chartSpec.mjs'), chartSpec),
      writeFile(join(workspace, 'global-top.vue'), navigationBridgeComponent),
    ])

    const port = await getPort({ host: '127.0.0.1' })
    const base = `/slidev/${mode}/${deckId}/`
    const child = spawn(process.execPath, [
      slidevCli,
      join(workspace, 'slides.md'),
      '--port', String(port),
      '--base', base,
      '--log', 'error',
    ], {
      cwd: workspace,
      env: {
        PATH: process.env.PATH,
        HOME: join(workspace, '.home'),
        TMPDIR: join(workspace, '.tmp'),
        // The gateway is a production process, but each embedded Slidev instance
        // is intentionally a Vite development server for near-real-time preview.
        NODE_ENV: 'development',
        BROWSER: 'none',
        CI: '1',
        // Native tooling used by Slidev otherwise sees Railway's host CPU count
        // and may create dozens of threads per deck inside a PID-limited container.
        TOKIO_WORKER_THREADS: process.env.SLIDEV_TOKIO_WORKER_THREADS || '2',
        RAYON_NUM_THREADS: process.env.SLIDEV_RAYON_NUM_THREADS || '2',
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    })

    const server = {
      key,
      child,
      compatibility,
      css: source.css || '',
      transformed,
      parsed,
      slideCount,
      deckId,
      version,
      mode,
      base,
      port,
      target: `http://localhost:${port}`,
      workspace,
      lastUsedAt: Date.now(),
      ready: false,
      logs: [],
    }
    this.servers.set(key, server)
    const capture = chunk => {
      const text = chunk.toString()
      server.logs.push(text)
      if (server.logs.length > 30) server.logs.shift()
    }
    child.stdout.on('data', capture)
    child.stderr.on('data', capture)
    child.once('exit', () => {
      if (this.servers.get(server.key) === server)
        this.servers.delete(server.key)
      else {
        const registered = [...this.servers.entries()].find(([, candidate]) => candidate === server)
        if (registered) this.servers.delete(registered[0])
      }
      rm(workspace, { recursive: true, force: true }).catch(() => {})
    })

    try {
      await waitForServer(server)
      server.moduleSignature = await verifySlideModules(server, slideCount, false)
      server.ready = true
      if (retireExisting) this.retireSuperseded(server)
      return server
    }
    catch (error) {
      this.stop(server)
      const details = server.logs.join('').trim()
      const reason = error instanceof Error ? error.message : String(error)
      const startupError = new Error(`Slidev did not start for deck ${deckId}: ${details ? `${details}\n${reason}` : reason}`)
      const capacityFailure = /can't spawn worker thread|resource temporarily unavailable/i.test(details)
      startupError.statusCode = capacityFailure ? 503 : 422
      startupError.code = capacityFailure ? 'slidev_capacity_exhausted' : 'slidev_startup_failed'
      startupError.publicMessage = startupDiagnostic(details)
      throw startupError
    }
  }

  async fetchSource(deckId, version) {
    const response = await fetch(`${this.apiBase}/internal/decks/${deckId}/source?version=${version}`, {
      headers: { 'x-interdeck-internal-token': this.internalToken },
    })
    if (!response.ok)
      throw new Error(`Deck source request failed with ${response.status}`)
    return response.json()
  }

  evictIfNeeded(requested) {
    const reservedStarts = [...this.starting.keys()].filter(key => !this.servers.has(key)).length
    if (this.servers.size + reservedStarts < this.maxServers) return
    const candidates = [...this.servers.values()]
      .filter(server => server.mode === 'preview' && server.deckId !== requested.deckId)
      .sort((a, b) => a.lastUsedAt - b.lastUsedAt)
    if (candidates[0]) return this.stop(candidates[0])

    const error = new Error('No disposable Slidev worker slot is available')
    error.statusCode = 503
    error.code = 'slidev_capacity_busy'
    error.publicMessage = 'Slide preview capacity is temporarily busy. Try again in a few seconds.'
    throw error
  }

  retireSuperseded(readyServer) {
    for (const server of this.servers.values()) {
      if (server === readyServer || server.deckId !== readyServer.deckId) continue
      const olderInSameMode = server.mode === readyServer.mode && server.version < readyServer.version
      const previewReplacedByPresentation = (
        readyServer.mode === 'present'
        && server.mode === 'preview'
        && server.version <= readyServer.version
      )
      if (olderInSameMode || previewReplacedByPresentation)
        this.stop(server)
    }
  }

  cleanup() {
    const cutoff = Date.now() - this.idleTtlMs
    for (const server of this.servers.values()) {
      if (server.lastUsedAt < cutoff)
        this.stop(server)
    }
  }

  stop(server) {
    if (this.servers.get(server.key) === server)
      this.servers.delete(server.key)
    else {
      const registered = [...this.servers.entries()].find(([, candidate]) => candidate === server)
      if (registered) this.servers.delete(registered[0])
    }
    if (!server.child.killed)
      server.child.kill('SIGTERM')
  }

  close() {
    clearInterval(this.cleanupTimer)
    for (const server of this.servers.values())
      this.stop(server)
  }
}

function viteConfig(production, workspace) {
  const config = production
    ? {
        cacheDir: '.vite',
        // Railway randomly distributes requests between gateway replicas and
        // does not provide sticky sessions. Vite's dependency optimizer emits
        // worker-local, revisioned `.vite/deps` URLs; an export page generated
        // by one replica can otherwise request a file that only exists in the
        // other replica's optimizer generation and receive 504 Outdated
        // Optimize Dep. Serving dependencies directly is a little less warm-
        // cache efficient, but makes the export worker's HTTP surface
        // deterministic across replicas.
        optimizeDeps: { noDiscovery: true, include: [] },
        server: { ws: { protocol: 'wss', clientPort: 443 } },
      }
    : { cacheDir: '.vite', optimizeDeps: { noDiscovery: true, include: [] } }
  config.server = {
    ...config.server,
    host: '127.0.0.1',
    fs: { strict: true, allow: [workspace, resolve(here, '../../../node_modules')] },
  }
  return `export default ${JSON.stringify(config, null, 2)}\n`
}

export function lockDownExportSource(markdown) {
  const match = markdown.match(/^---[\t ]*\r?\n([\s\S]*?)\r?\n---(?:[\t ]*\r?\n|$)/)
  if (!match) return `---\nmcp: false\n---\n\n${markdown}`
  const headmatter = parseDocument(match[1])
  if (headmatter.errors.length) throw headmatter.errors[0]
  headmatter.set('mcp', false)
  return `---\n${headmatter.toString()}---\n${markdown.slice(match[0].length)}`
}

export function startupDiagnostic(details) {
  const missingTheme = details.match(/The theme "([^"]+)" was not found/)
  if (missingTheme) {
    return `Theme "${missingTheme[1]}" is approved but missing from this worker image. Please report this deployment problem.`
  }
  if (/can't spawn worker thread|resource temporarily unavailable/i.test(details))
    return 'Slide preview capacity was exhausted. Retry in a few seconds; the previous successful preview remains available.'
  return 'Slidev could not compile this version. Check the deck headmatter, Markdown, Vue components, and referenced local files.'
}

function sameCompatibility(left, right) {
  return left?.theme === right?.theme
    && JSON.stringify(left?.addons || []) === JSON.stringify(right?.addons || [])
}

function styleBlocks(markdown) {
  return [...markdown.matchAll(/<style\b[^>]*>[\s\S]*?<\/style>/gi)]
    .map(match => match[0].trim())
    .join('\n')
}

function restartRequiredError(message) {
  const error = new Error(message)
  error.statusCode = 409
  error.code = 'slidev_restart_required'
  error.publicMessage = 'This saved change requires Slidev to restart. Select Refresh preview to apply it.'
  return error
}

function changedSlideIndexes(nextSlides, previousSlides) {
  return nextSlides.flatMap((slide, index) => {
    const previous = previousSlides[index]
    return slide.content.trim() !== previous.content.trim()
      || (slide.frontmatterRaw || '') !== (previous.frontmatterRaw || '')
      || (slide.note || '') !== (previous.note || '')
      ? [index]
      : []
  })
}

async function updateSlidevSlides(server, slides, indexes) {
  for (const index of indexes) {
    const slide = slides[index]
    const response = await fetch(`${server.target}/__slidev/slides/${index + 1}.json`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        content: slide.content,
        frontmatterRaw: slide.frontmatterRaw || '',
        note: slide.note || '',
      }),
      signal: AbortSignal.timeout(5_000),
    })
    if (response.ok) continue
    const details = await response.text()
    throw previewCompileError(details.slice(0, 2_000))
  }
}

async function waitForServer(server) {
  const deadline = Date.now() + 45_000
  while (Date.now() < deadline) {
    if (server.child.exitCode != null)
      throw new Error(`Slidev exited with code ${server.child.exitCode}`)
    try {
      const response = await fetch(`${server.target}${server.base}`, {
        signal: AbortSignal.timeout(1000),
      })
      if (response.ok) return
    }
    catch {}
    await new Promise(resolve => setTimeout(resolve, 250))
  }
  throw new Error('Slidev startup timed out')
}

async function waitForSlideRegistry(server, slideCount) {
  const deadline = Date.now() + 4_000
  const expected = new RegExp(`componentsCache\\s*=\\s*new Array\\(${slideCount}\\)`)
  let lastFailure = ''
  while (Date.now() < deadline) {
    try {
      const response = await fetch(
        `${server.target}${server.base}@slidev/slides?t=${Date.now()}`,
        {
          headers: { 'cache-control': 'no-cache' },
          signal: AbortSignal.timeout(2_000),
        },
      )
      const body = await response.text()
      if (response.ok
        && (response.headers.get('content-type') || '').includes('javascript')
        && expected.test(body)) return
      lastFailure = body
    }
    catch (error) {
      lastFailure = error instanceof Error ? error.message : String(error)
    }
    await delay(50)
  }
  const error = new Error(lastFailure.slice(0, 2_000) || 'Slidev did not refresh its slide registry in time')
  error.statusCode = 409
  error.code = 'slidev_structural_hmr_unavailable'
  throw error
}

async function verifySlideModules(server, slideCount, waitForWatcher = true, previousSignature = null) {
  if (waitForWatcher) await delay(100)
  const deadline = Date.now() + 3_000
  let lastFailure = ''
  while (Date.now() < deadline) {
    const checkTimestamp = Date.now()
    const entry = await fetch(`${server.target}${server.base}@slidev/slides`, {
      headers: { 'cache-control': 'no-cache' },
      signal: AbortSignal.timeout(2_000),
    })
    const entryType = entry.headers.get('content-type') || ''
    if (!entry.ok || !entryType.includes('javascript')) {
      lastFailure = await entry.text()
      await delay(50)
      continue
    }

    // Load the facades before their concrete modules so Vite records the same
    // dependency graph a browser creates. Slidev's HMR loader uses that graph
    // to invalidate the compiled Markdown/Vue module after an edit.
    const facades = await Promise.all(Array.from({ length: slideCount }, (_, index) => (
      Promise.all(['md', 'frontmatter'].map(moduleType => fetch(
        `${server.target}${server.base}@slidev/slides/${index + 1}/${moduleType}`, {
          headers: { 'cache-control': 'no-cache' },
          signal: AbortSignal.timeout(5_000),
        },
      )))
    )))
    const failedFacade = facades.flat().find(response => (
      !response.ok || !(response.headers.get('content-type') || '').includes('javascript')
    ))
    if (failedFacade) {
      lastFailure = await failedFacade.text()
      await delay(50)
      continue
    }

    const responseGroups = await Promise.all(Array.from({ length: slideCount }, (_, index) => (
      Promise.all(['md', 'frontmatter'].map(moduleType => fetch(
        `${server.target}${server.base}slides.md__slidev_${index + 1}.${moduleType}?import&t=${checkTimestamp}`, {
          headers: { 'cache-control': 'no-cache' },
          signal: AbortSignal.timeout(5_000),
        },
      )))
    )))
    const responses = responseGroups.flat()
    const bodies = await Promise.all(responses.map(response => response.text()))
    const failedIndex = responses.findIndex(response => (
      !response.ok || !(response.headers.get('content-type') || '').includes('javascript')
    ))
    if (failedIndex < 0) {
      // The frontmatter module can be invalidated before the Markdown/Vue
      // module is ready. Only the compiled slide bodies prove the visual
      // update has actually crossed Vite's watcher boundary.
      const signature = bodies
        .filter((_, index) => index % 2 === 0)
        .map(body => body.replace(/([?&])t=\d+/g, '$1t=*'))
        .join('\n/* interdeck-slide-boundary */\n')
      if (previousSignature === null || signature !== previousSignature) return signature
      lastFailure = 'Slidev did not refresh its compiled slide modules in time'
      await delay(50)
      continue
    }
    lastFailure = bodies[failedIndex] || `Slide module request failed with ${responses[failedIndex].status}`
    await delay(50)
  }
  throw previewCompileError(lastFailure.slice(0, 2_000) || 'Slidev did not refresh its slide modules in time')
}

function previewCompileError(details) {
  const error = new Error(details || 'Slidev could not compile the updated preview')
  error.statusCode = 422
  error.code = 'slidev_compile_failed'
  error.publicMessage = 'Slidev could not compile this version. Check the highlighted Markdown and try again.'
  return error
}

function delay(milliseconds) {
  return new Promise(resolve => setTimeout(resolve, milliseconds))
}
