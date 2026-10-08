<script setup lang="ts">
import { computed } from 'vue'

import PackedWordCloud from './PackedWordCloud.vue'

const props = defineProps<{
  definition: Record<string, any>
  live?: Record<string, any>
  display: string
  reveal: string
  bodyHtml: string
  clickStep: number
  clickStart: number
}>()

const item = computed(() => props.live)
const kind = computed(() => props.definition.kind)
const config = computed(() => ({ ...(props.definition.config || {}), ...(item.value?.config || {}) }))
const options = computed(() => item.value?.options?.length ? item.value.options : props.definition.options || [])
const result = computed(() => item.value?.result || {})
const showTitle = computed(() => String(config.value['show-title'] ?? 'false').toLowerCase() === 'true')
const title = computed(() => String(item.value?.title || props.definition.title || '').trim())
const cloudHeight = computed(() => normalizeDimension(config.value.height, '320px'))
const cloudWidth = computed(() => normalizeDimension(config.value.width, '100%'))
const cloudSpacing = computed(() => {
  const value = Number(config.value.spacing)
  return Number.isFinite(value) ? Math.min(.5, Math.max(.04, value)) : .16
})
const resultsVisible = computed(() => {
  if (!item.value) return true
  const policy = item.value.config?.results
  if (policy === 'presenter' || policy === 'hidden') return false
  if (policy === 'manual' || policy === 'on-close') return item.value.results_revealed
  return true
})
const choiceRows = computed(() => {
  const total = Math.max(0, Number(result.value.count || 0))
  return options.value.map((option: any) => {
    const count = Math.max(0, Number(result.value.counts?.[option.id] || 0))
    return { ...option, count, percentage: total ? count / total * 100 : 0 }
  })
})
const rankedOptions = computed(() => {
  const ranking = item.value?.phase === 'ranked' ? result.value.ranking : null
  if (!Array.isArray(ranking)) return options.value
  const byId = new Map(options.value.map((option: any) => [option.id, option]))
  return ranking.map((id: string) => byId.get(id)).filter(Boolean)
})
const resultOrderedOptions = computed(() => {
  const ranking = result.value.ranking
  if (!Array.isArray(ranking)) return options.value
  const byId = new Map(options.value.map((option: any) => [option.id, option]))
  return ranking.map((id: string) => byId.get(id)).filter(Boolean)
})
const allocationRows = computed(() => options.value.map((option: any) => ({
  ...option,
  average: Number(result.value.averages?.[option.id] || 0),
})))
const surveyQuestions = computed(() => Array.isArray(config.value.questions) ? config.value.questions : [])
const visibleRankedOptions = computed(() => {
  if (props.reveal !== 'click' || item.value?.phase === 'ranked') return rankedOptions.value
  const visibleFromClicks = props.clickStart ? Math.max(0, props.clickStep - props.clickStart + 1) : rankedOptions.value.length
  const visible = Math.max(Number(item.value?.revealed_count || 0), visibleFromClicks)
  return rankedOptions.value.slice(0, visible)
})
const summary = computed(() => {
  const count = Number(result.value.count || 0)
  const responses = `${count} ${count === 1 ? 'response' : 'responses'}`
  if (!item.value) return 'Waiting for presentation'
  if (!resultsVisible.value && kind.value !== 'ranked-list') return `${responses} · Results hidden`
  if (['poll', 'quiz', 'reaction'].includes(kind.value)) return responses
  if (kind.value === 'word-cloud') return `${result.value.submission_count || 0} words · ${count} ${count === 1 ? 'person' : 'people'}`
  if (kind.value === 'rating') return result.value.average == null ? 'No ratings yet' : `Average ${Number(result.value.average).toFixed(1)} · ${responses}`
  if (kind.value === 'number') return result.value.average == null ? 'No estimates yet' : `Average ${Number(result.value.average).toFixed(1)} · Median ${Number(result.value.median).toFixed(1)}`
  if (kind.value === 'allocation') return `${count} ${count === 1 ? 'allocation' : 'allocations'}`
  if (kind.value === 'matrix') return result.value.average_x == null ? 'No points yet' : `Room center ${Number(result.value.average_x).toFixed(1)}, ${Number(result.value.average_y).toFixed(1)}`
  if (kind.value === 'ranking') return `${count} complete ${count === 1 ? 'ranking' : 'rankings'}`
  if (kind.value === 'image-hotspot') return `${count} placed ${count === 1 ? 'pin' : 'pins'}`
  if (kind.value === 'survey') return `${count} complete ${count === 1 ? 'survey' : 'surveys'}`
  if (kind.value === 'ranked-list') return item.value.phase === 'ranked' ? `Final ranking · ${count} voters` : `Voting open · ${item.value.revealed_count || 0} of ${options.value.length} ideas · ${count} voters`
  return responses
})
const timer = computed(() => Number(item.value?.timer_remaining_seconds ?? item.value?.config?.timer ?? 0))
const timerLabel = computed(() => `${Math.floor(timer.value / 60)}:${String(Math.max(0, timer.value % 60)).padStart(2, '0')}`)

function matrixPointStyle(point: Record<string, any>) {
  const xMin = Number(config.value['x-min'] || 0)
  const xMax = Number(config.value['x-max'] || 10)
  const yMin = Number(config.value['y-min'] || 0)
  const yMax = Number(config.value['y-max'] || 10)
  return {
    left: `${(Number(point.x) - xMin) / (xMax - xMin) * 100}%`,
    bottom: `${(Number(point.y) - yMin) / (yMax - yMin) * 100}%`,
  }
}

function normalizedPointStyle(point: Record<string, any>) {
  return { left: `${Number(point.x) * 100}%`, top: `${Number(point.y) * 100}%` }
}

function normalizeDimension(value: unknown, fallback: string) {
  const source = String(value ?? '').trim()
  if (/^\d{2,4}$/.test(source)) return `${source}px`
  if (/^\d{1,3}%$/.test(source)) return source
  return fallback
}
</script>

<template>
  <section :class="['universal-interaction', { 'is-slide': display === 'slide', ranked: kind === 'ranked-list' }]">
    <h2 v-if="showTitle && title" class="interaction-title">{{ title }}</h2>
    <div v-if="bodyHtml" class="interaction-copy" v-html="bodyHtml" />
    <div class="interaction-status"><i />{{ summary }}</div>
    <div v-if="item?.config?.timer" :class="['interaction-timer', { elapsed: timer === 0 }]">{{ timerLabel }}</div>

    <div v-if="['poll', 'quiz', 'reaction'].includes(kind)" class="interaction-bars">
      <div v-for="option in choiceRows" :key="option.id" :class="['bar-row', { correct: resultsVisible && kind === 'quiz' && result.correct_option_id === option.id }]">
        <i :style="{ width: `${resultsVisible ? option.percentage : 0}%` }" />
        <span>{{ option.label }}</span>
        <strong v-if="resultsVisible">{{ option.count }} <small>{{ Math.round(option.percentage) }}%</small></strong>
      </div>
    </div>

    <PackedWordCloud
      v-else-if="kind === 'word-cloud'"
      :words="result.words || []"
      :height="cloudHeight"
      :width="cloudWidth"
      :spacing="cloudSpacing"
      class="interaction-words"
    />

    <ol v-else-if="kind === 'ranked-list'" class="ranked-options">
      <li v-for="(option, index) in visibleRankedOptions" :key="option?.id">
        <b>{{ item?.phase === 'ranked' ? index + 1 : '•' }}</b>
        <span>{{ option?.label }}</span>
        <span v-if="item?.phase === 'ranked'" class="vote-totals">
          <strong class="up">▲ {{ result.scores?.[option?.id || '']?.up || 0 }}</strong>
          <strong class="down">▼ {{ result.scores?.[option?.id || '']?.down || 0 }}</strong>
        </span>
      </li>
    </ol>

    <div v-else-if="kind === 'image-choice' && resultsVisible" class="image-choices">
      <article v-for="option in options" :key="option.id"><img :src="option.image_url" :alt="option.label"><span>{{ option.label }}</span><strong>{{ result.counts?.[option.id] || 0 }}</strong></article>
    </div>

    <div v-else-if="kind === 'allocation' && resultsVisible" class="interaction-bars">
      <div v-for="option in allocationRows" :key="option.id" class="bar-row allocation-row">
        <i :style="{ width: `${Math.min(100, option.average / Number(config.total || 100) * 100)}%` }" />
        <span>{{ option.label }}</span>
        <strong>{{ option.average.toFixed(1) }}</strong>
      </div>
    </div>

    <div v-else-if="kind === 'matrix' && resultsVisible" class="interaction-matrix">
      <i v-for="(point, index) in result.points || []" :key="index" :style="matrixPointStyle(point)" />
      <span class="x-label">{{ config['x-label'] || 'Horizontal' }}</span>
      <span class="y-label">{{ config['y-label'] || 'Vertical' }}</span>
    </div>

    <ol v-else-if="kind === 'ranking' && resultsVisible" class="result-ranking">
      <li v-for="(option, index) in resultOrderedOptions" :key="option?.id">
        <b>{{ index + 1 }}</b><span>{{ option?.label }}</span>
        <strong v-if="result.count">avg {{ Number(result.scores?.[option?.id]?.average_rank || 0).toFixed(1) }}</strong>
      </li>
    </ol>

    <div v-else-if="kind === 'image-hotspot' && resultsVisible" class="interaction-hotspot">
      <img :src="config.image" :alt="config.alt">
      <i v-for="(point, index) in result.points || []" :key="index" :style="normalizedPointStyle(point)" />
    </div>

    <div v-else-if="kind === 'survey' && resultsVisible" class="survey-results">
      <article v-for="question in surveyQuestions" :key="question.id">
        <span>{{ question.label }}</span>
        <strong v-if="question.type === 'rating'">{{ result.questions?.[question.id]?.average == null ? '—' : Number(result.questions[question.id].average).toFixed(1) }}</strong>
        <div v-else-if="question.type === 'choice'">
          <small v-for="option in question.options" :key="option.id">{{ option.label }} · {{ result.questions?.[question.id]?.counts?.[option.id] || 0 }}</small>
        </div>
        <strong v-else>{{ result.questions?.[question.id]?.count || 0 }} responses</strong>
      </article>
    </div>

    <div v-else-if="kind === 'rating' && resultsVisible" class="metric-result">
      <strong>{{ result.average == null ? '—' : Number(result.average).toFixed(1) }}</strong>
      <span>{{ config.min || 1 }} to {{ config.max || 5 }}</span>
    </div>

    <div v-else-if="kind === 'number' && resultsVisible" class="metric-result">
      <strong>{{ result.average == null ? '—' : Number(result.average).toFixed(1) }}{{ config.unit ? ` ${config.unit}` : '' }}</strong>
      <span>room average</span>
    </div>
  </section>
</template>

<style scoped>
.universal-interaction { padding: 20px 24px; border: 2px solid color-mix(in srgb, currentColor 15%, transparent); border-radius: 18px; background: color-mix(in srgb, var(--slidev-theme-primary, #6366f1) 6%, transparent); }
.universal-interaction.is-slide { display: flex; flex-direction: column; min-height: 100%; padding: 0; border: 0; border-radius: 0; background: transparent; }
.interaction-title { margin: 0 0 12px; font-size: 1.18em; line-height: 1.12; }
.interaction-copy :deep(> :first-child) { margin-top: 0; }
.interaction-copy :deep(> :last-child) { margin-bottom: 0; }
.interaction-status { display: flex; align-items: center; gap: 8px; margin-top: 12px; font-size: .68em; opacity: .72; }
.is-slide .interaction-status { margin-top: auto; }
.interaction-status i { width: 8px; height: 8px; border-radius: 50%; background: #22c55e; box-shadow: 0 0 0 4px #22c55e22; }
.interaction-timer { align-self: flex-start; margin-top: 12px; padding: 5px 10px; border-radius: 9px; color: var(--slidev-theme-primary); background: color-mix(in srgb, var(--slidev-theme-primary) 12%, transparent); font: 800 .9em ui-monospace, monospace; }
.interaction-timer.elapsed { color: #dc2626; }
.interaction-bars { display: grid; gap: 8px; margin-top: 14px; font-size: .72em; }
.bar-row { position: relative; display: flex; justify-content: space-between; gap: 20px; padding: 10px 12px; border-radius: 9px; background: color-mix(in srgb, currentColor 7%, transparent); overflow: hidden; }
.bar-row > i { position: absolute; inset: 0 auto 0 0; background: color-mix(in srgb, var(--slidev-theme-primary) 22%, transparent); transition: width .35s ease; }
.bar-row > span, .bar-row > strong { position: relative; z-index: 1; }
.bar-row strong { display: flex; gap: 7px; font-variant-numeric: tabular-nums; }
.bar-row small { font-size: .72em; opacity: .65; }
.bar-row.correct { outline: 2px solid #22c55e; }
.interaction-words { flex: none; max-width: 100%; min-height: 180px; margin-top: 14px; }
.ranked-options, .result-ranking { display: grid; gap: 10px; padding: 0; margin: 18px 0 0; list-style: none; }
.ranked-options li, .result-ranking li { display: grid; grid-template-columns: 34px 1fr auto; align-items: center; gap: 12px; min-height: 48px; padding: 8px 14px; border-radius: 12px; background: color-mix(in srgb, currentColor 7%, transparent); }
.ranked-options li > b, .result-ranking li > b { display: grid; place-items: center; width: 28px; height: 28px; border-radius: 50%; color: white; background: var(--slidev-theme-primary); font-size: .66em; }
.vote-totals { display: inline-flex; gap: 7px; font-size: .76em; }
.vote-totals strong { padding: 3px 7px; border-radius: 999px; }
.vote-totals .up { color: #15803d; background: #22c55e20; }
.vote-totals .down { color: #dc2626; background: #ef444420; }
.image-choices { display: grid; grid-template-columns: repeat(2, 1fr); gap: 12px; margin-top: 14px; }
.image-choices article { display: grid; grid-template-columns: 1fr auto; overflow: hidden; border-radius: 12px; background: color-mix(in srgb, currentColor 6%, transparent); }
.image-choices img { grid-column: 1 / -1; width: 100%; height: 110px; object-fit: cover; }
.image-choices span, .image-choices strong { padding: 8px 10px; }
.allocation-row > i { background: color-mix(in srgb, var(--slidev-theme-primary) 18%, transparent); }
.interaction-matrix { position: relative; height: 250px; margin: 20px 20px 28px 34px; border-left: 2px solid currentColor; border-bottom: 2px solid currentColor; background: linear-gradient(to right, transparent 49.7%, color-mix(in srgb, currentColor 15%, transparent) 50%, transparent 50.3%), linear-gradient(to top, transparent 49.7%, color-mix(in srgb, currentColor 15%, transparent) 50%, transparent 50.3%); }
.interaction-matrix i { position: absolute; width: 10px; height: 10px; border-radius: 50%; background: var(--slidev-theme-primary); transform: translate(-50%, 50%); opacity: .58; }
.interaction-matrix .x-label { position: absolute; right: 0; bottom: -25px; font-size: .52em; }
.interaction-matrix .y-label { position: absolute; top: 0; left: -32px; font-size: .52em; writing-mode: vertical-rl; transform: rotate(180deg); }
.interaction-hotspot { position: relative; align-self: center; max-width: 100%; margin-top: 14px; overflow: hidden; }
.interaction-hotspot img { display: block; max-width: 100%; max-height: 310px; object-fit: contain; }
.interaction-hotspot i { position: absolute; width: 12px; height: 12px; border: 2px solid white; border-radius: 50%; background: var(--slidev-theme-primary); box-shadow: 0 2px 5px #0005; transform: translate(-50%, -50%); opacity: .68; }
.survey-results { display: grid; gap: 8px; margin-top: 14px; }
.survey-results article { display: grid; grid-template-columns: minmax(0, 1fr) auto; align-items: center; gap: 8px; padding: 9px 11px; border-radius: 9px; background: color-mix(in srgb, currentColor 6%, transparent); }
.survey-results article > span { font-size: .65em; }
.survey-results article > strong { color: var(--slidev-theme-primary); }
.survey-results article > div { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 4px 9px; max-width: 55%; }
.survey-results small { font-size: .48em; opacity: .75; }
.metric-result { display: grid; place-items: center; align-content: center; flex: 1; min-height: 180px; }
.metric-result strong { color: var(--slidev-theme-primary); font-size: 3em; line-height: 1; }
.metric-result span { margin-top: 8px; font-size: .62em; opacity: .68; }
</style>
