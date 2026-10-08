import assert from 'node:assert/strict'
import test from 'node:test'

import { findCurrentSlideContent, findSlideNumberByKey, replaceCurrentSlideContent } from './current-slide-source.mjs'

const deck = `---
theme: default
---

<!-- interdeck-slide: welcome -->
# Welcome

---
layout: center
class: text-center
---

<!-- interdeck-slide: centered -->
# Centered

---

<!-- interdeck-slide: final -->
# Final
`

test('keeps deck headmatter protected while exposing per-slide frontmatter', () => {
  assert.equal(findCurrentSlideContent(deck, 1)?.value, '# Welcome\n\n')
  assert.equal(
    findCurrentSlideContent(deck, 2)?.value,
    '---\nlayout: center\nclass: text-center\n---\n\n# Centered\n\n',
  )
  assert.equal(findCurrentSlideContent(deck, 3)?.value, '# Final\n')
})

test('replaces one slide and its frontmatter without changing its marker or neighbours', () => {
  const updated = replaceCurrentSlideContent(
    deck,
    2,
    '---\nlayout: image-right\nclass: text-left\n---\n\n# A safer focused edit\n\n',
  )
  assert.match(updated, /layout: image-right\nclass: text-left\n---\n\n<!-- interdeck-slide: centered -->\n# A safer focused edit/)
  assert.match(updated, /<!-- interdeck-slide: welcome -->\n# Welcome/)
  assert.match(updated, /<!-- interdeck-slide: final -->\n# Final/)
})

test('can add or remove per-slide frontmatter while preserving a valid separator', () => {
  const withFrontmatter = replaceCurrentSlideContent(
    deck,
    3,
    '---\nlayout: center\n---\n\n# Final centered\n',
  )
  assert.match(withFrontmatter, /---\nlayout: center\n---\n\n<!-- interdeck-slide: final -->\n# Final centered/)

  const withoutFrontmatter = replaceCurrentSlideContent(deck, 2, '# Centered without properties\n\n')
  assert.match(withoutFrontmatter, /# Welcome\n\n---\n\n<!-- interdeck-slide: centered -->\n# Centered without properties/)
})

test('never exposes or replaces deck headmatter from the first-slide editor', () => {
  const updated = replaceCurrentSlideContent(deck, 1, '# Renamed welcome\n\n')
  assert.ok(updated.startsWith('---\ntheme: default\n---'))
  assert.equal(findCurrentSlideContent(updated, 1)?.value, '# Renamed welcome\n\n')
})

test('clamps an out-of-range slide number to the final stable slide', () => {
  assert.equal(findCurrentSlideContent(deck, 99)?.slideKey, 'final')
})

test('finds a focused slide by its stable key', () => {
  assert.equal(findSlideNumberByKey(deck, 'centered'), 2)
  assert.equal(findSlideNumberByKey(deck, 'missing'), null)
})
