import assert from 'node:assert/strict'
import test from 'node:test'
import { runtimeConfig } from './runtime-config.mjs'

test('production rejects unset, short, reused, or predictable service keys', () => {
  for (const value of ['', 'short', 'a'.repeat(64), 'replace-me-in-production']) {
    assert.throws(() => runtimeConfig({ NODE_ENV: 'production', INTERNAL_SERVICE_TOKEN: value, SLIDEV_TOKEN_SECRET: value }))
  }
  const key = Array.from({ length: 16 }, (_, index) => index.toString(16)).join('').repeat(4)
  assert.throws(() => runtimeConfig({ APP_ENV: 'production', INTERNAL_SERVICE_TOKEN: key, SLIDEV_TOKEN_SECRET: key }))
  assert.equal(runtimeConfig({ APP_ENV: 'production', INTERNAL_SERVICE_TOKEN: key, SLIDEV_TOKEN_SECRET: key.split('').reverse().join('') }).appEnv, 'production')
  assert.equal(runtimeConfig({}).appEnv, 'development')
})
