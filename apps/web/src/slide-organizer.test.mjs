import assert from 'node:assert/strict'
import test from 'node:test'
import { parseSync } from '@slidev/parser'

import {
  deleteOrganizedSlide,
  duplicateOrganizedSlide,
  insertOrganizedSlide,
  listOrganizedSlides,
  reorderOrganizedSlides,
} from './slide-organizer.mjs'

const source = `---
theme: default
title: Demo
---

<!-- interdeck-slide: welcome -->
# Welcome

---
layout: center
---

<!-- interdeck-slide: vote -->
# Vote

:::interact{type="poll" id="priority"}
- [a] A
- [b] B
:::

---

<!-- interdeck-slide: results -->
# Results

:::echarts{source="priority"}
{}
:::
`

test('lists stable slides and keeps the headmatter slide fixed', () => {
  const slides = listOrganizedSlides(source)
  assert.deepEqual(slides.map(slide => [slide.key, slide.title, slide.fixed]), [
    ['welcome', 'Welcome', true],
    ['vote', 'Vote', false],
    ['results', 'Results', false],
  ])
})

test('reorders complete slide units while preserving headmatter and frontmatter', () => {
  const reordered = reorderOrganizedSlides(source, ['results', 'vote'])
  assert.ok(reordered.startsWith('---\ntheme: default'))
  assert.ok(reordered.indexOf('interdeck-slide: results') < reordered.indexOf('interdeck-slide: vote'))
  assert.match(reordered, /layout: center[\s\S]*interdeck-slide: vote/)
})

test('duplicates a slide with fresh stable and interaction ids', () => {
  const duplicated = duplicateOrganizedSlide(source, 'vote')
  assert.equal(duplicated.slideKey, 'vote-copy')
  assert.match(duplicated.source, /interdeck-slide: vote-copy/)
  assert.match(duplicated.source, /id="priority-copy"/)
  assert.equal(listOrganizedSlides(duplicated.source).length, 4)
})

test('deletes non-title slides but protects deck headmatter', () => {
  assert.equal(deleteOrganizedSlide(source, 'welcome'), source)
  const deleted = deleteOrganizedSlide(source, 'vote')
  assert.doesNotMatch(deleted, /interdeck-slide: vote/)
  assert.match(deleted, /interdeck-slide: welcome/)
  assert.match(deleted, /interdeck-slide: results/)
})

test('inserts after the title or a middle slide without changing existing frontmatter and content', () => {
  for (const key of ['welcome', 'vote']) {
    const inserted = insertOrganizedSlide(source, key)
    const before = listOrganizedSlides(source)
    const after = listOrganizedSlides(inserted.source)
    assert.equal(after.findIndex(slide => slide.key === inserted.slideKey), before.findIndex(slide => slide.key === key) + 1)
    for (const slide of before) {
      const kept = after.find(candidate => candidate.key === slide.key)
      assert.equal(inserted.source.slice(kept.blockStart, kept.blockEnd), source.slice(slide.blockStart, slide.blockEnd))
    }
    assert.ok(inserted.source.startsWith('---\ntheme: default\ntitle: Demo\n---'))
    assert.equal(parseSync(inserted.source, 'slides.md').slides.length, 4)
  }
})

test('adds after the last slide without a final newline and creates unique stable keys', () => {
  let next = source.replace(/\n$/, '')
  const keys = []
  for (let i = 0; i < 3; i++) {
    const inserted = insertOrganizedSlide(next, 'results')
    keys.push(inserted.slideKey)
    next = inserted.source
  }
  assert.equal(new Set(keys).size, 3)
  assert.equal(parseSync(next, 'slides.md').slides.length, 6)
  assert.deepEqual(insertOrganizedSlide(source, 'missing'), { source, slideKey: null })
})
