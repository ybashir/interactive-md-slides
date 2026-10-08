import assert from 'node:assert/strict'
import test from 'node:test'

import { SlidevManager, startupDiagnostic } from './slidev-manager.mjs'

function fakeServer({ key, deckId, version, mode, lastUsedAt = 0 }) {
  return {
    key,
    deckId,
    version,
    mode,
    lastUsedAt,
    ready: true,
    child: {
      killed: false,
      kill() { this.killed = true },
    },
  }
}

function manager(maxServers = 4) {
  return new SlidevManager({ apiBase: 'http://api.local', internalToken: 'test', maxServers })
}

test('retires only lower versions for the same deck and mode', () => {
  const subject = manager()
  const oldPreview = fakeServer({ key: 'preview:deck-a:1', deckId: 'deck-a', version: 1, mode: 'preview' })
  const newPreview = fakeServer({ key: 'preview:deck-a:2', deckId: 'deck-a', version: 2, mode: 'preview' })
  const presentation = fakeServer({ key: 'present:deck-a:1', deckId: 'deck-a', version: 1, mode: 'present' })
  const otherDeck = fakeServer({ key: 'preview:deck-b:1', deckId: 'deck-b', version: 1, mode: 'preview' })
  subject.servers = new Map([oldPreview, newPreview, presentation, otherDeck].map(server => [server.key, server]))

  subject.retireSuperseded(newPreview)

  assert.equal(oldPreview.child.killed, true)
  assert.deepEqual([...subject.servers.keys()], [newPreview.key, presentation.key, otherDeck.key])
  subject.close()
})

test('a ready presentation releases its no-longer-needed preview worker', () => {
  const subject = manager()
  const preview = fakeServer({ key: 'preview:deck-a:2', deckId: 'deck-a', version: 2, mode: 'preview' })
  const presentation = fakeServer({ key: 'present:deck-a:2', deckId: 'deck-a', version: 2, mode: 'present' })
  subject.servers = new Map([preview, presentation].map(server => [server.key, server]))

  subject.retireSuperseded(presentation)

  assert.equal(preview.child.killed, true)
  assert.equal(presentation.child.killed, false)
  subject.close()
})

test('capacity eviction preserves the requested deck preview and presentation workers', () => {
  const subject = manager(3)
  const requestedPreview = fakeServer({ key: 'preview:deck-a:1', deckId: 'deck-a', version: 1, mode: 'preview', lastUsedAt: 1 })
  const presentation = fakeServer({ key: 'present:deck-b:1', deckId: 'deck-b', version: 1, mode: 'present', lastUsedAt: 2 })
  const disposablePreview = fakeServer({ key: 'preview:deck-c:1', deckId: 'deck-c', version: 1, mode: 'preview', lastUsedAt: 3 })
  subject.servers = new Map([requestedPreview, presentation, disposablePreview].map(server => [server.key, server]))

  subject.evictIfNeeded({ deckId: 'deck-a', mode: 'preview' })

  assert.equal(disposablePreview.child.killed, true)
  assert.equal(requestedPreview.child.killed, false)
  assert.equal(presentation.child.killed, false)
  subject.close()
})

test('reports capacity instead of interrupting a presentation or last-good preview', () => {
  const subject = manager(2)
  const requestedPreview = fakeServer({ key: 'preview:deck-a:1', deckId: 'deck-a', version: 1, mode: 'preview' })
  const presentation = fakeServer({ key: 'present:deck-b:1', deckId: 'deck-b', version: 1, mode: 'present' })
  subject.servers = new Map([requestedPreview, presentation].map(server => [server.key, server]))

  assert.throws(
    () => subject.evictIfNeeded({ deckId: 'deck-a', mode: 'preview' }),
    error => error.code === 'slidev_capacity_busy' && error.statusCode === 503,
  )
  assert.equal(requestedPreview.child.killed, false)
  assert.equal(presentation.child.killed, false)
  subject.close()
})

test('deduplicates concurrent startup requests for the same version', async () => {
  const subject = manager()
  let starts = 0
  let release
  const gate = new Promise(resolve => { release = resolve })
  const server = fakeServer({ key: 'preview:deck-a:2', deckId: 'deck-a', version: 2, mode: 'preview' })
  subject.start = async () => {
    starts += 1
    await gate
    return server
  }

  const first = subject.ensure({ deckId: 'deck-a', version: 2, mode: 'preview' })
  const second = subject.ensure({ deckId: 'deck-a', version: 2, mode: 'preview' })
  release()

  assert.equal(await first, server)
  assert.equal(await second, server)
  assert.equal(starts, 1)
  subject.close()
})

test('updates an active preview worker instead of starting a version-specific process', async () => {
  const subject = manager()
  const preview = fakeServer({ key: 'preview:deck-a', deckId: 'deck-a', version: 1, mode: 'preview' })
  subject.servers.set(preview.key, preview)
  let updates = 0
  let starts = 0
  subject.updatePreview = async (server, version) => {
    updates += 1
    server.version = version
    return server
  }
  subject.start = async () => {
    starts += 1
    return preview
  }

  const target = await subject.ensure({ deckId: 'deck-a', version: 2, mode: 'preview' })

  assert.equal(target, preview)
  assert.equal(target.version, 2)
  assert.equal(updates, 1)
  assert.equal(starts, 0)
  assert.deepEqual([...subject.servers.keys()], ['preview:deck-a'])
  subject.close()
})

test('an older preview token reuses the newest active deck worker', async () => {
  const subject = manager()
  const preview = fakeServer({ key: 'preview:deck-a', deckId: 'deck-a', version: 4, mode: 'preview' })
  subject.servers.set(preview.key, preview)

  const target = await subject.ensure({ deckId: 'deck-a', version: 3, mode: 'preview' })

  assert.equal(target, preview)
  assert.equal(target.version, 4)
  assert.equal(preview.child.killed, false)
  subject.close()
})

test('reports resource exhaustion as a capacity problem', () => {
  assert.equal(
    startupDiagnostic("OS can't spawn worker thread: Resource temporarily unavailable"),
    'Slide preview capacity was exhausted. Retry in a few seconds; the previous successful preview remains available.',
  )
})

test('export source always disables the upstream MCP endpoint', async () => {
  const { lockDownExportSource } = await import('./slidev-manager.mjs')
  const { parseSync } = await import('@slidev/parser')
  for (const source of ['# No headmatter', '---\ntheme: seriph\nmcp: true\n---\n# Existing headmatter']) {
    const result = lockDownExportSource(source)
    assert.equal(parseSync(result, 'slides.md').slides[0].frontmatter.mcp, false)
    assert.match(result, /# (No|Existing) headmatter/)
  }
})
