import assert from 'node:assert/strict'
import test from 'node:test'

import {
  deleteOrganizedSlide,
  duplicateOrganizedSlide,
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
