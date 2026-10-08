import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

import { applyClickGroups, collectVisibleElementInteractionIds, elementResultHtml, markClickSyntax, parseUniversalDeck, renderUniversalSlide } from './universal-deck.mjs'

const playerSource = readFileSync(new URL('./views/DeckPlayerView.vue', import.meta.url), 'utf8')
const liveQaSource = readFileSync(new URL('./components/player/UniversalLiveQa.vue', import.meta.url), 'utf8')
const interactionSource = readFileSync(new URL('./components/player/UniversalInteraction.vue', import.meta.url), 'utf8')
const presenterSource = readFileSync(new URL('./views/PresentView.vue', import.meta.url), 'utf8')

test('parses structural slide changes without a compiler workspace', () => {
  const deck = parseUniversalDeck(`---
theme: seriph
title: Universal player
transition: slide-left
---

<!-- interdeck-slide: opening -->

# Opening title

---
layout: two-cols-header
class: text-center
---

<!-- interdeck-slide: comparison -->

# Comparison

::left::

Left side

::right::

Right side
`)

  assert.equal(deck.theme, 'seriph')
  assert.equal(deck.title, 'Universal player')
  assert.equal(deck.slides.length, 2)
  assert.equal(deck.slides[0].title, 'Opening title')
  assert.equal(deck.slides[1].layout, 'two-cols-header')
  assert.equal(deck.slides[1].classes, 'text-center')
  assert.match(deck.slides[1].slots.left, /Left side/)
  assert.match(deck.slides[1].slots.right, /Right side/)
})

test('preserves scoped slide styles separately from deck-global styles', () => {
  const deck = parseUniversalDeck(`---
title: Styled deck
---

# Styled slide

<style scoped>
.slidev-layout h1 { color: rebeccapurple; }
</style>
`)

  assert.equal(deck.scopedStyles.length, 1)
  assert.equal(deck.scopedStyles[0].slideIndex, 0)
  assert.match(deck.scopedStyles[0].css, /rebeccapurple/)
  assert.doesNotMatch(deck.styles, /rebeccapurple/)
  assert.doesNotMatch(deck.slides[0].slots.default, /<style/)
})

test('renders live summary as a dedicated universal-player segment', () => {
  const deck = parseUniversalDeck(`# What the room told us\n\n::live-summary{show="all"}\n`)
  const rendered = renderUniversalSlide(deck.slides[0], [], [], 0)
  const summary = rendered.slots.default.find(segment => segment.kind === 'live-summary')
  assert.equal(summary?.show, 'all')
})

test('keeps interaction titles opt-in and removes a duplicated block heading when enabled', () => {
  const deck = parseUniversalDeck(`# Poll\n\n:::interact{type="poll" id="priority" show-title="true"}\n# Which priority?\n- [one] One\n- [two] Two\n:::\n`)
  const rendered = renderUniversalSlide(deck.slides[0], [{
    id: 'priority', kind: 'poll', title: 'Which priority?', options: [], config: {},
  }], [], 0)
  const interaction = rendered.slots.default.find(segment => segment.kind === 'interaction')

  assert.equal(interaction?.definition?.config?.['show-title'], 'true')
  assert.doesNotMatch(interaction?.bodyHtml || '', /Which priority/)
  assert.match(interactionSource, /showTitle.*\['show-title'\]/)
  assert.match(interactionSource, /v-if="showTitle && title"/)
})

test('provides projected renderers for every structured interaction family', () => {
  for (const kind of ['allocation', 'matrix', 'ranking', 'image-hotspot', 'survey', 'rating', 'number'])
    assert.match(interactionSource, new RegExp(`kind === '${kind}'`))
})

test('renders inline emoji reaction totals beside their element', () => {
  const html = elementResultHtml('reaction', {
    options: [
      { id: 'like', label: '👍' },
      { id: 'celebrate', label: '🎉' },
      { id: 'question', label: '🤔' },
    ],
    result: { count: 4, counts: { like: 1, celebrate: 3, question: 0 } },
  })

  assert.match(html, /universal-element-reactions/)
  assert.match(html, /👍<\/b> 1/)
  assert.match(html, /🎉<\/b> 3/)
  assert.doesNotMatch(html, /🤔/)
  assert.doesNotMatch(html, /universal-element-votes/)
})

test('reports only element interactions whose click containers are visible', () => {
  const visible = collectVisibleElementInteractionIds(`
    <ul>
      <li data-click="1" data-click-visible="true"><span data-interdeck-element-id="first"></span></li>
      <li data-click="2"><span data-interdeck-element-id="second"></span></li>
      <li><span data-interdeck-element-id="always"></span></li>
    </ul>
  `)

  assert.deepEqual([...visible].sort(), ['always', 'first'])
})

test('reveals an outer v-click before its nested v-clicks items', () => {
  const html = `<div data-click-single-start="2"></div><div class="option-two"><h2>Option 2</h2><div data-click-group-start="1"></div><ul><li>First detail</li><li>Second detail</li></ul><div data-click-group-end="1"></div></div><div data-click-single-end="2"></div>`
  const firstState = { next: 1 }
  const secondState = { next: 1 }
  const firstHtml = applyClickGroups(html, 1, firstState)
  const secondHtml = applyClickGroups(html, 2, secondState)

  assert.equal(firstState.next - 1, 3)
  assert.match(firstHtml, /data-click="1" data-click-visible="true"/)
  assert.match(firstHtml, /<li data-click="2">/)
  assert.match(secondHtml, /<li data-click="2" data-click-visible="true">/)
})

test('marks a v-click HTML directive outside its nested v-clicks group', () => {
  const marked = markClickSyntax(`<div v-click class="option-two"><h2>Option 2</h2><v-clicks><ul><li>First detail</li><li>Second detail</li></ul></v-clicks></div>`)
  const outerStart = marked.indexOf('data-click-single-start="1"')
  const innerStart = marked.indexOf('data-click-group-start="2"')
  const innerEnd = marked.indexOf('data-click-group-end="2"')
  const outerEnd = marked.indexOf('data-click-single-end="1"')

  assert.ok(outerStart >= 0)
  assert.ok(outerStart < innerStart)
  assert.ok(innerStart < innerEnd)
  assert.ok(innerEnd < outerEnd)
  assert.doesNotMatch(marked, /\bv-click\b/)

  const state = { next: 1 }
  const html = applyClickGroups(marked, 1, state)
  assert.equal(state.next - 1, 3)
  assert.match(html, /data-click="1" data-click-visible="true"/)
  assert.match(html, /<li data-click="2">/)
})

test('centers block media when a Slidev slide uses text-center', () => {
  assert.match(playerSource, /\.slidev-layout\.text-center :is\(img, svg, video, iframe\)[^{]*\{[^}]*margin-right: auto;[^}]*margin-left: auto;/)
})

test('keeps the universal presentation toolbar feature-complete', () => {
  for (const control of [
    'Fullscreen (F)',
    'Previous (←)',
    'Next (→ or Space)',
    'Slide overview (O)',
    'Laser pointer (L)',
    'Drawing tools (D)',
    'Record presentation (R)',
    'Keyboard shortcuts (?)',
  ]) assert.match(playerSource, new RegExp(control.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')))
})

test('reveals presentation controls only from the lower-left hover target or active tools', () => {
  assert.match(playerSource, /\.player-navigation \{[^}]*opacity: 0;/)
  assert.match(playerSource, /\.player-navigation-hotspot \{[^}]*bottom: 0;[^}]*left: 0;[^}]*width: 72px;[^}]*height: 72px;/)
  assert.match(playerSource, /\.player-navigation-hotspot:hover \+ \.player-navigation,[^{]*\.player-navigation:hover,[^{]*\.player-navigation:focus-within \{[^}]*opacity: 1;[^}]*pointer-events: auto;/)
  assert.doesNotMatch(playerSource, /force-visible/)
  assert.match(playerSource, /@click\.capture="releasePointerFocus"/)
  assert.match(playerSource, /@mouseleave="blurToolbarOnLeave"/)
})

test('uses only the hover player toolbar for navigation and fullscreen', () => {
  assert.doesNotMatch(presenterSource, /class="stage-controls"/)
  assert.match(playerSource, /document\.documentElement\.requestFullscreen\(\)/)
  assert.match(playerSource, /document\.exitFullscreen\(\)/)
})

test('keeps approved questions inside a presenter-scrollable slide region', () => {
  assert.match(liveQaSource, /height: 360px;[^}]*max-height: 360px;/s)
  assert.match(liveQaSource, /overflow-y: auto;/)
  assert.match(liveQaSource, /scrollbar-gutter: stable;/)
  assert.match(liveQaSource, /@wheel\.stop/)
  assert.match(liveQaSource, /@keydown="scrollQuestions"/)
  assert.match(liveQaSource, /\['ArrowDown', 'ArrowUp', 'PageDown', 'PageUp', 'Home', 'End'\]/)
})
