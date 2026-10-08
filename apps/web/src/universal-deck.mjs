import { parseSync } from '@slidev/parser'
import DOMPurify from 'dompurify'
import MarkdownIt from 'markdown-it'
import { chartRevealClicks, parseChartSpec, parseEChartsSpec } from './chart-spec.mjs'

const markdown = new MarkdownIt({ html: true, linkify: true, breaks: false, typographer: true })
const attributePattern = /([a-zA-Z][a-zA-Z0-9_-]*)=(?:"([^"]*)"|'([^']*)'|([^\s]+))/g
const interactionPattern = /:::interact\{([^}]*)\}\s*([\s\S]*?)\s*:::/g
const chartPattern = /:::chart\{([^}]*)\}\s*([\s\S]*?)\s*:::/g
const echartsPattern = /:::echarts(?:\{([^}]*)\})?\s*([\s\S]*?)\s*:::/g
const inlineChartPattern = /^::chart\{([^}]*)\}\s*$/gm
const optionPattern = /^\s*-\s*\[([a-zA-Z0-9][a-zA-Z0-9_-]*)\]\s+(.+?)\s*$/gm
const elementPattern = /^(\s*(?:[-*+]\s+)?)(.+?)\s+\{([^}\n]*interact=(?:"[^"]+"|'[^']+'|[^\s}]+)[^}\n]*)\}\s*$/gm
const stylePattern = /<style\b([^>]*)>([\s\S]*?)<\/style>/gi

export function parseUniversalDeck(source) {
  const parsed = parseSync(String(source || ''), 'slides.md')
  const styles = []
  const scopedStyles = []
  const slides = parsed.slides.map((slide, index) => {
    const content = String(slide.content || '').replace(stylePattern, (_match, attributes, css) => {
      if (/\bscoped\b/i.test(attributes)) scopedStyles.push({ slideIndex: index, css })
      else styles.push(css)
      return ''
    })
    return {
      index,
      title: extractSlideTitle(content, index),
      frontmatter: slide.frontmatter || {},
      layout: String(slide.frontmatter?.layout || 'default'),
      classes: normalizeClasses(slide.frontmatter?.class),
      background: slide.frontmatter?.background || '',
      transition: String(slide.frontmatter?.transition || parsed.slides[0]?.frontmatter?.transition || 'slide-left'),
      slots: splitSlots(content),
    }
  })
  return {
    theme: String(parsed.slides[0]?.frontmatter?.theme || 'default').replace('@slidev/theme-', ''),
    title: String(parsed.slides[0]?.frontmatter?.title || ''),
    styles: styles.join('\n'),
    scopedStyles,
    slides,
  }
}

function extractSlideTitle(content, index) {
  const heading = String(content || '').match(/^\s*#{1,3}\s+(.+?)\s*$/m)?.[1] || ''
  const plain = heading
    .replace(/<[^>]+>/g, '')
    .replace(/[`*_~]/g, '')
    .trim()
  return plain || `Slide ${index + 1}`
}

export function renderUniversalSlide(slide, definitions, liveInteractions, clickStep) {
  const byId = new Map((definitions || []).map(item => [item.id, item]))
  const liveById = new Map((liveInteractions || []).map(item => [item.id, item]))
  const clickState = { next: 1, visibleElementInteractionIds: new Set() }
  const slots = {}
  for (const [name, source] of Object.entries(slide.slots || {}))
    slots[name] = renderSlot(source, byId, liveById, clickStep, clickState)
  return {
    slots,
    clicksTotal: clickState.next - 1,
    visibleElementInteractionIds: [...clickState.visibleElementInteractionIds],
  }
}

function renderSlot(source, definitions, liveInteractions, clickStep, clickState) {
  const blocks = new Map()
  let blockNumber = 0
  const token = (kind, value = {}) => {
    const id = `block-${++blockNumber}`
    blocks.set(id, { kind, ...value })
    return `\n\n<div data-interdeck-token="${id}"></div>\n\n`
  }

  let prepared = String(source || '').replace(/^<!--\s*interdeck-slide:[^>]+-->\s*/gm, '')
  prepared = prepared.replace(echartsPattern, (_match, rawAttributes = '', body) => {
    try {
      const spec = parseEChartsSpec(rawAttributes, body)
      const revealClicks = chartRevealClicks(spec)
      const clickStart = revealClicks ? clickState.next : 0
      clickState.next += revealClicks
      return token('chart', { spec, definition: definitions.get(spec.source), live: liveInteractions.get(spec.source), clickStart })
    } catch (error) {
      return `\n\n> Invalid ECharts option: ${escapeHtml(error instanceof Error ? error.message : String(error))}\n\n`
    }
  })
  prepared = prepared.replace(chartPattern, (_match, rawAttributes, body) => {
    try {
      const spec = parseChartSpec(rawAttributes, body)
      return token('chart', { spec, definition: definitions.get(spec.source), live: liveInteractions.get(spec.source) })
    } catch (error) {
      return `\n\n> Invalid Interdeck chart: ${escapeHtml(error instanceof Error ? error.message : String(error))}\n\n`
    }
  })
  prepared = prepared.replace(inlineChartPattern, (_match, rawAttributes) => {
    try {
      const spec = parseChartSpec(rawAttributes, '')
      return token('chart', { spec, definition: definitions.get(spec.source), live: liveInteractions.get(spec.source) })
    } catch (error) {
      return `\n\n> Invalid Interdeck chart: ${escapeHtml(error instanceof Error ? error.message : String(error))}\n\n`
    }
  })
  prepared = prepared.replace(interactionPattern, (_match, rawAttributes, body) => {
    const attributes = parseAttributes(rawAttributes)
    const definition = definitions.get(attributes.id)
    const kind = attributes.type || definition?.kind || 'poll'
    let readable = ['poll', 'quiz', 'reaction', 'ranked-list', 'image-choice', 'allocation', 'ranking', 'survey'].includes(kind)
      ? body.replace(optionPattern, '').trim()
      : body.replace(optionPattern, '- $2').trim()
    if (attributes['show-title'] === 'true')
      readable = readable.replace(/^\s*#{1,6}\s+.+(?:\r?\n|$)/, '').trim()
    const clickStart = kind === 'ranked-list' && attributes.reveal === 'click' ? clickState.next : 0
    if (clickStart) clickState.next += Math.max(0, definition?.options?.length || 0)
    const resolvedDefinition = definition
      ? { ...definition, config: { ...(definition.config || {}), ...attributes } }
      : { id: attributes.id, kind, title: '', options: [], config: attributes }
    return token('interaction', {
      definition: resolvedDefinition,
      live: liveInteractions.get(attributes.id),
      display: attributes.display || 'card',
      reveal: attributes.reveal || 'all',
      clickStart,
      bodyHtml: cleanHtml(markdown.render(readable)),
    })
  })
  prepared = prepared.replace(/::audience-qr(?:\{([^}]*)\})?/g, (_match, rawAttributes = '') => {
    const attributes = parseAttributes(rawAttributes)
    const size = /^\d{2,4}$/.test(attributes.size || '') ? Number(attributes.size) : 180
    return token('qr', { size })
  })
  prepared = prepared.replace(/::audience-count(?:\{([^}]*)\})?/g, (_match, rawAttributes = '') => {
    const attributes = parseAttributes(rawAttributes)
    return token('audience-count', { label: attributes.label?.trim() || 'people joined' })
  })
  prepared = prepared.replace(/::live-qa(?:\{([^}]*)\})?/g, (_match, rawAttributes = '') => {
    const attributes = parseAttributes(rawAttributes)
    return token('live-qa', {
      max: /^\d{1,2}$/.test(attributes.max || '') ? Number(attributes.max) : 8,
      show: attributes.show === 'all' ? 'all' : 'open',
    })
  })
  prepared = prepared.replace(/::live-summary(?:\{([^}]*)\})?/g, (_match, rawAttributes = '') => {
    const attributes = parseAttributes(rawAttributes)
    return token('live-summary', {
      show: attributes.show === 'all' ? 'all' : 'responded',
    })
  })
  prepared = prepared.replace(elementPattern, (_match, prefix, label, rawAttributes) => {
    const attributes = parseAttributes(rawAttributes)
    const definition = definitions.get(attributes.id)
    const live = liveInteractions.get(attributes.id)
    return `${prefix}${label} <span data-interdeck-element-id="${escapeHtml(attributes.id || '')}">${elementResultHtml(attributes.interact || definition?.kind || 'updown', live, definition?.options || [])}</span>`
  })

  prepared = markClickSyntax(prepared)
    .replace(/<\/?(?:Transform|v-after|v-click-hide)\b[^>]*>/gi, '')

  let html = markdown.render(prepared)
  html = applyClickGroups(html, clickStep, clickState)
  html = cleanHtml(html)
  for (const id of collectVisibleElementInteractionIds(html))
    clickState.visibleElementInteractionIds.add(id)
  return splitBlocks(html, blocks)
}

export function collectVisibleElementInteractionIds(html) {
  const visible = new Set()
  const stack = []
  const voidElements = new Set(['area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'param', 'source', 'track', 'wbr'])
  const tagPattern = /<(\/)?([a-zA-Z][a-zA-Z0-9-]*)([^>]*)>/g
  for (const match of String(html || '').matchAll(tagPattern)) {
    const closing = Boolean(match[1])
    const name = match[2].toLowerCase()
    const attributes = match[3] || ''
    if (closing) {
      for (let index = stack.length - 1; index >= 0; index -= 1) {
        const opened = stack.pop()
        if (opened.name === name) break
      }
      continue
    }
    const parentHidden = stack.at(-1)?.hidden || false
    const clickHidden = /\bdata-click(?:=|\s|>)/.test(attributes) && !/\bdata-click-visible(?:=|\s|>)/.test(attributes)
    const hidden = parentHidden || clickHidden
    const interactionId = attributes.match(/\bdata-interdeck-element-id="([^"]+)"/)?.[1]
    if (interactionId && !hidden) visible.add(interactionId)
    const selfClosing = /\/\s*$/.test(attributes) || voidElements.has(name)
    if (!selfClosing) stack.push({ name, hidden })
  }
  return visible
}

export function applyClickGroups(html, clickStep, state) {
  return renderClickRegion(html, false, clickStep, state)
}

export function markClickSyntax(source) {
  const tokenPattern = /<!--[\s\S]*?-->|<\/?[a-zA-Z][^>]*>/g
  const directivePattern = /\s+v-click(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?(?=\s|\/?>)/i
  const voidElements = new Set(['area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'param', 'source', 'track', 'wbr'])
  const stack = []
  let clickGroup = 0
  let cursor = 0
  let rendered = ''

  const marker = (kind, boundary, id) => `<div data-click-${kind}-${boundary}="${id}"></div>`
  for (const match of String(source || '').matchAll(tokenPattern)) {
    rendered += source.slice(cursor, match.index)
    cursor = (match.index || 0) + match[0].length
    const token = match[0]
    if (token.startsWith('<!--')) {
      rendered += token
      continue
    }

    const closing = token.match(/^<\/\s*([a-zA-Z][a-zA-Z0-9-]*)/)
    if (closing) {
      const tag = closing[1].toLowerCase()
      const entry = stack.pop()
      if (!entry || entry.tag !== tag) {
        rendered += token
        continue
      }
      if (!entry.omit) rendered += token
      if (entry.clickKind) rendered += marker(entry.clickKind, 'end', entry.clickId)
      continue
    }

    const opening = token.match(/^<\s*([a-zA-Z][a-zA-Z0-9-]*)/)
    if (!opening) {
      rendered += token
      continue
    }
    const tag = opening[1].toLowerCase()
    const selfClosing = /\/\s*>$/.test(token) || voidElements.has(tag)
    const wrapperKind = tag === 'v-clicks' ? 'group' : tag === 'v-click' ? 'single' : ''
    const hasDirective = !wrapperKind && directivePattern.test(token)
    const clickKind = wrapperKind || (hasDirective ? 'single' : '')
    const clickId = clickKind ? ++clickGroup : 0

    if (clickKind) rendered += marker(clickKind, 'start', clickId)
    if (!wrapperKind) rendered += hasDirective ? token.replace(directivePattern, '') : token
    if (selfClosing) {
      if (clickKind) rendered += marker(clickKind, 'end', clickId)
    }
    else stack.push({ tag, omit: Boolean(wrapperKind), clickKind, clickId })
  }
  rendered += source.slice(cursor)
  return rendered
}

function renderClickRegion(source, revealListItems, clickStep, state) {
  const startPattern = /<div data-click-(group|single)-start="(\d+)"><\/div>/g
  const listPattern = /<li(\s|>)/g
  let cursor = 0
  let rendered = ''

  while (cursor < source.length) {
    startPattern.lastIndex = cursor
    const start = startPattern.exec(source)
    listPattern.lastIndex = cursor
    const listItem = revealListItems ? listPattern.exec(source) : null

    if (listItem && (!start || (listItem.index || 0) < (start.index || 0))) {
      const click = state.next++
      rendered += source.slice(cursor, listItem.index)
      rendered += `<li data-click="${click}"${click <= clickStep ? ' data-click-visible="true"' : ''}${listItem[1]}`
      cursor = (listItem.index || 0) + listItem[0].length
      continue
    }

    if (!start) {
      rendered += source.slice(cursor)
      break
    }

    const kind = start[1]
    const id = start[2]
    const endToken = `<div data-click-${kind}-end="${id}"></div>`
    const bodyStart = (start.index || 0) + start[0].length
    const end = source.indexOf(endToken, bodyStart)
    if (end < 0) {
      rendered += source.slice(cursor)
      break
    }

    rendered += source.slice(cursor, start.index)
    const body = source.slice(bodyStart, end)
    if (kind === 'single') {
      const click = state.next++
      rendered += `<div data-click="${click}"${click <= clickStep ? ' data-click-visible="true"' : ''}>${renderClickRegion(body, false, clickStep, state)}</div>`
    }
    else rendered += renderClickRegion(body, true, clickStep, state)
    cursor = end + endToken.length
  }

  return rendered
}

function splitBlocks(html, blocks) {
  const segments = []
  const pattern = /<div data-interdeck-token="([^"]+)"><\/div>/g
  let cursor = 0
  for (const match of html.matchAll(pattern)) {
    if (match.index > cursor) segments.push({ kind: 'html', html: html.slice(cursor, match.index) })
    const block = blocks.get(match[1])
    if (block) segments.push(block)
    cursor = match.index + match[0].length
  }
  if (cursor < html.length) segments.push({ kind: 'html', html: html.slice(cursor) })
  return segments
}

function splitSlots(content) {
  const slots = { default: '' }
  let active = 'default'
  for (const line of String(content || '').split('\n')) {
    const marker = line.match(/^::([a-zA-Z][a-zA-Z0-9-]*)::\s*$/)
    if (marker) {
      active = marker[1]
      slots[active] ||= ''
    }
    else {
      slots[active] += `${line}\n`
    }
  }
  return slots
}

function parseAttributes(source) {
  const attributes = {}
  for (const match of String(source || '').matchAll(attributePattern))
    attributes[match[1]] = match[2] ?? match[3] ?? match[4]
  return attributes
}

export function elementResultHtml(kind, interaction, definitionOptions = []) {
  const result = interaction?.result || {}
  if (kind === 'rating') {
    const rating = result.average == null ? '—' : Number(result.average).toFixed(1)
    return `<span class="universal-element-vote rating">★ ${rating}</span>`
  }
  if (kind === 'reaction') {
    const options = interaction?.options?.length ? interaction.options : definitionOptions
    const counts = result.counts || {}
    const visibleOptions = options.filter(option => Number(counts[option.id] || 0) > 0)
    if (!visibleOptions.length) return ''
    return `<span class="universal-element-reactions">${visibleOptions.map(option => (
      `<span title="${escapeHtml(option.label)}: ${Number(counts[option.id] || 0)}"><b>${escapeHtml(option.label)}</b> ${Number(counts[option.id] || 0)}</span>`
    )).join('')}</span>`
  }
  const up = Number(result.up || 0)
  const down = Number(result.down || 0)
  return `<span class="universal-element-votes"><span class="up">▲ ${up}</span><span class="down">▼ ${down}</span></span>`
}

function escapeHtml(value) {
  return String(value ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;')
}

function normalizeClasses(value) {
  if (Array.isArray(value)) return value.map(String).join(' ')
  return String(value || '')
    .split(/\s+/)
    .filter(token => /^[a-zA-Z0-9_:\-/\[\].]+$/.test(token))
    .join(' ')
}

function cleanHtml(html) {
  // The browser export exposes a configured DOMPurify instance. Its Node
  // entrypoint is a factory without `sanitize`, which is used only by the
  // parser's DOM-free structural tests.
  if (typeof DOMPurify?.sanitize !== 'function') return html
  return DOMPurify.sanitize(html, {
    ADD_ATTR: ['target', 'data-click', 'data-click-visible', 'data-interdeck-token', 'data-interdeck-element-id', 'data-click-group-start', 'data-click-group-end', 'data-click-single-start', 'data-click-single-end'],
  })
}
