<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import { api } from '../../api'
import type { LiveState, LiveSummary } from '../../types'

const props = defineProps<{ deckId: string; state: LiveState | null; show: string }>()
const report = ref<LiveSummary | null>(null)
const error = ref('')
const loading = ref(true)
const scrollRegion = ref<HTMLElement | null>(null)
let refreshTimer: number | undefined

const interactions = computed(() => (report.value?.interactions || [])
  .filter(item => props.show === 'all' || item.response_count > 0))
const hasQuestions = computed(() => (report.value?.questions.approved || 0) > 0)
const kindLabels: Record<string, string> = {
  'word-cloud': 'Word cloud', 'free-text': 'Written response', 'ranked-list': 'Idea voting',
  'image-choice': 'Image choice', 'image-hotspot': 'Image hotspot', updown: 'Up/down vote',
}
const kindLabel = (kind: string) => kindLabels[kind] || kind.replaceAll('-', ' ')

onMounted(refresh)
onBeforeUnmount(() => window.clearInterval(refreshTimer))
watch(() => props.state?.live, (live) => {
  window.clearInterval(refreshTimer)
  refreshTimer = live ? window.setInterval(refresh, 5000) : undefined
}, { immediate: true })

async function refresh() {
  try {
    report.value = await api<LiveSummary>(`/api/decks/${props.deckId}/live-summary`)
    error.value = ''
  }
  catch (reason) {
    error.value = reason instanceof Error ? reason.message : 'The live summary is temporarily unavailable.'
  }
  finally { loading.value = false }
}

function scrollReport(event: KeyboardEvent) {
  const region = scrollRegion.value
  if (!region || !['ArrowDown', 'ArrowUp', 'PageDown', 'PageUp', 'Home', 'End'].includes(event.key)) return
  event.preventDefault()
  event.stopPropagation()
  if (event.key === 'Home') region.scrollTo({ top: 0, behavior: 'smooth' })
  else if (event.key === 'End') region.scrollTo({ top: region.scrollHeight, behavior: 'smooth' })
  else region.scrollBy({
    top: event.key === 'ArrowDown' ? 90 : event.key === 'ArrowUp' ? -90 : event.key === 'PageDown' ? region.clientHeight * .8 : -region.clientHeight * .8,
    behavior: 'smooth',
  })
}
</script>

<template>
  <section class="universal-live-summary" aria-live="polite">
    <header>
      <div><i /><span>ROOM SUMMARY</span></div>
      <strong v-if="report">Results session {{ report.result_epoch }}</strong>
    </header>
    <div v-if="report" class="summary-metrics">
      <div><b>{{ report.participant_count }}</b><span>participants</span></div>
      <div><b>{{ report.response_count }}</b><span>interaction responses</span></div>
      <div><b>{{ report.questions.approved }}</b><span>approved questions</span></div>
    </div>
    <div ref="scrollRegion" class="summary-scroll" tabindex="0" aria-label="Scrollable interaction summary" @keydown="scrollReport" @wheel.stop>
      <div v-if="loading" class="summary-empty">Preparing the current results session…</div>
      <div v-else-if="error && !report" class="summary-empty">{{ error }}</div>
      <div v-else-if="report" class="summary-list">
        <article v-for="item in interactions" :key="item.id">
          <div class="summary-meta"><span>Slide {{ item.slide_number }}</span><small>{{ kindLabel(item.kind) }}</small><b>{{ item.response_count }}</b></div>
          <h3>{{ item.title || `Interaction on slide ${item.slide_number}` }}</h3>
          <p>{{ item.headline }}</p>
          <div v-if="item.highlights.length" class="summary-highlights"><span v-for="highlight in item.highlights" :key="highlight">{{ highlight }}</span></div>
        </article>
        <article v-if="hasQuestions" class="questions-card">
          <div class="summary-meta"><span>Deck-wide</span><small>Q&amp;A</small><b>{{ report.questions.approved }}</b></div>
          <h3>Questions from the room</h3>
          <p>{{ report.questions.answered }} answered · {{ report.questions.total_votes }} audience upvotes</p>
          <ol><li v-for="question in report.questions.top.slice(0, 8)" :key="question.id"><span>{{ question.body }}</span><small>{{ question.display_name || 'Anonymous' }}<template v-if="question.slide_number"> · Slide {{ question.slide_number }}</template> · ▲ {{ question.votes }}</small></li></ol>
        </article>
        <div v-if="!interactions.length && !hasQuestions" class="summary-empty">No responses have been collected in this results session yet.</div>
      </div>
    </div>
    <footer><span>Current epoch · refreshes while presenting</span><small>Focus this report and use ↑ ↓ or Page Up/Down to scroll</small></footer>
  </section>
</template>

<style scoped>
.universal-live-summary { display: grid; grid-template-rows: auto auto minmax(0, 1fr) auto; gap: 11px; width: 100%; height: 420px; min-height: 0; max-height: 420px; }
.universal-live-summary > header, .universal-live-summary > footer { display: flex; align-items: center; justify-content: space-between; gap: 18px; }
.universal-live-summary > header > div { display: flex; align-items: center; gap: 9px; color: var(--slidev-theme-primary); font-size: .54em; font-weight: 850; letter-spacing: .12em; }
.universal-live-summary > header i { width: 9px; height: 9px; border-radius: 50%; background: #22c55e; box-shadow: 0 0 0 5px #22c55e20; }
.universal-live-summary > header strong { padding: 5px 10px; border-radius: 999px; background: color-mix(in srgb, currentColor 7%, transparent); font-size: .48em; }
.summary-metrics { display: grid; grid-template-columns: repeat(3, 1fr); gap: 9px; }
.summary-metrics > div { display: flex; align-items: baseline; gap: 7px; padding: 8px 11px; border-radius: 11px; background: color-mix(in srgb, var(--slidev-theme-primary) 8%, transparent); }
.summary-metrics b { color: var(--slidev-theme-primary); font-size: .72em; }.summary-metrics span { font-size: .42em; opacity: .68; }
.summary-scroll { min-height: 0; padding: 1px 9px 1px 1px; overflow-y: auto; overscroll-behavior: contain; scroll-snap-type: y proximity; scrollbar-gutter: stable; outline: none; touch-action: pan-y; }
.summary-scroll:focus-visible { border-radius: 12px; box-shadow: 0 0 0 3px color-mix(in srgb, var(--slidev-theme-primary) 22%, transparent); }
.summary-list { display: grid; gap: 9px; }.summary-list article { padding: 12px 14px; border: 1px solid color-mix(in srgb, currentColor 13%, transparent); border-radius: 14px; background: color-mix(in srgb, currentColor 3%, transparent); scroll-snap-align: start; }
.summary-meta { display: flex; align-items: center; gap: 8px; margin-bottom: 5px; font-size: .4em; text-transform: uppercase; letter-spacing: .06em; opacity: .65; }.summary-meta span { color: var(--slidev-theme-primary); font-weight: 850; }.summary-meta b { margin-left: auto; }
.summary-list h3 { margin: 0; font-size: .66em; line-height: 1.2; }.summary-list p { margin: 3px 0 0; font-size: .5em; line-height: 1.35; opacity: .78; }
.summary-highlights { display: flex; flex-wrap: wrap; gap: 5px; margin-top: 8px; }.summary-highlights span { padding: 3px 7px; border-radius: 999px; background: color-mix(in srgb, var(--slidev-theme-primary) 9%, transparent); font-size: .38em; }
.questions-card ol { display: grid; gap: 6px; margin: 9px 0 0; padding-left: 1.2em; }.questions-card li { padding-left: 4px; font-size: .46em; }.questions-card li span, .questions-card li small { display: block; }.questions-card li small { margin-top: 2px; opacity: .58; }
.summary-empty { display: grid; min-height: 140px; place-items: center; padding: 20px; border: 2px dashed color-mix(in srgb, currentColor 13%, transparent); border-radius: 14px; font-size: .56em; text-align: center; opacity: .62; }
.universal-live-summary > footer { padding-top: 7px; border-top: 1px solid color-mix(in srgb, currentColor 10%, transparent); font-size: .39em; opacity: .55; }
</style>
