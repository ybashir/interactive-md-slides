<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { ApiError, api, patch } from '../api'
import UniversalSlideSlot from '../components/player/UniversalSlideSlot.vue'
import { parseUniversalDeck, renderUniversalSlide } from '../universal-deck.mjs'
import type { DeckDetail, LiveState } from '../types'

const route = useRoute()
const router = useRouter()
const deckId = route.params.id as string
const deck = ref<DeckDetail | null>(null)
const state = ref<LiveState | null>(null)
const loading = ref(true)
const error = ref('')
const slideNumber = ref(Math.max(1, Number(route.query.slide || 1)))
const clickStep = ref(0)
const viewport = ref<HTMLElement | null>(null)
const scale = ref(1)
const overviewOpen = ref(false)
const fullscreenActive = ref(false)
const shortcutsOpen = ref(false)
const drawingToolsOpen = ref(false)
const drawingMode = ref<DrawingMode>('off')
const drawingColor = ref('#ef4444')
const drawingSize = ref(4)
const drawingLayer = ref<SVGSVGElement | null>(null)
const drawings = ref<Record<number, DrawingShape[]>>({})
const activeDrawing = ref<DrawingShape | null>(null)
const laserEnabled = ref(false)
const laserPoint = ref({ x: 0, y: 0, visible: false })
const recording = ref(false)
const recordingError = ref('')
const qaActionError = ref('')
const answeringQuestionIds = ref<string[]>([])
let liveTimer: number | undefined
let resizeObserver: ResizeObserver | undefined
let customStyle: HTMLStyleElement | undefined
let recordingStream: MediaStream | undefined
let mediaRecorder: MediaRecorder | undefined
let recordingChunks: Blob[] = []

type DrawingMode = 'off' | 'pen' | 'line' | 'arrow' | 'rectangle' | 'ellipse' | 'eraser'
type DrawingPoint = { x: number, y: number }
type DrawingShape = {
  id: string
  mode: Exclude<DrawingMode, 'off' | 'eraser'>
  color: string
  size: number
  start: DrawingPoint
  end: DrawingPoint
  points: DrawingPoint[]
}
const drawingModes: Array<{ mode: DrawingMode, label: string, icon: string }> = [
  { mode: 'pen', label: 'Freehand', icon: '✎' },
  { mode: 'line', label: 'Line', icon: '╱' },
  { mode: 'arrow', label: 'Arrow', icon: '↗' },
  { mode: 'rectangle', label: 'Rectangle', icon: '□' },
  { mode: 'ellipse', label: 'Ellipse', icon: '○' },
  { mode: 'eraser', label: 'Erase', icon: '⌫' },
]

const parsed = computed(() => deck.value ? parseUniversalDeck(deck.value.markdown) : null)
const slide = computed(() => parsed.value?.slides[slideNumber.value - 1])
const rendered = computed(() => slide.value && deck.value
  ? renderUniversalSlide(slide.value, deck.value.interactions, state.value?.interactions || [], clickStep.value)
  : { slots: {}, clicksTotal: 0, visibleElementInteractionIds: [] })
const joinUrl = computed(() => deck.value ? `${window.location.origin}/j/${deck.value.join_code}` : '')
const presenterControls = computed(() => route.query.embedded === 'true')
const canvasStyle = computed(() => ({ transform: `translate(-50%, -50%) scale(${scale.value})` }))
const slideStyle = computed(() => {
  const background = String(slide.value?.background || '').trim()
  if (!background) return {}
  if (/^(?:#|rgb|hsl|linear-gradient|radial-gradient|var\()/i.test(background)) return { background }
  return { backgroundImage: `url(${JSON.stringify(background)})`, backgroundSize: 'cover', backgroundPosition: 'center' }
})
const layoutClass = computed(() => [
  'slidev-layout',
  `layout-${slide.value?.layout || 'default'}`,
  slide.value?.layout || 'default',
  slide.value?.classes || '',
])
const transitionName = computed(() => ['fade', 'slide-left', 'slide-right'].includes(slide.value?.transition || '') ? slide.value?.transition : 'fade')
const allSegments = computed(() => Object.values(rendered.value.slots).flat())
const visibleDrawings = computed(() => [
  ...(drawings.value[slideNumber.value] || []),
  ...(activeDrawing.value ? [activeDrawing.value] : []),
])

onMounted(async () => {
  window.addEventListener('keydown', onKeydown)
  window.addEventListener('message', onCommand)
  document.addEventListener('fullscreenchange', onFullscreenChange)
  customStyle = document.createElement('style')
  customStyle.dataset.interdeckDeckStyle = deckId
  document.head.appendChild(customStyle)
  try {
    await reloadDeck()
    if (!deck.value) throw new Error('The deck could not be loaded')
    if (slideNumber.value > deck.value.slides.length) slideNumber.value = deck.value.slides.length || 1
    await refreshLiveState()
    // The editor preview receives source updates directly and has no audience
    // activity to follow. Only the embedded live presenter needs continuous
    // result refreshes; this avoids a permanent request-per-second tax while
    // somebody is simply editing a deck.
    if (route.query.embedded === 'true') liveTimer = window.setInterval(refreshLiveState, 1000)
    await nextTick()
    resizeObserver = new ResizeObserver(updateScale)
    if (viewport.value) resizeObserver.observe(viewport.value)
    updateScale()
    notifyReady()
  }
  catch (reason) {
    if (reason instanceof ApiError && reason.status === 401) return router.replace('/login')
    error.value = reason instanceof Error ? reason.message : 'The deck could not be loaded'
  }
  finally {
    loading.value = false
  }
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown)
  window.removeEventListener('message', onCommand)
  document.removeEventListener('fullscreenchange', onFullscreenChange)
  window.clearInterval(liveTimer)
  resizeObserver?.disconnect()
  customStyle?.remove()
  stopRecording(false)
})

watch([slideNumber, clickStep, () => rendered.value.clicksTotal], () => reportNavigation())
watch([() => deck.value?.css, () => parsed.value?.styles, () => parsed.value?.scopedStyles], syncCustomStyles)

async function refreshLiveState() {
  try { state.value = await api<LiveState>(`/api/decks/${deckId}/live-state`) }
  catch { /* The deck remains renderable when its live state is temporarily unavailable. */ }
}

async function markQuestionsAnswered(questionIds: string[]) {
  const ids = [...new Set(questionIds)].filter(Boolean)
  if (!presenterControls.value || !ids.length || answeringQuestionIds.value.length) return
  qaActionError.value = ''
  answeringQuestionIds.value = ids
  try {
    await Promise.all(ids.map(questionId => patch(`/api/decks/${deckId}/questions/${questionId}`, { lifecycle_status: 'answered' })))
    await refreshLiveState()
  }
  catch (reason) {
    qaActionError.value = reason instanceof Error ? reason.message : 'The question could not be marked answered.'
    await refreshLiveState()
  }
  finally {
    answeringQuestionIds.value = []
  }
}

async function reloadDeck() {
  const next = await api<DeckDetail>(`/api/decks/${deckId}`)
  await applyDeck(next)
}

async function applyDeck(next: DeckDetail) {
  if (next.id !== deckId) throw new Error('The updated deck did not match this player')
  const previousSlideKey = deck.value?.slides[slideNumber.value - 1]?.key
  const previousClicksTotal = rendered.value.clicksTotal
  const wasAtEndOfSlide = deck.value !== null && clickStep.value >= previousClicksTotal
  deck.value = next
  error.value = ''
  const preservedSlideIndex = previousSlideKey
    ? next.slides.findIndex(candidate => candidate.key === previousSlideKey)
    : -1
  slideNumber.value = preservedSlideIndex >= 0
    ? preservedSlideIndex + 1
    : clamp(slideNumber.value, 1, next.slides.length || 1)
  syncCustomStyles()
  await nextTick()
  clickStep.value = wasAtEndOfSlide
    ? rendered.value.clicksTotal
    : clamp(clickStep.value, 0, rendered.value.clicksTotal)
}

function syncCustomStyles() {
  if (!customStyle) return
  const scoped = (parsed.value?.scopedStyles || [])
    .map(style => scopeSlideCss(style.css, `.slidev-layout[data-interdeck-slide="${style.slideIndex}"]`))
    .join('\n')
  customStyle.textContent = `${deck.value?.css || ''}\n${parsed.value?.styles || ''}\n${scoped}`
}

function scopeSlideCss(css: string, scope: string) {
  try {
    const sheet = new CSSStyleSheet()
    sheet.replaceSync(css)
    scopeCssRules(sheet.cssRules, scope)
    return Array.from(sheet.cssRules, rule => rule.cssText).join('\n')
  }
  catch {
    // Keep malformed author CSS visible to browser diagnostics. Saving and the
    // rest of the deck must remain usable even if one style block is invalid.
    return css
  }
}

function scopeCssRules(rules: CSSRuleList, scope: string) {
  for (const rule of Array.from(rules)) {
    if ('selectorText' in rule && typeof rule.selectorText === 'string') {
      const selectors = splitSelectorList(rule.selectorText)
        .map(selector => selector === ':root' ? scope : `${scope} ${selector}`)
      try { rule.selectorText = selectors.join(', ') }
      catch { /* Leave an unsupported selector unchanged instead of losing the rule. */ }
    }
    else if ('cssRules' in rule) {
      const nested = (rule as CSSGroupingRule).cssRules
      if (nested) scopeCssRules(nested, scope)
    }
  }
}

function splitSelectorList(source: string) {
  const selectors: string[] = []
  let current = ''
  let depth = 0
  let quote = ''
  let escaped = false
  for (const character of source) {
    if (escaped) {
      current += character
      escaped = false
    }
    else if (character === '\\') {
      current += character
      escaped = true
    }
    else if (quote) {
      current += character
      if (character === quote) quote = ''
    }
    else if (character === '"' || character === "'") {
      current += character
      quote = character
    }
    else if (character === '(' || character === '[') {
      current += character
      depth += 1
    }
    else if (character === ')' || character === ']') {
      current += character
      depth = Math.max(0, depth - 1)
    }
    else if (character === ',' && depth === 0) {
      if (current.trim()) selectors.push(current.trim())
      current = ''
    }
    else current += character
  }
  if (current.trim()) selectors.push(current.trim())
  return selectors
}

function updateScale() {
  const width = viewport.value?.clientWidth || window.innerWidth
  const height = viewport.value?.clientHeight || window.innerHeight
  scale.value = Math.max(.1, Math.min(width / 980, height / 551))
}

function notifyReady() {
  if (window.parent !== window)
    window.parent.postMessage({ type: 'interdeck:ready' }, window.location.origin)
}

function reportNavigation(commandId = '') {
  if (window.parent === window) return
  window.parent.postMessage({
    type: 'interdeck:navigation',
    slideNumber: slideNumber.value,
    clickStep: clickStep.value,
    clicksTotal: rendered.value.clicksTotal,
    visibleElementInteractionIds: rendered.value.visibleElementInteractionIds,
    commandId,
  }, window.location.origin)
}

function onCommand(event: MessageEvent) {
  if (event.origin !== window.location.origin || event.source !== window.parent) return
  const message = event.data
  if (!message) return
  if (message.type === 'interdeck:reload-source') {
    const update = message.deck && typeof message.deck === 'object'
      ? applyDeck(message.deck as DeckDetail)
      : reloadDeck()
    void update.catch(reason => {
      error.value = reason instanceof Error ? reason.message : 'The updated deck could not be loaded'
    })
    return
  }
  if (message.type !== 'interdeck:command') return
  if (message.action === 'next') next()
  else if (message.action === 'prev') previous()
  else if (message.action === 'go') {
    const requestedSlideKey = typeof message.slideKey === 'string' ? message.slideKey : ''
    const keyedSlideIndex = requestedSlideKey
      ? (deck.value?.slides.findIndex(slide => slide.key === requestedSlideKey) ?? -1)
      : -1
    const requestedSlideNumber = keyedSlideIndex >= 0 ? keyedSlideIndex + 1 : Number(message.slideNumber) || 1
    slideNumber.value = clamp(requestedSlideNumber, 1, parsed.value?.slides.length || 1)
    clickStep.value = clamp(Number(message.clickStep) || 0, 0, rendered.value.clicksTotal)
  }
  reportNavigation(message.commandId)
}

function next() {
  if (clickStep.value < rendered.value.clicksTotal) clickStep.value += 1
  else if (slideNumber.value < (parsed.value?.slides.length || 1)) {
    slideNumber.value += 1
    clickStep.value = 0
  }
}

function previous() {
  if (clickStep.value > 0) clickStep.value -= 1
  else if (slideNumber.value > 1) {
    slideNumber.value -= 1
    nextTick(() => { clickStep.value = rendered.value.clicksTotal })
  }
}

function onKeydown(event: KeyboardEvent) {
  const target = event.target as HTMLElement | null
  if (target?.closest('.qa-scroll') && ['ArrowDown', 'ArrowUp', 'PageDown', 'PageUp', 'Home', 'End'].includes(event.key)) return
  if (target?.closest('button, input, textarea, select')) return
  if (['ArrowRight', 'PageDown', ' '].includes(event.key)) {
    event.preventDefault()
    next()
  }
  else if (['ArrowLeft', 'PageUp'].includes(event.key)) {
    event.preventDefault()
    previous()
  }
  else if (event.key.toLowerCase() === 'f') void toggleFullscreen()
  else if (event.key.toLowerCase() === 'o') overviewOpen.value = !overviewOpen.value
  else if (event.key.toLowerCase() === 'd') toggleDrawingTools()
  else if (event.key.toLowerCase() === 'l') toggleLaser()
  else if (event.key.toLowerCase() === 'r') void toggleRecording()
  else if (event.key.toLowerCase() === 'c' && route.query.embedded === 'true') {
    window.parent.postMessage({ type: 'interdeck:open-presenter-console' }, window.location.origin)
  }
  else if (event.key === '?') shortcutsOpen.value = !shortcutsOpen.value
  else if (event.key === 'Escape') closePlayerPanels()
}

function goToSlide(number: number) {
  slideNumber.value = clamp(number, 1, parsed.value?.slides.length || 1)
  clickStep.value = 0
  overviewOpen.value = false
}

async function toggleFullscreen() {
  if (document.fullscreenElement) await document.exitFullscreen()
  else await document.documentElement.requestFullscreen()
}

function onFullscreenChange() {
  fullscreenActive.value = !!document.fullscreenElement
  nextTick(updateScale)
}

function blurToolbarOnLeave(event: MouseEvent) {
  const toolbar = event.currentTarget as HTMLElement | null
  const active = document.activeElement as HTMLElement | null
  if (toolbar && active && toolbar.contains(active)) active.blur()
}

function releasePointerFocus(event: MouseEvent) {
  if (event.detail <= 0) return
  nextTick(() => {
    const active = document.activeElement as HTMLElement | null
    if (active?.closest('.player-navigation')) active.blur()
  })
}

function toggleDrawingTools() {
  drawingToolsOpen.value = !drawingToolsOpen.value
  drawingMode.value = drawingToolsOpen.value ? (drawingMode.value === 'off' ? 'pen' : drawingMode.value) : 'off'
  laserEnabled.value = false
  laserPoint.value.visible = false
}

function toggleLaser() {
  laserEnabled.value = !laserEnabled.value
  laserPoint.value.visible = false
  if (laserEnabled.value) {
    drawingToolsOpen.value = false
    drawingMode.value = 'off'
  }
}

function closePlayerPanels() {
  overviewOpen.value = false
  shortcutsOpen.value = false
  drawingToolsOpen.value = false
  drawingMode.value = 'off'
  laserEnabled.value = false
  laserPoint.value.visible = false
}

function pointerOnDrawingLayer(event: PointerEvent): DrawingPoint {
  const rect = drawingLayer.value?.getBoundingClientRect()
  if (!rect) return { x: 0, y: 0 }
  return {
    x: clamp((event.clientX - rect.left) * 980 / rect.width, 0, 980),
    y: clamp((event.clientY - rect.top) * 551 / rect.height, 0, 551),
  }
}

function beginDrawing(event: PointerEvent) {
  if (drawingMode.value === 'off') return
  event.preventDefault()
  drawingLayer.value?.setPointerCapture(event.pointerId)
  if (drawingMode.value === 'eraser') {
    const id = (event.target as Element | null)?.closest('[data-drawing-id]')?.getAttribute('data-drawing-id')
    if (id) drawings.value[slideNumber.value] = currentDrawings().filter(shape => shape.id !== id)
    return
  }
  const point = pointerOnDrawingLayer(event)
  activeDrawing.value = {
    id: crypto.randomUUID(),
    mode: drawingMode.value,
    color: drawingColor.value,
    size: drawingSize.value,
    start: point,
    end: point,
    points: [point],
  }
}

function continueDrawing(event: PointerEvent) {
  if (!activeDrawing.value) return
  const point = pointerOnDrawingLayer(event)
  activeDrawing.value.end = point
  if (activeDrawing.value.mode === 'pen') activeDrawing.value.points.push(point)
}

function finishDrawing(event: PointerEvent) {
  if (!activeDrawing.value) return
  continueDrawing(event)
  drawings.value[slideNumber.value] = [...currentDrawings(), activeDrawing.value]
  activeDrawing.value = null
}

function currentDrawings() {
  return drawings.value[slideNumber.value] || []
}

function points(shape: DrawingShape) {
  return shape.points.map(point => `${point.x},${point.y}`).join(' ')
}

function ellipse(shape: DrawingShape) {
  return {
    cx: (shape.start.x + shape.end.x) / 2,
    cy: (shape.start.y + shape.end.y) / 2,
    rx: Math.abs(shape.end.x - shape.start.x) / 2,
    ry: Math.abs(shape.end.y - shape.start.y) / 2,
  }
}

function undoDrawing() {
  drawings.value[slideNumber.value] = currentDrawings().slice(0, -1)
}

function clearDrawings() {
  drawings.value[slideNumber.value] = []
}

function moveLaser(event: PointerEvent) {
  if (!laserEnabled.value) return
  laserPoint.value = { x: event.clientX, y: event.clientY, visible: true }
}

async function toggleRecording() {
  recordingError.value = ''
  if (recording.value) {
    stopRecording(true)
    return
  }
  try {
    if (!navigator.mediaDevices?.getDisplayMedia || typeof MediaRecorder === 'undefined')
      throw new Error('Screen recording is not supported by this browser.')
    recordingStream = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: true })
    recordingChunks = []
    mediaRecorder = new MediaRecorder(recordingStream)
    mediaRecorder.addEventListener('dataavailable', event => {
      if (event.data.size) recordingChunks.push(event.data)
    })
    mediaRecorder.addEventListener('stop', downloadRecording, { once: true })
    recordingStream.getVideoTracks()[0]?.addEventListener('ended', () => stopRecording(true), { once: true })
    mediaRecorder.start(1000)
    recording.value = true
  }
  catch (reason) {
    recordingError.value = reason instanceof Error ? reason.message : 'Screen recording could not start.'
    stopRecording(false)
  }
}

function stopRecording(save: boolean) {
  if (mediaRecorder && mediaRecorder.state !== 'inactive') {
    if (!save) mediaRecorder.removeEventListener('stop', downloadRecording)
    mediaRecorder.stop()
  }
  recordingStream?.getTracks().forEach(track => track.stop())
  recordingStream = undefined
  mediaRecorder = undefined
  recording.value = false
  if (!save) recordingChunks = []
}

function downloadRecording() {
  if (!recordingChunks.length) return
  const blob = new Blob(recordingChunks, { type: mediaRecorder?.mimeType || 'video/webm' })
  const link = document.createElement('a')
  link.href = URL.createObjectURL(blob)
  link.download = `${parsed.value?.title || 'interdeck-presentation'}-${new Date().toISOString().slice(0, 19).replace(/:/g, '-')}.webm`
  link.click()
  window.setTimeout(() => URL.revokeObjectURL(link.href), 1000)
  recordingChunks = []
}

function clamp(value: number, minimum: number, maximum: number) {
  return Math.min(maximum, Math.max(minimum, value))
}
</script>

<template>
  <main ref="viewport" :class="['universal-player', `theme-${parsed?.theme || 'default'}`]" @pointermove="moveLaser" @pointerleave="laserPoint.visible = false">
    <div v-if="loading" class="player-message">Loading deck…</div>
    <div v-else-if="error" class="player-message error"><strong>Deck unavailable</strong><span>{{ error }}</span></div>
    <template v-else-if="deck && slide">
      <div class="universal-canvas" :style="canvasStyle">
        <Transition :name="transitionName" mode="out-in">
        <section :key="slideNumber" :class="layoutClass" :style="slideStyle" :data-interdeck-slide="slide.index">
          <template v-if="slide.layout === 'two-cols-header'">
            <header class="layout-header"><UniversalSlideSlot :segments="rendered.slots.default || []" :deck-id="deckId" :join-url="joinUrl" :state="state" :click-step="clickStep" :presenter-controls="presenterControls" :answering-question-ids="answeringQuestionIds" @answer-question="markQuestionsAnswered" /></header>
            <div class="layout-columns"><div><UniversalSlideSlot :segments="rendered.slots.left || []" :deck-id="deckId" :join-url="joinUrl" :state="state" :click-step="clickStep" :presenter-controls="presenterControls" :answering-question-ids="answeringQuestionIds" @answer-question="markQuestionsAnswered" /></div><div><UniversalSlideSlot :segments="rendered.slots.right || []" :deck-id="deckId" :join-url="joinUrl" :state="state" :click-step="clickStep" :presenter-controls="presenterControls" :answering-question-ids="answeringQuestionIds" @answer-question="markQuestionsAnswered" /></div></div>
          </template>
          <template v-else-if="slide.layout === 'two-cols'">
            <div class="layout-columns"><div><UniversalSlideSlot :segments="rendered.slots.left || rendered.slots.default || []" :deck-id="deckId" :join-url="joinUrl" :state="state" :click-step="clickStep" :presenter-controls="presenterControls" :answering-question-ids="answeringQuestionIds" @answer-question="markQuestionsAnswered" /></div><div><UniversalSlideSlot :segments="rendered.slots.right || []" :deck-id="deckId" :join-url="joinUrl" :state="state" :click-step="clickStep" :presenter-controls="presenterControls" :answering-question-ids="answeringQuestionIds" @answer-question="markQuestionsAnswered" /></div></div>
          </template>
          <UniversalSlideSlot v-else :segments="allSegments" :deck-id="deckId" :join-url="joinUrl" :state="state" :click-step="clickStep" :presenter-controls="presenterControls" :answering-question-ids="answeringQuestionIds" @answer-question="markQuestionsAnswered" />
        </section>
        </Transition>
        <svg
          ref="drawingLayer"
          :class="['player-drawing-layer', { active: drawingMode !== 'off' }]"
          viewBox="0 0 980 551"
          aria-label="Slide drawing layer"
          @pointerdown="beginDrawing"
          @pointermove="continueDrawing"
          @pointerup="finishDrawing"
          @pointercancel="finishDrawing"
        >
          <defs>
            <marker id="interdeck-arrowhead" markerWidth="5" markerHeight="5" refX="4" refY="2.5" orient="auto" markerUnits="strokeWidth">
              <path d="M0,0 L5,2.5 L0,5 Z" fill="context-stroke" />
            </marker>
          </defs>
          <template v-for="shape in visibleDrawings" :key="shape.id">
            <polyline v-if="shape.mode === 'pen'" :data-drawing-id="shape.id" :points="points(shape)" :stroke="shape.color" :stroke-width="shape.size" />
            <line v-else-if="shape.mode === 'line' || shape.mode === 'arrow'" :data-drawing-id="shape.id" :x1="shape.start.x" :y1="shape.start.y" :x2="shape.end.x" :y2="shape.end.y" :stroke="shape.color" :stroke-width="shape.size" :marker-end="shape.mode === 'arrow' ? 'url(#interdeck-arrowhead)' : undefined" />
            <rect v-else-if="shape.mode === 'rectangle'" :data-drawing-id="shape.id" :x="Math.min(shape.start.x, shape.end.x)" :y="Math.min(shape.start.y, shape.end.y)" :width="Math.abs(shape.end.x - shape.start.x)" :height="Math.abs(shape.end.y - shape.start.y)" :stroke="shape.color" :stroke-width="shape.size" />
            <ellipse v-else-if="shape.mode === 'ellipse'" :data-drawing-id="shape.id" v-bind="ellipse(shape)" :stroke="shape.color" :stroke-width="shape.size" />
          </template>
        </svg>
      </div>
      <i v-if="laserEnabled && laserPoint.visible" class="player-laser" :style="{ left: `${laserPoint.x}px`, top: `${laserPoint.y}px` }" />
      <div class="player-progress"><i :style="{ width: `${(slideNumber / (parsed?.slides.length || 1)) * 100}%` }" /></div>
      <i class="player-navigation-hotspot" aria-hidden="true" />
      <nav class="player-navigation" aria-label="Presentation controls" @click.capture="releasePointerFocus" @mouseleave="blurToolbarOnLeave">
        <button :title="fullscreenActive ? 'Exit fullscreen (F)' : 'Fullscreen (F)'" @click="toggleFullscreen">{{ fullscreenActive ? '×' : '⛶' }}</button>
        <button title="Previous (←)" :disabled="slideNumber === 1 && clickStep === 0" @click="previous">←</button>
        <button title="Next (→ or Space)" :disabled="slideNumber === parsed?.slides.length && clickStep === rendered.clicksTotal" @click="next">→</button>
        <button class="player-slide-number" title="Slide overview (O)" @click="overviewOpen = true">▦&nbsp; {{ slideNumber }} / {{ parsed?.slides.length }}</button>
        <span />
        <button :class="{ active: laserEnabled }" title="Laser pointer (L)" @click="toggleLaser">◉</button>
        <button :class="{ active: drawingToolsOpen }" title="Drawing tools (D)" @click="toggleDrawingTools">✎</button>
        <button :class="['record-button', { active: recording }]" :title="recording ? 'Stop and save recording (R)' : 'Record presentation (R)'" @click="toggleRecording">{{ recording ? '■' : '●' }}</button>
        <button title="Keyboard shortcuts (?)" @click="shortcutsOpen = true">?</button>
      </nav>
      <section v-if="drawingToolsOpen" class="player-drawing-tools" aria-label="Drawing tools">
        <div class="drawing-modes">
          <button v-for="item in drawingModes" :key="item.mode" :class="{ active: drawingMode === item.mode }" :title="item.label" @click="drawingMode = item.mode">{{ item.icon }}</button>
        </div>
        <div class="drawing-options">
          <button v-for="color in ['#ef4444', '#f59e0b', '#22c55e', '#38bdf8', '#8b5cf6', '#111827']" :key="color" class="drawing-color" :class="{ active: drawingColor === color }" :style="{ background: color }" :aria-label="`Use ${color}`" @click="drawingColor = color" />
          <input v-model.number="drawingSize" type="range" min="2" max="12" aria-label="Drawing thickness" />
          <button title="Undo last drawing" :disabled="!currentDrawings().length" @click="undoDrawing">Undo</button>
          <button title="Clear drawings on this slide" :disabled="!currentDrawings().length" @click="clearDrawings">Clear</button>
        </div>
      </section>
      <p v-if="recordingError" class="player-tool-error" role="alert">{{ recordingError }} <button aria-label="Dismiss" @click="recordingError = ''">×</button></p>
      <p v-if="qaActionError" class="player-tool-error qa-action-error" role="alert">{{ qaActionError }} <button aria-label="Dismiss" @click="qaActionError = ''">×</button></p>
      <div v-if="overviewOpen" class="player-overview" role="dialog" aria-modal="true" aria-label="Slide overview" @click.self="overviewOpen = false">
        <header><strong>Slides</strong><button aria-label="Close overview" @click="overviewOpen = false">×</button></header>
        <div>
          <button v-for="item in parsed?.slides" :key="item.index" :class="{ active: item.index + 1 === slideNumber }" @click="goToSlide(item.index + 1)">
            <b>{{ item.index + 1 }}</b><span>{{ item.title || `Slide ${item.index + 1}` }}</span>
          </button>
        </div>
      </div>
      <div v-if="shortcutsOpen" class="player-shortcuts" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts" @click.self="shortcutsOpen = false">
        <section>
          <header><strong>Presentation shortcuts</strong><button aria-label="Close shortcuts" @click="shortcutsOpen = false">×</button></header>
          <dl>
            <div><dt>Next</dt><dd><kbd>→</kbd> <kbd>Space</kbd></dd></div>
            <div><dt>Previous</dt><dd><kbd>←</kbd></dd></div>
            <div><dt>Slide overview</dt><dd><kbd>O</kbd></dd></div>
            <div><dt>Fullscreen</dt><dd><kbd>F</kbd></dd></div>
            <div><dt>Drawing tools</dt><dd><kbd>D</kbd></dd></div>
            <div><dt>Laser pointer</dt><dd><kbd>L</kbd></dd></div>
            <div><dt>Record screen</dt><dd><kbd>R</kbd></dd></div>
            <div><dt>Close tools</dt><dd><kbd>Esc</kbd></dd></div>
          </dl>
        </section>
      </div>
    </template>
  </main>
</template>

<style>
.universal-player { position: fixed; inset: 0; display: grid; place-items: center; color: #172033; background: #dedbd4; overflow: hidden; }
.universal-canvas { position: absolute; top: 50%; left: 50%; width: 980px; height: 551px; transform-origin: center; }
.universal-canvas .slidev-layout { box-sizing: border-box; width: 980px; height: 551px; padding: 42px 54px; color: #172033; background-color: #fff; font: 24px/1.45 Inter, ui-sans-serif, system-ui, sans-serif; overflow: hidden; }
.fade-enter-active, .fade-leave-active, .slide-left-enter-active, .slide-left-leave-active, .slide-right-enter-active, .slide-right-leave-active { position: absolute; inset: 0; transition: opacity .24s ease, transform .28s ease; }
.fade-enter-from, .fade-leave-to { opacity: 0; }
.slide-left-enter-from, .slide-right-leave-to { opacity: 0; transform: translateX(38px); }
.slide-left-leave-to, .slide-right-enter-from { opacity: 0; transform: translateX(-38px); }
.slidev-layout h1 { margin: 0 0 22px; font-size: 2.25em; font-weight: 760; line-height: 1.08; letter-spacing: -.035em; }
.slidev-layout h2 { margin: 0 0 16px; font-size: 1.45em; line-height: 1.16; }
.slidev-layout h3 { margin: 0 0 12px; font-size: 1.15em; }
.slidev-layout p { margin: 0 0 16px; }
.slidev-layout ul, .slidev-layout ol { margin: 12px 0; padding-left: 1.3em; }
.slidev-layout li { margin: .3em 0; }
.slidev-layout img { max-width: 100%; max-height: 410px; object-fit: contain; }
.slidev-layout pre { padding: 14px 17px; border-radius: 10px; color: #e9eef8; background: #171c28; font-size: .65em; line-height: 1.5; overflow: auto; }
.slidev-layout code { font-family: 'DM Mono', ui-monospace, monospace; }
.slidev-layout blockquote { margin: 18px 0; padding-left: 18px; border-left: 4px solid var(--slidev-theme-primary); opacity: .78; }
.slidev-layout table { width: 100%; border-collapse: collapse; font-size: .72em; }
.slidev-layout th, .slidev-layout td { padding: 8px 11px; border: 1px solid #d7dbe2; text-align: left; }
.universal-markdown { display: contents; }
.slidev-layout [data-click]:not([data-click-visible]) { visibility: hidden; opacity: 0; transform: translateY(8px); }
.slidev-layout [data-click][data-click-visible] { visibility: visible; opacity: 1; transform: none; transition: opacity .25s ease, transform .25s ease; }
.slidev-layout.text-center, .slidev-layout .text-center { text-align: center; }
.slidev-layout.text-center :is(img, svg, video, iframe), .slidev-layout .text-center :is(img, svg, video, iframe) { display: block; margin-right: auto; margin-left: auto; }
.slidev-layout.bg-white { background: white; }
.slidev-layout.layout-center, .slidev-layout.center, .slidev-layout.cover, .slidev-layout.intro, .slidev-layout.fact, .slidev-layout.statement, .slidev-layout.section { display: flex; flex-direction: column; justify-content: center; }
.slidev-layout.cover h1, .slidev-layout.intro h1, .slidev-layout.section h1 { font-size: 2.7em; }
.slidev-layout.fact, .slidev-layout.statement { align-items: center; text-align: center; }
.slidev-layout.fact h1 { font-size: 3.5em; }
.layout-two-cols-header { display: grid; grid-template-rows: auto minmax(0, 1fr); gap: 14px; }
.layout-header { min-height: 0; }
.layout-columns { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 38px; min-height: 0; }
.layout-columns > div { min-width: 0; min-height: 0; }
.theme-seriph { --slidev-theme-primary: #5d8392; }
.theme-seriph .slidev-layout { color: #252525; background: #fffdfa; }
.theme-seriph .slidev-layout h1, .theme-seriph .slidev-layout h2 { color: var(--slidev-theme-primary); font-family: Georgia, 'Times New Roman', serif; }
.theme-default, .theme-apple-basic, .theme-bricks, .theme-shibainu { --slidev-theme-primary: #3ab7d3; }
.player-drawing-layer { position: absolute; z-index: 4; inset: 0; width: 980px; height: 551px; pointer-events: none; touch-action: none; }
.player-drawing-layer.active { cursor: crosshair; pointer-events: auto; }
.player-drawing-layer :is(polyline, line, rect, ellipse) { fill: none; stroke-linecap: round; stroke-linejoin: round; pointer-events: stroke; }
.player-laser { position: fixed; z-index: 7; width: 14px; height: 14px; border: 2px solid white; border-radius: 50%; background: #ef4444; box-shadow: 0 0 0 8px #ef44442e, 0 2px 8px #0005; pointer-events: none; transform: translate(-50%, -50%); }
.universal-element-votes { display: inline-flex; gap: 5px; margin-left: 8px; vertical-align: middle; }
.universal-element-votes span, .universal-element-vote { display: inline-flex; padding: 2px 8px; border-radius: 999px; font-size: .55em; font-weight: 750; }
.universal-element-votes .up { color: #15803d; background: #22c55e20; }
.universal-element-votes .down { color: #dc2626; background: #ef444420; }
.universal-element-vote.rating { color: var(--slidev-theme-primary); background: color-mix(in srgb, var(--slidev-theme-primary) 12%, transparent); }
.universal-element-reactions { display: inline-flex; flex-wrap: wrap; gap: 5px; margin-left: 8px; vertical-align: middle; }
.universal-element-reactions span { display: inline-flex; align-items: center; gap: 3px; padding: 2px 8px; border-radius: 999px; color: currentColor; background: color-mix(in srgb, var(--slidev-theme-primary) 12%, transparent); font-size: .55em; font-weight: 750; font-variant-numeric: tabular-nums; }
.universal-element-reactions b { font-size: 1.2em; line-height: 1; }
.player-message { display: grid; justify-items: center; gap: 8px; color: #626978; font-size: 16px; }
.player-message.error { color: #923b35; }
.player-progress { position: fixed; z-index: 3; right: 0; bottom: 0; left: 0; height: 3px; background: #11182718; }
.player-progress i { display: block; height: 100%; background: var(--slidev-theme-primary, #3ab7d3); transition: width .25s ease; }
.player-navigation-hotspot { position: fixed; z-index: 6; bottom: 0; left: 0; width: 72px; height: 72px; }
.player-navigation { position: fixed; z-index: 6; bottom: 13px; left: 14px; display: flex; align-items: center; gap: 5px; padding: 6px; border: 1px solid #ffffff2e; border-radius: 10px; background: #111827e8; color: white; font-size: 12px; opacity: 0; pointer-events: none; box-shadow: 0 8px 24px #0003; transform: translateY(5px); transition: opacity .2s, transform .2s; }
.player-navigation-hotspot:hover + .player-navigation, .player-navigation:hover, .player-navigation:focus-within { opacity: 1; pointer-events: auto; transform: translateY(0); }
.player-navigation button { display: grid; place-items: center; min-width: 28px; height: 28px; padding: 0 7px; border: 0; border-radius: 6px; color: white; background: #ffffff12; font: inherit; cursor: pointer; }
.player-navigation button:hover, .player-navigation button.active { background: #ffffff2b; }
.player-navigation .player-slide-number { min-width: 72px; }
.player-navigation .record-button { color: #ff8b8b; }
.player-navigation .record-button.active { color: white; background: #dc2626; animation: recording-pulse 1.2s ease-in-out infinite; }
.player-navigation > span { width: 1px; height: 18px; margin: 0 2px; background: #ffffff25; }
.player-navigation button:disabled { opacity: .3; }
@keyframes recording-pulse { 50% { opacity: .55; } }
.player-drawing-tools { position: fixed; z-index: 6; bottom: 58px; left: 14px; display: grid; gap: 7px; padding: 8px; border: 1px solid #ffffff2e; border-radius: 11px; color: white; background: #111827ed; box-shadow: 0 10px 30px #0004; }
.drawing-modes, .drawing-options { display: flex; align-items: center; gap: 5px; }
.player-drawing-tools button { min-width: 30px; height: 30px; padding: 0 8px; border: 0; border-radius: 6px; color: white; background: #ffffff10; cursor: pointer; }
.player-drawing-tools button:hover, .player-drawing-tools button.active { background: #ffffff2b; }
.player-drawing-tools button:disabled { cursor: default; opacity: .35; }
.player-drawing-tools .drawing-color { min-width: 22px; width: 22px; height: 22px; padding: 0; border: 2px solid transparent; border-radius: 50%; }
.player-drawing-tools .drawing-color.active { border-color: white; box-shadow: 0 0 0 2px #ffffff3d; }
.player-drawing-tools input { width: 80px; accent-color: var(--slidev-theme-primary, #3ab7d3); }
.player-tool-error { position: fixed; z-index: 9; right: 18px; bottom: 18px; display: flex; align-items: center; gap: 12px; max-width: 420px; margin: 0; padding: 11px 14px; border-radius: 9px; color: #fee2e2; background: #7f1d1ded; box-shadow: 0 10px 30px #0004; font-size: 14px; }
.player-tool-error button { border: 0; color: inherit; background: transparent; font-size: 20px; cursor: pointer; }
.player-overview { position: fixed; z-index: 8; inset: 0; padding: 32px; color: white; background: #090c12e8; backdrop-filter: blur(14px); overflow: auto; }
.player-overview > header { display: flex; align-items: center; justify-content: space-between; width: min(1040px, 100%); margin: 0 auto 22px; font-size: 22px; }
.player-overview > header button { width: 36px; height: 36px; border: 1px solid #ffffff24; border-radius: 8px; color: white; background: #ffffff10; font-size: 23px; cursor: pointer; }
.player-overview > div { display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: 14px; width: min(1040px, 100%); margin: auto; }
.player-overview > div > button { display: grid; grid-template-columns: 30px minmax(0, 1fr); align-items: center; gap: 10px; min-height: 90px; padding: 14px; border: 1px solid #ffffff20; border-radius: 10px; color: white; background: #ffffff0b; text-align: left; cursor: pointer; }
.player-overview > div > button:hover, .player-overview > div > button.active { border-color: var(--slidev-theme-primary, #3ab7d3); background: #ffffff16; }
.player-overview b { display: grid; place-items: center; width: 28px; height: 28px; border-radius: 50%; background: #ffffff12; }
.player-overview span { overflow: hidden; font-size: 14px; line-height: 1.3; text-overflow: ellipsis; }
.player-shortcuts { position: fixed; z-index: 9; inset: 0; display: grid; place-items: center; padding: 28px; color: white; background: #090c12c9; backdrop-filter: blur(10px); }
.player-shortcuts > section { width: min(460px, 100%); padding: 22px; border: 1px solid #ffffff20; border-radius: 14px; background: #161b27; box-shadow: 0 22px 70px #0007; }
.player-shortcuts header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 16px; font-size: 19px; }
.player-shortcuts header button { width: 32px; height: 32px; border: 0; border-radius: 7px; color: white; background: #ffffff10; font-size: 21px; cursor: pointer; }
.player-shortcuts dl { display: grid; gap: 3px; margin: 0; }
.player-shortcuts dl div { display: flex; align-items: center; justify-content: space-between; min-height: 38px; border-bottom: 1px solid #ffffff0f; }
.player-shortcuts dt, .player-shortcuts dd { margin: 0; font-size: 14px; }
.player-shortcuts kbd { display: inline-grid; min-width: 28px; padding: 3px 7px; border: 1px solid #ffffff24; border-radius: 5px; background: #ffffff0d; text-align: center; }
</style>
