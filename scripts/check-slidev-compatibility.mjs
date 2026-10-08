// Boots a real transformed Interdeck deck with every approved Slidev theme.
// This catches missing packages and theme/runtime incompatibilities before a
// pinned Slidev or theme upgrade reaches Railway.
import assert from 'node:assert/strict'
import { createServer } from 'node:http'

import { slidevCatalog } from '../services/gateway/src/slidev-catalog.mjs'
import { SlidevManager } from '../services/gateway/src/slidev-manager.mjs'

const internalToken = 'compatibility-fixture-token'
const sources = new Map()
const sourceServer = createServer((request, response) => {
  if (request.headers['x-interdeck-internal-token'] !== internalToken) {
    response.writeHead(401).end()
    return
  }
  const deckId = request.url?.match(/^\/internal\/decks\/([^/]+)\/source/)?.[1]
  const source = deckId && sources.get(deckId)
  if (!source) {
    response.writeHead(404).end()
    return
  }
  response.writeHead(200, { 'content-type': 'application/json' })
  response.end(JSON.stringify(source))
})

await new Promise((resolve, reject) => {
  sourceServer.once('error', reject)
  sourceServer.listen(0, '127.0.0.1', resolve)
})
const address = sourceServer.address()
if (!address || typeof address === 'string') throw new Error('fixture source server did not bind')

const manager = new SlidevManager({
  apiBase: `http://127.0.0.1:${address.port}`,
  internalToken,
  idleTtlSeconds: 60,
  maxServers: 1,
})
const checks = []

try {
  for (const [index, theme] of slidevCatalog.themes.entries()) {
    const deckId = `00000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`
    sources.set(deckId, {
      deck_id: deckId,
      title: `${theme.label} compatibility fixture`,
      version: 1,
      join_url: 'http://127.0.0.1/j/CMP-TST',
      markdown: fixtureMarkdown(theme.id),
    })
    const worker = await manager.ensure({ deckId, version: 1, mode: 'preview' })
    const response = await fetch(`${worker.target}${worker.base}`)
    assert.equal(response.status, 200, `${theme.id} returned ${response.status}`)
    assert.match(await response.text(), /<title>/i)
    checks.push({ theme: theme.id, package: theme.packageName, ok: true })
    manager.stop(worker)
    await new Promise(resolve => worker.child.once('exit', resolve))
  }
  console.log(JSON.stringify({ passed: true, slidev: slidevCatalog.slidevVersion, checks }, null, 2))
}
finally {
  manager.close()
  await new Promise(resolve => sourceServer.close(resolve))
}

function fixtureMarkdown(theme) {
  return `---
theme: ${theme}
title: Interdeck compatibility fixture
transition: slide-left
---
<!-- interdeck-slide: welcome -->

# ${theme} works

::audience-qr{size="120"}

---
<!-- interdeck-slide: interaction -->

:::interact{type="poll" id="compatibility-poll" results="after-vote"}
# Does the interaction runtime compile?

- [yes] Yes
- [also-yes] Also yes
:::
`
}
