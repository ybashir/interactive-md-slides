import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

import { elementVoteComponent, interactionComponent, liveQaComponent, liveSummaryComponent, transformMarkdown } from './transform.mjs'

test('transforms block, element, QR, and live Q&A interactions into Slidev components', () => {
  const source = `# Demo

::audience-qr{size="160"}

::live-qa{max="5" show="all"}

::live-summary

:::interact{type="poll" id="choice"}
# Choose one
- [one] One
- [two] Two
:::

- Ship weekly {interact="updown" id="ship-weekly"}
- Celebrate progress {interact="reaction" id="progress-reaction" options="like:👍|celebrate:🎉|question:🤔"}
- Confidence {interact="rating" id="confidence" min="1" max="5"}
`
  const result = transformMarkdown(source, { deckId: 'deck-1', joinUrl: 'https://slides.example/j/ABC-123' })
  assert.match(result, /<AudienceQR/)
  assert.match(result, /<InterdeckLiveQA/)
  assert.match(result, /<InterdeckLiveSummary/)
  assert.match(result, /:max-themes="5" show="all"/)
  assert.match(result, /deck-id="deck-1"/)
  assert.match(result, /fallback-url="https:\/\/slides\.example\/j\/ABC-123"/)
  assert.match(result, /<InterdeckInteraction/)
  assert.match(result, /interaction-id="choice"/)
  assert.doesNotMatch(result, /\n- One/)
  assert.doesNotMatch(result, /\n- Two/)
  assert.match(result, /<InterdeckElementVote/)
  assert.match(result, /interaction-id="progress-reaction" kind="reaction"/)
  assert.match(result, /options-json="\[\{&quot;id&quot;:&quot;like&quot;,&quot;label&quot;:&quot;👍&quot;\}/)
  assert.match(result, /interaction-id="confidence" kind="rating"/)
  assert.doesNotMatch(result, /:::interact/)
  assert.doesNotMatch(result, /::live-qa/)
  assert.doesNotMatch(result, /::live-summary/)
})

test('renders poll options once as proportional live bars', () => {
  assert.match(interactionComponent, /const choiceRows = computed/)
  assert.match(interactionComponent, /count \/ total \* 100/)
  assert.match(interactionComponent, /v-for="option in choiceRows"/)
  assert.match(interactionComponent, /Math\.round\(option\.percentage\) \}\}%/)
  assert.match(interactionComponent, /transition: width \.45s/)
})

test('renders an interaction title only when show-title is explicitly enabled', () => {
  const source = `:::interact{type="poll" id="choice" question="Choose one" show-title="true"}\n# Duplicate heading\n- [one] One\n- [two] Two\n:::`
  const result = transformMarkdown(source, { deckId: 'deck-1', joinUrl: 'https://slides.example/j/ABC-123' })

  assert.match(result, /source-title="Choose one"/)
  assert.match(result, /:show-title="true"/)
  assert.doesNotMatch(result, /# Duplicate heading/)
  assert.match(interactionComponent, /v-if="showTitle && title"/)

  const defaultResult = transformMarkdown(source.replace(' show-title="true"', ''), { deckId: 'deck-1', joinUrl: 'https://slides.example/j/ABC-123' })
  assert.match(defaultResult, /:show-title="false"/)
  assert.match(defaultResult, /# Duplicate heading/)
})

test('renders separate positive and negative element vote totals', () => {
  assert.match(elementVoteComponent, /result\?\.up \|\| 0/)
  assert.match(elementVoteComponent, /result\?\.down \|\| 0/)
  assert.match(elementVoteComponent, /▲ \{\{ up \}\}/)
  assert.match(elementVoteComponent, /▼ \{\{ down \}\}/)
  assert.doesNotMatch(elementVoteComponent, /result\?\.score/)
  assert.match(elementVoteComponent, /\.element-vote\.up \{ color: #15803d;/)
  assert.match(elementVoteComponent, /\.element-vote\.down \{ color: #dc2626;/)
})

test('renders inline single-select reaction totals in exported decks', () => {
  assert.match(elementVoteComponent, /kind === 'reaction'/)
  assert.match(elementVoteComponent, /interaction\.value\?\.result\?\.counts\?\.\[option\.id\]/)
  assert.match(elementVoteComponent, /filter\(option => option\.count > 0\)/)
  assert.match(elementVoteComponent, /kind === 'reaction' && reactions\.length/)
  assert.match(elementVoteComponent, /v-for="option in reactions"/)
  assert.match(elementVoteComponent, /\{\{ option\.label \}\}/)
  assert.match(elementVoteComponent, /\{\{ option\.count \}\}/)
})

test('keeps compiled Slidev Q&A inside a presenter-scrollable slide region', () => {
  assert.match(liveQaComponent, /height: 360px;[^}]*max-height: 360px;/s)
  assert.match(liveQaComponent, /overflow-y: auto;/)
  assert.match(liveQaComponent, /scrollbar-gutter: stable;/)
  assert.match(liveQaComponent, /@wheel\.stop/)
  assert.match(liveQaComponent, /@keydown="scrollQuestions"/)
})

test('keeps the live deck summary current-epoch and presenter-scrollable', () => {
  const result = transformMarkdown('::live-summary{show="all"}', { deckId: 'deck-1', joinUrl: 'https://slides.example/j/ABC-123' })
  assert.match(result, /<InterdeckLiveSummary deck-id="deck-1" show="all"/)
  assert.match(liveSummaryComponent, /\/live-summary/)
  assert.match(liveSummaryComponent, /window\.setInterval\(refresh, 5000\)/)
  assert.match(liveSummaryComponent, /overflow-y: auto;/)
  assert.match(liveSummaryComponent, /@wheel\.stop/)
  assert.match(liveSummaryComponent, /@keydown="scrollReport"/)
})

test('keeps private deck images as same-origin runtime resources', () => {
  const ownUrl = '/api/decks/dec-1/assets/a55e-1/content'
  const otherUrl = '/api/decks/dec-2/assets/a55e-2/content'
  const source = `# Assets

![Loop & planning](${ownUrl})
![Another deck](${otherUrl})
![Remote](https://example.com/image.svg)
`
  const result = transformMarkdown(source, { deckId: 'dec-1', joinUrl: 'https://slides.example/j/ABC-123' })

  assert.match(result, /<img :src="'\/api\/decks\/dec-1\/assets\/a55e-1\/content'" alt="Loop &amp; planning" \/>/)
  assert.match(result, /!\[Another deck\]\(\/api\/decks\/dec-2\/assets\/a55e-2\/content\)/)
  assert.match(result, /!\[Remote\]\(https:\/\/example\.com\/image\.svg\)/)
})

test('keeps raw private asset images at runtime and preserves HTML sizing', () => {
  const ownUrl = '/api/decks/dec-1/assets/a55e-1/content'
  const otherUrl = '/api/decks/dec-2/assets/a55e-2/content'
  const source = `# Raw assets

\\<img src="${ownUrl}" width="300">
<img class="diagram" src='${ownUrl}' width="50%" />
<img src="${otherUrl}" width="200">
`
  const result = transformMarkdown(source, { deckId: 'dec-1', joinUrl: 'https://slides.example/j/ABC-123' })

  assert.match(result, new RegExp(`<img :src="'${ownUrl}'" width="300">`))
  assert.match(result, new RegExp(`<img class="diagram" :src="'${ownUrl}'" width="50%" \\/>`))
  assert.match(result, new RegExp(`<img src="${otherUrl}" width="200">`))
  assert.doesNotMatch(result, /\\<img/)
})

test('preserves reaction options for the live slide renderer', () => {
  const source = `# React

:::interact{type="reaction" id="reaction" results="always"}
# How does this land?
- [love] ❤️ Love it
- [question] ❓ Questions
:::
`
  const result = transformMarkdown(source, { deckId: 'deck-1', joinUrl: 'https://slides.example/j/ABC-123' })

  assert.match(result, /kind="reaction"/)
  assert.match(result, /options-json="\[\{&quot;id&quot;:&quot;love&quot;/)
  assert.match(result, /&quot;label&quot;:&quot;❤️ Love it&quot;/)
  assert.doesNotMatch(result, /\n- ❤️ Love it/)
})

test('delegates responsive word-cloud packing with configurable size and spacing', () => {
  const source = `:::interact{type="word-cloud" id="mood" height="380" spacing="0.2"}\n:::`
  const result = transformMarkdown(source, { deckId: 'deck-1', joinUrl: 'https://slides.example/j/ABC-123' })

  assert.match(result, /cloud-height="380px"/)
  assert.match(result, /:cloud-spacing="0\.2"/)
  assert.match(interactionComponent, /import VueWordCloud from 'vuewordcloud'/)
  assert.match(interactionComponent, /:spacing="cloudSpacing"/)
  assert.match(interactionComponent, /weight === 1 \? ' person' : ' people'/)
  assert.match(interactionComponent, /result\.submission_count/)
  assert.match(interactionComponent, /people === 1 \? 'person' : 'people'/)
})

test('turns structured inputs into dedicated live renderers without duplicate option text', () => {
  const source = `# Structured

:::interact{type="allocation" id="budget" total="100"}
# Allocate the budget
- [quality] Quality
- [speed] Speed
:::

:::interact{type="image-choice" id="visual"}
# Choose a visual
- [calm] Calm {image="https://example.com/calm.png"}
- [bold] Bold {image="https://example.com/bold.png"}
:::
`
  const result = transformMarkdown(source, { deckId: 'deck-1', joinUrl: 'https://slides.example/j/ABC-123' })

  assert.match(result, /kind="allocation"/)
  assert.match(result, /kind="image-choice"/)
  assert.doesNotMatch(result, /\[quality\]/)
  assert.doesNotMatch(result, /\{image=/)
})

test('transforms audience ordering and image hotspots into live renderers', () => {
  const source = `# Parity

:::interact{type="ranking" id="priority-order" results="after-vote"}
# Order the priorities
- [quality] Quality
- [speed] Speed
- [learning] Learning
:::

:::interact{type="image-hotspot" id="focus-map" image="https://example.com/map.png" alt="Office map"}
# Place a pin
:::
`
  const result = transformMarkdown(source, { deckId: 'deck-1', joinUrl: 'https://slides.example/j/ABC-123' })
  assert.match(result, /kind="ranking"/)
  assert.match(result, /kind="image-hotspot"/)
  assert.match(result, /options-json="\[\{&quot;id&quot;:&quot;quality&quot;/)
  assert.doesNotMatch(result, /- Quality/)
})

test('transforms a mixed survey without duplicating its question definitions', () => {
  const source = `# Pulse

:::interact{type="survey" id="team-pulse" results="presenter"}
# Quick pulse
- [confidence] How confident are you? {type="rating" min="1" max="5"}
- [priority] Which priority matters? {type="choice" options="quality:Quality|speed:Speed"}
:::
`
  const result = transformMarkdown(source, { deckId: 'deck-1', joinUrl: 'https://slides.example/j/ABC-123' })
  assert.match(result, /kind="survey"/)
  assert.match(result, /# Quick pulse/)
  assert.doesNotMatch(result, /- \[confidence\]/)
  assert.doesNotMatch(result, /How confident are you\? \{type=/)
})

test('transforms the Amazing Sea Creatures sample without leaving interaction directives behind', async () => {
  const source = await readFile(new URL('../../../samples/amazing-sea-creatures.md', import.meta.url), 'utf8')
  const result = transformMarkdown(source, {
    deckId: 'sea-creatures-deck',
    joinUrl: 'https://slides.example.org/j/ABC-123',
  })

  assert.equal((result.match(/<InterdeckElementVote/g) || []).length, 3)
  assert.equal((result.match(/<InterdeckInteraction/g) || []).length, 7)
  assert.match(result, /kind="ranked-list"/)
  assert.match(result, /kind="survey"/)
  assert.match(result, /<InterdeckLiveSummary/)
  assert.match(result, /<InterdeckChart/)
  assert.match(result, /<AudienceQR/)
  assert.doesNotMatch(result, /:::interact/)
  assert.doesNotMatch(result, /\{interact=/)
  assert.doesNotMatch(result, /interdeck-slide:/)
})

test('transforms an interaction-first ranked list into one live slide component', () => {
  const source = `# Priorities

:::interact{type="ranked-list" id="priority-rank" display="slide" reveal="click" results="on-close"}
# Rank our priorities
- [quality] Quality
- [speed] Speed
- [learning] Learning
:::
`
  const result = transformMarkdown(source, { deckId: 'deck-1', joinUrl: 'https://slides.example/j/ABC-123' })

  assert.match(result, /kind="ranked-list"/)
  assert.match(result, /display="slide"/)
  assert.match(result, /reveal="click"/)
  assert.match(result, /options-json="\[\{&quot;id&quot;:&quot;quality&quot;/)
  assert.match(result, /# Rank our priorities/)
  assert.doesNotMatch(result, /\[quality\]/)
  assert.doesNotMatch(result, /- Quality/)
})
