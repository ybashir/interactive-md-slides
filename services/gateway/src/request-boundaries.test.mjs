import assert from 'node:assert/strict'
import { createHmac } from 'node:crypto'
import { spawn } from 'node:child_process'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { once } from 'node:events'
import test from 'node:test'
import getPort from 'get-port'

test('real gateway limits costly routes and verifies every export credential', { timeout: 20000 }, async () => {
  const root = await mkdtemp(join(tmpdir(), 'interdeck-gateway-boundaries-'))
  await writeFile(join(root, 'index.html'), '<!doctype html><title>Fixture</title>')
  const port = await getPort({ host: '127.0.0.1' })
  const base = `http://127.0.0.1:${port}`
  const secret = 'development-slidev-token-secret'
  const child = spawn(process.execPath, [fileURLToPath(new URL('./index.mjs', import.meta.url))], {
    env: { ...process.env, APP_ENV: 'test', PORT: String(port), GATEWAY_BIND_ADDR: '127.0.0.1', GATEWAY_TRUST_PROXY: '0', INTERNAL_API_BASE: 'http://127.0.0.1:9', INTERNAL_SERVICE_TOKEN: 'development-internal-token', SLIDEV_TOKEN_SECRET: secret, WEB_DIST_DIR: root, GATEWAY_AUTH_REQUESTS_PER_MINUTE: '2', GATEWAY_SOURCE_REQUESTS_PER_MINUTE: '2', GATEWAY_STATIC_REQUESTS_PER_MINUTE: '2', GATEWAY_EXPORT_REQUESTS_PER_MINUTE: '20' },
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  let output = ''
  for (const stream of [child.stdout, child.stderr]) stream.on('data', data => { output = (output + data).slice(-10000) })
  try {
    let ready = false
    for (let attempt = 0; attempt < 100; attempt++) {
      try { ready = (await fetch(`${base}/_gateway/health`)).ok } catch {}
      if (ready) break
      assert.equal(child.exitCode, null, output)
      await new Promise(resolve => setTimeout(resolve, 50))
    }
    assert.ok(ready, output)
    for (let attempt = 0; attempt < 3; attempt++) {
      const forwarded = { 'x-forwarded-for': `192.0.2.${attempt + 1}` }
      const staticResponse = await fetch(base, { headers: forwarded })
      const sourceResponse = await fetch(`${base}/_gateway/slidev/preflight`, { method: 'POST', headers: { ...forwarded, 'content-type': 'application/json' }, body: JSON.stringify({ deck_id: '00000000-0000-4000-8000-000000000001', access: 'forged' }) })
      const authResponse = await fetch(`${base}/api/auth/google/start`, { headers: forwarded })
      if (attempt < 2) {
        assert.equal(staticResponse.status, 200)
        assert.equal(sourceResponse.status, 401)
        assert.equal(authResponse.status, 502)
      } else {
        for (const response of [staticResponse, sourceResponse, authResponse]) {
          assert.equal(response.status, 429, 'spoofed forwarded IPs must not evade untrusted-client limits')
          assert.ok(Number(response.headers.get('retry-after')) > 0)
          assert.equal((await response.json()).error.code, 'gateway_rate_limit')
        }
      }
    }
    const path = '/slidev/preview/00000000-0000-4000-8000-000000000001/'
    const payload = Buffer.from(JSON.stringify({ deck_id: '00000000-0000-4000-8000-000000000001', mode: 'preview', version: 1, exp: Math.floor(Date.now() / 1000) + 60 })).toString('base64url')
    const signed = `${payload}.${createHmac('sha256', secret).update(payload).digest('base64url')}`
    const rejected = await fetch(`${base}${path}?access=forged`, { headers: { cookie: `interdeck_slidev=${signed}` }, redirect: 'manual' })
    assert.equal(rejected.status, 401, 'an invalid query credential cannot fall back to a valid cookie')
    const accepted = await fetch(`${base}${path}?access=${signed}`, { redirect: 'manual' })
    assert.equal(accepted.status, 302)
    assert.equal(accepted.headers.get('location'), path)
    assert.match(accepted.headers.get('set-cookie'), /interdeck_slidev=.*HttpOnly/)
    assert.equal((await fetch(`${base}${path}`, { redirect: 'manual' })).status, 401)
    for (let request = 0; request < 17; request++)
      assert.equal((await fetch(`${base}${path}`)).status, 401)
    assert.equal((await fetch(`${base}${path}`)).status, 429)
  } finally {
    if (child.exitCode === null && child.signalCode === null) {
      const exited = once(child, 'exit')
      child.kill('SIGTERM')
      const deadline = setTimeout(() => child.kill('SIGKILL'), 10000)
      await exited
      clearTimeout(deadline)
    }
    await rm(root, { recursive: true, force: true })
  }
})
