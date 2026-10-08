import assert from 'node:assert/strict'
import test from 'node:test'
import { createRequire } from 'node:module'
import { verifyPatches } from './patch-dependencies.mjs'

const require = createRequire(import.meta.url)
test('installed security patches match the reviewed source hashes', () => verifyPatches())
test('deep brace patterns fail before recursive compilation can exhaust the stack', () => {
  const braces = require('braces')
  assert.deepEqual(braces.expand('a{b,c}'), ['ab', 'ac'])
  for (const opening of ['{', '(']) {
    assert.throws(() => braces(opening.repeat(4000) + 'x' + (opening === '{' ? '}' : ')').repeat(4000)), /nesting exceeds/)
  }
})
test('format widths and precision cannot allocate unbounded strings', () => {
  const { sprintf } = require('sprintf-js')
  assert.equal(sprintf('%03d', 7), '007')
  for (const format of ['%1000000000s', '%.1000000000f']) assert.throws(() => sprintf(format, 1), /exceeds 10000/)
})
