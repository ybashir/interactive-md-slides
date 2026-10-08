import assert from 'node:assert/strict'
import test from 'node:test'

import { diagnoseSource } from './source-diagnostics.mjs'

test('marks every duplicate slide and interaction ID at its source line', () => {
  const diagnostics = diagnoseSource(`<!-- interdeck-slide: intro -->
:::interact{type="poll" id="priority"}
:::
---
<!-- interdeck-slide: intro -->
- An idea {interact="vote" id='priority'}
`)
  assert.deepEqual(diagnostics.map(item => item.startLine), [1, 2, 5, 6])
  assert.match(diagnostics[0].message, /Duplicate slide ID/)
  assert.match(diagnostics[1].message, /Duplicate interaction ID/)
})

test('reports missing attributes and invalid stable IDs', () => {
  const diagnostics = diagnoseSource(`:::interact{results="always"}
- Vote here {interact="vote"}
<!-- interdeck-slide: x -->
`)
  assert.deepEqual(diagnostics.map(item => item.startLine), [1, 1, 2, 3])
  assert.match(diagnostics[0].message, /missing a stable/)
  assert.match(diagnostics[1].message, /missing a `type`/)
  assert.match(diagnostics[2].message, /Element interaction is missing/)
  assert.match(diagnostics[3].message, /must be 2–64/)
})

test('accepts valid block and element interaction syntax', () => {
  assert.deepEqual(diagnoseSource(`<!-- interdeck-slide: intro -->
:::interact{type="poll" id="priority"}
:::
- An idea {interact="vote" id="idea-vote"}
`), [])
})
