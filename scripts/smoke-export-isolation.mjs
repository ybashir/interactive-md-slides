import assert from 'node:assert/strict'
import { randomUUID } from 'node:crypto'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { SlidevManager } from '../services/gateway/src/slidev-manager.mjs'

const manager = new SlidevManager({ apiBase: 'http://unused.local', internalToken: 'export-test' })
const deckId = randomUUID()
const marker = `PUBLICATION_TEST_${randomUUID()}`
let canary
try {
  manager.fetchSource = async () => ({ join_url: 'http://localhost/j/TEST', markdown: '# Safe export' })
  const server = await manager.ensure({ deckId, version: 1, mode: 'export' })
  assert.match(server.transformed, /mcp: false/)
  const mcp = await fetch(`${server.target}/__mcp`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}' })
  assert.equal(mcp.status, 403, 'the upstream MCP mutation API must be disabled')
  assert.match(await mcp.text(), /MCP server is disabled/)
  assert.ok(!server.workspace.startsWith(resolve('.')), 'workspace must be outside the repository')
  canary = join(dirname(server.workspace), `${randomUUID()}.txt`)
  await writeFile(canary, marker)
  for (const path of [canary, resolve('README.md')]) {
    const response = await fetch(`${server.target}${server.base}@fs${path}?raw`)
    assert.ok(response.status >= 400, `filesystem access must be blocked: ${response.status}`)
    assert.ok(!(await response.text()).includes(marker))
  }
  for (const markdown of [`# Unsafe\n<<< ${canary}`, `---\nsrc: ${canary}\n---`]) {
    manager.fetchSource = async () => ({ join_url: 'http://localhost/j/TEST', markdown })
    await assert.rejects(manager.ensure({ deckId: randomUUID(), version: 1, mode: 'export' }), error => error.code === 'local_file_import')
  }
  console.log(JSON.stringify({ ok: true, repository_access: 'blocked', sibling_access: 'blocked', file_imports: 'blocked' }))
} finally {
  manager.close()
  if (canary) await rm(canary, { force: true })
}
