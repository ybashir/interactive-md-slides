import assert from 'node:assert/strict'
import { performance } from 'node:perf_hooks'

import { SlidevManager } from '../services/gateway/src/slidev-manager.mjs'

const deckId = '00000000-0000-4000-8000-000000000001'
const manager = new SlidevManager({
  apiBase: 'http://unused.local',
  internalToken: 'smoke-test',
  idleTtlSeconds: 60,
  maxServers: 2,
})

manager.fetchSource = async (_deckId, version) => ({
  join_url: 'http://localhost:5173/j/SMK-TST',
  markdown: `---
theme: seriph
background: ''
class: 'bg-white dark:bg-zinc-900'
---

# Lifecycle smoke version ${version}
`,
})

try {
  const startedAt = performance.now()
  const first = await manager.ensure({ deckId, version: 1, mode: 'preview' })
  const second = await manager.ensure({ deckId, version: 2, mode: 'preview' })

  // Slidev may need a replacement when its watcher cannot apply an HMR edit.
  // Both paths must serve the new source and keep one registered preview.
  assert.ok(first === second || first.version === 1, 'a replacement must preserve the previous verified source')
  assert.equal(second.child.killed, false, 'the updated preview should remain available')
  assert.deepEqual([...manager.servers.keys()], [`preview:${deckId}`])
  const module = await fetch(`${second.target}${second.base}slides.md__slidev_1.md?import`)
  assert.match(await module.text(), /Lifecycle smoke version 2/)

  console.log(JSON.stringify({
    ok: true,
    active_servers: manager.servers.size,
    active_version: second.version,
    elapsed_ms: Math.round(performance.now() - startedAt),
  }))
}
finally {
  manager.close()
}
