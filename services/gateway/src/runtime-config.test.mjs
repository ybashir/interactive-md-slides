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

test('proxy trust and request limits require explicit bounded configuration', () => {
  assert.equal(runtimeConfig({}).trustProxy, 0)
  assert.equal(runtimeConfig({ GATEWAY_TRUST_PROXY: '1' }).trustProxy, 1)
  for (const value of ['true', '-1', '6', '1.5', 'anything'])
    assert.throws(() => runtimeConfig({ GATEWAY_TRUST_PROXY: value }))
  for (const value of ['-1', '0', '1.5', 'Infinity', 'NaN'])
    assert.throws(() => runtimeConfig({ GATEWAY_SOURCE_REQUESTS_PER_MINUTE: value }))
  assert.equal(runtimeConfig({ GATEWAY_SOURCE_REQUESTS_PER_MINUTE: '120' }).requestLimits.source, 120)
})
