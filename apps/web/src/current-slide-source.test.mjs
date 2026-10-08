import assert from 'node:assert/strict'
import test from 'node:test'

import { findCurrentSlideContent, findProposalSlide, findSlideNumberByKey, replaceCurrentSlideContent } from './current-slide-source.mjs'

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

test('focuses an added slide when the assistant omits focus or points to an existing slide', () => {
  const proposed = deck.replace('<!-- interdeck-slide: final -->', '<!-- interdeck-slide: added -->\n# Added\n\n---\n\n<!-- interdeck-slide: final -->')
  for (const preferred of [undefined, 'centered', 'missing'])
    assert.deepEqual(findProposalSlide(deck, proposed, preferred, 2), { slideKey: 'added', slideNumber: 3 })
})

test('honors a chosen new slide when adding several and preserves focus for ordinary edits', () => {
  const proposed = `${deck}\n---\n\n<!-- interdeck-slide: first-new -->\n# New\n\n---\n\n<!-- interdeck-slide: second-new -->\n# New too\n`
  assert.deepEqual(findProposalSlide(deck, proposed, 'second-new', 1), { slideKey: 'second-new', slideNumber: 5 })
  assert.deepEqual(findProposalSlide(deck, deck.replace('# Centered', '# Edited'), undefined, 2), { slideKey: 'centered', slideNumber: 2 })
})

test('falls back to an existing nearby slide if a proposal removes the current one', () => {
  const proposed = deck.slice(0, deck.indexOf('\n---\n\n<!-- interdeck-slide: final -->'))
  assert.deepEqual(findProposalSlide(deck, proposed, 'missing', 3), { slideKey: 'centered', slideNumber: 2 })
})
