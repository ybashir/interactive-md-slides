import { parseChartSpec, parseEChartsSpec } from '../../../apps/web/src/chart-spec.mjs'

const blockPattern = /:::interact\{([^}]*)\}\s*([\s\S]*?)\s*:::/g
const chartPattern = /:::chart\{([^}]*)\}\s*([\s\S]*?)\s*:::/g
const echartsPattern = /:::echarts(?:\{([^}]*)\})?\s*([\s\S]*?)\s*:::/g
const inlineChartPattern = /^::chart\{([^}]*)\}\s*$/gm
const elementPattern = /^(\s*(?:[-*+]\s+)?)(.+?)\s+\{([^}\n]*interact=(?:"[^"]+"|'[^']+'|[^\s}]+)[^}\n]*)\}\s*$/gm
const qrPattern = /::audience-qr(?:\{([^}]*)\})?/g
const audienceCountPattern = /::audience-count(?:\{([^}]*)\})?/g
const liveQaPattern = /::live-qa(?:\{([^}]*)\})?/g
const liveSummaryPattern = /::live-summary(?:\{([^}]*)\})?/g
const slideMarkerPattern = /^<!--\s*interdeck-slide:\s*[a-zA-Z0-9][a-zA-Z0-9_-]*\s*-->\s*/gm
const attributePattern = /([a-zA-Z][a-zA-Z0-9_-]*)=(?:"([^"]*)"|'([^']*)'|([^\s]+))/g
const optionPattern = /^\s*-\s*\[([a-zA-Z0-9][a-zA-Z0-9_-]*)\]\s+(.+?)\s*$/gm
const optionImagePattern = /\s+\{image=(?:"([^"]+)"|'([^']+)'|([^\s}]+))\}\s*$/
const uploadedAssetImagePattern = /!\[([^\]\n]*)\]\((\/api\/decks\/([0-9a-f-]+)\/assets\/[0-9a-f-]+\/content)(?:\s+(?:"[^"]*"|'[^']*'))?\)/gi
const uploadedAssetHtmlImagePattern = /(\\)?<img\b([^>]*?)\ssrc\s*=\s*(["'])(\/api\/decks\/([0-9a-f-]+)\/assets\/[0-9a-f-]+\/content)\3([^>]*)>/gi

export function transformMarkdown(source, { deckId, joinUrl }) {
  // Stable Interdeck IDs are persistence metadata. Keeping the marker as the
  // first line after `---` can make Slidev interpret a bullet-led slide as YAML.
  let output = source.replace(slideMarkerPattern, '\n').replace(echartsPattern, (_match, attributes = '', body) => {
    try {
      const spec = parseEChartsSpec(attributes, body)
      return `\n<InterdeckChart deck-id="${escapeAttribute(deckId)}" spec-json="${escapeAttribute(JSON.stringify(spec))}" />\n`
    } catch (error) {
      return `\n> Invalid ECharts option: ${error instanceof Error ? error.message : String(error)}\n`
    }
  }).replace(chartPattern, (_match, attributes, body) => {
    try {
      const spec = parseChartSpec(attributes, body)
      return `\n<InterdeckChart deck-id="${escapeAttribute(deckId)}" spec-json="${escapeAttribute(JSON.stringify(spec))}" />\n`
    } catch (error) {
      return `\n> Invalid Interdeck chart: ${error instanceof Error ? error.message : String(error)}\n`
    }
  }).replace(inlineChartPattern, (_match, attributes) => {
    try {
      const spec = parseChartSpec(attributes, '')
      return `\n<InterdeckChart deck-id="${escapeAttribute(deckId)}" spec-json="${escapeAttribute(JSON.stringify(spec))}" />\n`
    } catch (error) {
      return `\n> Invalid Interdeck chart: ${error instanceof Error ? error.message : String(error)}\n`
    }
  }).replace(blockPattern, (_match, attributes, body) => {
    const attrs = parseAttributes(attributes)
    if (!attrs.id || !attrs.type)
      return `\n> Invalid Interdeck interaction: missing id or type\n`
    let readableBody = ['poll', 'quiz', 'reaction', 'ranked-list', 'image-choice', 'allocation', 'ranking', 'survey'].includes(attrs.type)
      ? body.replace(/^\s*-\s*\[[a-zA-Z0-9][a-zA-Z0-9_-]*\]\s+.*$/gm, '').trim()
      : body.replace(/^\s*-\s*\[[a-zA-Z0-9][a-zA-Z0-9_-]*\]\s+/gm, '- ')
    const showTitle = attrs['show-title'] === 'true'
    const sourceTitle = attrs.question || body.match(/^\s*#{1,6}\s+(.+?)\s*$/m)?.[1] || ''
    if (showTitle)
      readableBody = readableBody.replace(/^\s*#{1,6}\s+.+(?:\r?\n|$)/, '').trim()
    const options = attrs.type === 'survey' ? [] : [...body.matchAll(optionPattern)].map(match => {
      const rawLabel = match[2].trim()
      const image = rawLabel.match(optionImagePattern)
      return {
        id: match[1],
        label: image ? rawLabel.slice(0, image.index).trim() : rawLabel,
        ...(image ? { image_url: image[1] ?? image[2] ?? image[3] } : {}),
      }
    })
    const cloudHeight = normalizeCloudDimension(attrs.height, '320px')
    const cloudWidth = normalizeCloudDimension(attrs.width, '100%')
    const cloudSpacing = normalizeCloudSpacing(attrs.spacing)
    return `\n<InterdeckInteraction deck-id="${escapeAttribute(deckId)}" interaction-id="${escapeAttribute(attrs.id)}" kind="${escapeAttribute(attrs.type)}" display="${escapeAttribute(attrs.display || 'card')}" reveal="${escapeAttribute(attrs.reveal || 'all')}" source-title="${escapeAttribute(sourceTitle)}" :show-title="${showTitle}" options-json="${escapeAttribute(JSON.stringify(options))}" cloud-height="${cloudHeight}" cloud-width="${cloudWidth}" :cloud-spacing="${cloudSpacing}">\n\n${readableBody}\n\n</InterdeckInteraction>\n`
  })

  output = output.replace(elementPattern, (_match, prefix, label, attributes) => {
    const attrs = parseAttributes(attributes)
    if (!attrs.id)
      return `${prefix}${label}`
    const options = attrs.interact === 'reaction' ? parseInlineOptions(attrs.options || '') : []
    return `${prefix}${label} <InterdeckElementVote deck-id="${escapeAttribute(deckId)}" interaction-id="${escapeAttribute(attrs.id)}" kind="${escapeAttribute(attrs.interact || 'updown')}" options-json="${escapeAttribute(JSON.stringify(options))}" />`
  })

  output = output.replace(qrPattern, (_match, attributes = '') => {
    const attrs = parseAttributes(attributes)
    const size = /^\d{2,4}$/.test(attrs.size || '') ? attrs.size : '180'
    return `<AudienceQR deck-id="${escapeAttribute(deckId)}" fallback-url="${escapeAttribute(joinUrl)}" :size="${size}" />`
  })

  output = output.replace(audienceCountPattern, (_match, attributes = '') => {
    const attrs = parseAttributes(attributes)
    const label = attrs.label?.trim() || 'people joined'
    return `<AudienceCount deck-id="${escapeAttribute(deckId)}" label="${escapeAttribute(label)}" />`
  })

  output = output.replace(liveQaPattern, (_match, attributes = '') => {
    const attrs = parseAttributes(attributes)
    const max = /^\d{1,2}$/.test(attrs.max || '') ? attrs.max : '4'
    const show = ['open', 'all'].includes(attrs.show) ? attrs.show : 'open'
    return `<InterdeckLiveQA deck-id="${escapeAttribute(deckId)}" :max-themes="${max}" show="${show}" />`
  })

  output = output.replace(liveSummaryPattern, (_match, attributes = '') => {
    const attrs = parseAttributes(attributes)
    const show = attrs.show === 'all' ? 'all' : 'responded'
    return `<InterdeckLiveSummary deck-id="${escapeAttribute(deckId)}" show="${show}" />`
  })

  // Slidev asks Vite to import ordinary Markdown image URLs at compile time.
  // Private deck assets are same-origin runtime resources instead, so keep the
  // idiomatic Markdown in storage and emit a bound src only for this deck.
  output = output.replace(uploadedAssetImagePattern, (match, alt, url, assetDeckId) => {
    if (assetDeckId.toLowerCase() !== String(deckId).toLowerCase()) return match
    return `<img :src="'${url}'" alt="${escapeAttribute(alt)}" />`
  })

  // Raw HTML is useful when an author needs image attributes that Markdown
  // does not express, such as width. Rewrite the private URL as a Vue-bound
  // runtime source for the same reason as above. A single Markdown escape
  // before this specific tag is harmlessly removed so pasted `\<img ...>`
  // markup behaves as the author intended.
  output = output.replace(uploadedAssetHtmlImagePattern, (match, _escaped, before, _quote, url, assetDeckId, after) => {
    if (assetDeckId.toLowerCase() !== String(deckId).toLowerCase()) return match
    return `<img${before} :src="'${url}'"${after}>`
  })

  return output
}

export const chartComponent = `<script setup>
import { useSlideContext } from '@slidev/client'
import * as echarts from 'echarts'
import { computed, getCurrentInstance, nextTick, onBeforeUnmount, onMounted, ref, unref, watch, watchEffect } from 'vue'
import { chartAccessibleLabel, chartRevealClicks, resolveChartOption } from '../composables/chartSpec.mjs'
import { useInterdeckLive } from '../composables/useInterdeckLive.js'

const props = defineProps({ deckId: String, specJson: String })
const container = ref(null)
const spec = computed(() => {
  try { return JSON.parse(props.specJson || '{}') }
  catch { return { type: 'bar', height: 320, labels: [], series: [] } }
})
const interaction = useInterdeckLive(props.deckId, spec.value.source)
const accessibleLabel = computed(() => chartAccessibleLabel(spec.value))
const revealClicks = computed(() => chartRevealClicks(spec.value))
const visibleSeries = ref(revealClicks.value ? 1 : undefined)
const { $clicksContext: clicks, $renderContext } = useSlideContext()
const clickRegistrationId = 'interdeck-chart-' + getCurrentInstance().uid
let chart = null
let resizeObserver = null
let clicksRegistered = false
let stopRevealWatch = null
let previousSignature = ''
let previousVisibleSeries

function colors() {
  if (!container.value) return {}
  const style = window.getComputedStyle(container.value)
  return { primary: style.getPropertyValue('--slidev-theme-primary').trim() || '#6657d9', text: style.color || '#172033' }
}
function render() {
  if (!container.value) return
  if (!chart) chart = echarts.init(container.value, undefined, { renderer: 'svg' })
  const signature = JSON.stringify(spec.value)
  const mergeSeries = signature === previousSignature && visibleSeries.value !== previousVisibleSeries
  chart.setOption(
    resolveChartOption(spec.value, interaction.value, interaction.value, colors(), visibleSeries.value),
    mergeSeries ? { notMerge: false, replaceMerge: ['series'] } : { notMerge: true },
  )
  previousSignature = signature
  previousVisibleSeries = visibleSeries.value
  chart.resize()
}
onMounted(() => {
  if (revealClicks.value) {
    const totalSeries = revealClicks.value + 1
    if (unref($renderContext) === 'print') visibleSeries.value = totalSeries
    else {
      const info = clicks.calculateSince(undefined, revealClicks.value)
      if (!info) visibleSeries.value = totalSeries
      else {
        clicks.register(clickRegistrationId, info)
        clicksRegistered = true
        stopRevealWatch = watchEffect(() => {
          visibleSeries.value = 1 + Math.min(revealClicks.value, Math.max(0, info.currentOffset.value + 1))
        })
      }
    }
  }
  render()
  resizeObserver = new ResizeObserver(() => chart && chart.resize())
  resizeObserver.observe(container.value)
})
watch(() => [spec.value, interaction.value, visibleSeries.value], () => nextTick(render), { deep: true })
onBeforeUnmount(() => {
  if (clicksRegistered) clicks.unregister(clickRegistrationId)
  if (stopRevealWatch) stopRevealWatch()
  if (resizeObserver) resizeObserver.disconnect()
  if (chart) chart.dispose()
})
</script>

<template>
  <figure class="interdeck-chart" :style="{ height: spec.height + 'px' }">
    <div ref="container" role="img" :aria-label="accessibleLabel" />
  </figure>
</template>

<style scoped>
.interdeck-chart { width: 100%; min-width: 0; margin: 8px 0; }
.interdeck-chart > div { width: 100%; height: 100%; }
</style>`

export function parseAttributes(source) {
  const attributes = {}
  for (const match of source.matchAll(attributePattern))
    attributes[match[1]] = match[2] ?? match[3] ?? match[4]
  return attributes
}

function parseInlineOptions(source) {
  return String(source || '').split('|').map((entry) => {
    const separator = entry.indexOf(':')
    if (separator < 1) return null
    const id = entry.slice(0, separator).trim()
    const label = entry.slice(separator + 1).trim()
    return id && label ? { id, label } : null
  }).filter(Boolean)
}

function escapeAttribute(value) {
  return String(value)
    .replaceAll('&', '&amp;')
    .replaceAll('"', '&quot;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
}

function normalizeCloudDimension(value, fallback) {
  const source = String(value || '').trim()
  if (/^\d{2,4}$/.test(source)) return `${source}px`
  if (/^\d{1,3}%$/.test(source)) return source
  return fallback
}

function normalizeCloudSpacing(value) {
  const number = Number(value)
  if (!Number.isFinite(number)) return '.16'
  return String(Math.min(.5, Math.max(.04, number)))
}

export const interactionComponent = `<script setup>
import VueWordCloud from 'vuewordcloud'
import { computed, shallowRef, watch } from 'vue'
import { useInterdeckLive } from '../composables/useInterdeckLive.js'

const props = defineProps({
  deckId: String,
  interactionId: String,
  kind: String,
  display: String,
  reveal: String,
  sourceTitle: String,
  showTitle: { type: Boolean, default: false },
  optionsJson: String,
  cloudHeight: { type: String, default: '320px' },
  cloudWidth: { type: String, default: '100%' },
  cloudSpacing: { type: Number, default: .16 },
})
const interaction = useInterdeckLive(props.deckId, props.interactionId)
const title = computed(() => String(interaction.value?.title || props.sourceTitle || '').trim())
const sourceOptions = computed(() => {
  try { return JSON.parse(props.optionsJson || '[]') }
  catch { return [] }
})
const phase = computed(() => interaction.value?.phase || 'voting')
const resultsVisible = computed(() => {
  const item = interaction.value
  if (!item) return false
  const policy = item.config?.results
  if (policy === 'presenter' || policy === 'hidden') return false
  if (policy === 'manual' || policy === 'on-close') return item.results_revealed
  return true
})
const timerRemaining = computed(() => {
  const item = interaction.value
  return Number(item?.timer_remaining_seconds ?? item?.config?.timer ?? 0)
})
const timerLabel = computed(() => \`\${Math.floor(timerRemaining.value / 60)}:\${String(Math.max(0, timerRemaining.value % 60)).padStart(2, '0')}\`)
const choiceRows = computed(() => {
  const item = interaction.value
  const options = item?.options?.length ? item.options : sourceOptions.value
  const total = Math.max(0, Number(item?.result?.count || 0))
  return options.map(option => {
    const count = Math.max(0, Number(item?.result?.counts?.[option.id] || 0))
    return {
      ...option,
      count,
      percentage: total > 0 ? Math.min(100, count / total * 100) : 0,
    }
  })
})

const summary = computed(() => {
  const item = interaction.value
  if (!item) return 'Waiting for responses'
  const result = item.result || {}
  if (props.kind !== 'ranked-list' && !resultsVisible.value)
    return \`\${result.count || 0} responses · Results hidden\`
  if (props.kind === 'poll' || props.kind === 'quiz' || props.kind === 'reaction')
    return \`\${result.count || 0} responses\`
  if (props.kind === 'rating')
    return result.average == null ? 'No ratings yet' : \`Average \${Number(result.average).toFixed(1)} · \${result.count} responses\`
  if (props.kind === 'number')
    return result.average == null ? 'No estimates yet' : \`Average \${Number(result.average).toFixed(1)} · Median \${Number(result.median).toFixed(1)}\`
  if (props.kind === 'allocation')
    return \`\${result.count || 0} allocations\`
  if (props.kind === 'matrix')
    return result.average_x == null ? 'No points yet' : \`Room center \${Number(result.average_x).toFixed(1)}, \${Number(result.average_y).toFixed(1)}\`
  if (props.kind === 'ranking')
    return \`\${result.count || 0} complete rankings\`
  if (props.kind === 'image-hotspot')
    return \`\${result.count || 0} placed pins\`
  if (props.kind === 'survey')
    return \`\${result.count || 0} complete surveys\`
  if (props.kind === 'word-cloud') {
    const words = result.submission_count || 0
    const people = result.count || 0
    return \`\${words} word\${words === 1 ? '' : 's'} · \${people} \${people === 1 ? 'person' : 'people'}\`
  }
  if (props.kind === 'ranked-list') {
    const voters = result.count || 0
    if (item.phase !== 'ranked') return \`Voting open · \${item.revealed_count || 0} of \${item.options.length} ideas · \${voters} voter\${voters === 1 ? '' : 's'}\`
    return \`Final ranking · \${voters} voter\${voters === 1 ? '' : 's'}\`
  }
  return \`\${result.count || 0} responses\`
})

const rankedOptions = computed(() => {
  const item = interaction.value
  const options = item?.options?.length ? item.options : sourceOptions.value
  if (!item) return options
  const ranking = item.phase === 'ranked' ? item.result?.ranking : null
  if (!Array.isArray(ranking)) return options
  const byId = new Map(options.map(option => [option.id, option]))
  return ranking.map(id => byId.get(id)).filter(Boolean)
})
const resultOrderedOptions = computed(() => {
  const item = interaction.value
  const options = item?.options?.length ? item.options : sourceOptions.value
  const ranking = item?.result?.ranking
  if (!Array.isArray(ranking)) return options
  const byId = new Map(options.map(option => [option.id, option]))
  return ranking.map(id => byId.get(id)).filter(Boolean)
})
const wordCloudPalette = ['#2563eb', '#7c3aed', '#db2777', '#059669', '#d97706', '#dc2626']
const wordCloudRotations = [0, 0, 0, 0, 0, 90, -90]
const wordCloudWords = shallowRef([])
const wordCloudSource = computed(() => interaction.value?.result?.words || [])
const wordCloudSignature = computed(() => JSON.stringify(
  wordCloudSource.value.map(word => [String(word.text || ''), Number(word.count || 0)]),
))
const wordCloudStyle = computed(() => ({ height: props.cloudHeight, width: props.cloudWidth }))
function stableWordHash(text) {
  let hash = 2166136261
  for (let index = 0; index < text.length; index += 1)
    hash = Math.imul(hash ^ text.charCodeAt(index), 16777619)
  return hash >>> 0
}
watch(wordCloudSignature, () => {
  wordCloudWords.value = wordCloudSource.value
    .map((word) => {
      const text = String(word.text || '').trim()
      const hash = stableWordHash(text)
      return {
        text,
        weight: Math.max(1, Number(word.count || 0)),
        color: wordCloudPalette[hash % wordCloudPalette.length],
        rotation: wordCloudRotations[(hash >>> 8) % wordCloudRotations.length],
        rotationUnit: 'deg',
        fontFamily: 'Inter',
        fontWeight: 700,
      }
    })
    .filter(word => word.text)
}, { immediate: true })
function matrixPointStyle(point) {
  const config = interaction.value?.config || {}
  const xMin = Number(config['x-min'] || 0)
  const xMax = Number(config['x-max'] || 10)
  const yMin = Number(config['y-min'] || 0)
  const yMax = Number(config['y-max'] || 10)
  return {
    left: ((Number(point.x) - xMin) / (xMax - xMin) * 100) + '%',
    bottom: ((Number(point.y) - yMin) / (yMax - yMin) * 100) + '%',
  }
}
function normalizedPointStyle(point) {
  return { left: (Number(point.x) * 100) + '%', top: (Number(point.y) * 100) + '%' }
}
</script>

<template>
  <section :class="['interdeck-interaction', { 'is-slide': display === 'slide', 'is-ranked-list': kind === 'ranked-list' }]">
    <h2 v-if="showTitle && title" class="interdeck-interaction-title">{{ title }}</h2>
    <div class="interdeck-content"><slot /></div>
    <div class="interdeck-live-row">
      <span class="interdeck-live-dot" />
      <span>{{ summary }}</span>
    </div>
    <div v-if="interaction?.config?.timer" :class="['interdeck-timer', { elapsed: timerRemaining === 0 }]">{{ timerLabel }}</div>
    <div v-if="kind === 'poll' || kind === 'quiz' || kind === 'reaction'" class="interdeck-bars">
      <div v-for="option in choiceRows" :key="option.id" :class="['interdeck-bar-row', { correct: resultsVisible && kind === 'quiz' && interaction?.result.correct_option_id === option.id }]" :aria-label="resultsVisible ? option.label + ': ' + option.count + ' votes, ' + Math.round(option.percentage) + ' percent' : option.label">
        <i class="interdeck-bar-fill" :style="{ width: (resultsVisible ? option.percentage : 0) + '%' }" />
        <span>{{ option.label }}</span>
        <strong v-if="resultsVisible">{{ option.count }} <small>{{ Math.round(option.percentage) }}%</small></strong>
      </div>
    </div>
    <div v-if="interaction && resultsVisible && kind === 'image-choice'" class="interdeck-image-choices">
      <article v-for="option in interaction.options" :key="option.id">
        <img :src="option.image_url" :alt="option.label">
        <span>{{ option.label }}</span>
        <strong>{{ interaction.result.counts?.[option.id] || 0 }}</strong>
      </article>
    </div>
    <div v-if="interaction && resultsVisible && kind === 'allocation'" class="interdeck-bars">
      <div v-for="option in interaction.options" :key="option.id" class="interdeck-bar-row">
        <span>{{ option.label }}</span>
        <strong>{{ Number(interaction.result.averages?.[option.id] || 0).toFixed(1) }}</strong>
      </div>
    </div>
    <div v-if="interaction && resultsVisible && kind === 'matrix'" class="interdeck-matrix">
      <i v-for="(point, index) in interaction.result.points" :key="index" :style="matrixPointStyle(point)" />
      <span class="x-label">{{ interaction.config['x-label'] || 'Horizontal' }}</span>
      <span class="y-label">{{ interaction.config['y-label'] || 'Vertical' }}</span>
    </div>
    <ol v-if="interaction && resultsVisible && kind === 'ranking'" class="interdeck-result-ranking">
      <li v-for="(option, index) in resultOrderedOptions" :key="option.id">
        <b>{{ index + 1 }}</b><span>{{ option.label }}</span>
        <strong v-if="interaction.result.count">avg {{ Number(interaction.result.scores?.[option.id]?.average_rank || 0).toFixed(1) }}</strong>
      </li>
    </ol>
    <div v-if="interaction && resultsVisible && kind === 'image-hotspot'" class="interdeck-hotspot">
      <img :src="interaction.config.image" :alt="interaction.config.alt">
      <i v-for="(point, index) in interaction.result.points" :key="index" :style="normalizedPointStyle(point)" />
    </div>
    <div v-if="interaction && resultsVisible && kind === 'survey'" class="interdeck-survey-results">
      <article v-for="question in interaction.config.questions" :key="question.id">
        <span>{{ question.label }}</span>
        <strong v-if="question.type === 'rating'">{{ interaction.result.questions?.[question.id]?.average == null ? '—' : Number(interaction.result.questions[question.id].average).toFixed(1) }}</strong>
        <div v-else-if="question.type === 'choice'">
          <small v-for="option in question.options" :key="option.id">{{ option.label }} · {{ interaction.result.questions?.[question.id]?.counts?.[option.id] || 0 }}</small>
        </div>
        <strong v-else>{{ interaction.result.questions?.[question.id]?.count || 0 }} responses</strong>
      </article>
    </div>
    <VueWordCloud v-if="interaction && resultsVisible && kind === 'word-cloud'" class="interdeck-words" :style="wordCloudStyle" :words="wordCloudWords" :font-size-ratio="3.5" :spacing="cloudSpacing" :animation-duration="350" :animation-overlap="1" font-family="Inter" font-weight="700" role="img" :aria-label="'Word cloud with ' + wordCloudWords.length + ' entries'">
      <template #default="{ text, weight }"><span :title="text + ' · ' + weight + (weight === 1 ? ' person' : ' people')">{{ text }}</span></template>
    </VueWordCloud>
    <TransitionGroup v-if="kind === 'ranked-list' && reveal === 'click'" name="interdeck-rank" tag="ol" class="interdeck-ranked-options">
      <li v-for="(option, index) in rankedOptions" v-click :key="option.id">
        <span class="interdeck-rank-number">{{ phase === 'ranked' ? index + 1 : '•' }}</span>
        <span>{{ option.label }}</span>
        <span v-if="phase === 'ranked'" class="interdeck-vote-totals">
          <strong class="up">▲ {{ interaction?.result.scores?.[option.id]?.up || 0 }}</strong>
          <strong class="down">▼ {{ interaction?.result.scores?.[option.id]?.down || 0 }}</strong>
        </span>
      </li>
    </TransitionGroup>
    <TransitionGroup v-else-if="kind === 'ranked-list'" name="interdeck-rank" tag="ol" class="interdeck-ranked-options">
      <li v-for="(option, index) in rankedOptions" :key="option.id">
        <span class="interdeck-rank-number">{{ phase === 'ranked' ? index + 1 : '•' }}</span>
        <span>{{ option.label }}</span>
        <span v-if="phase === 'ranked'" class="interdeck-vote-totals">
          <strong class="up">▲ {{ interaction?.result.scores?.[option.id]?.up || 0 }}</strong>
          <strong class="down">▼ {{ interaction?.result.scores?.[option.id]?.down || 0 }}</strong>
        </span>
      </li>
    </TransitionGroup>
  </section>
</template>

<style scoped>
.interdeck-interaction { border: 2px solid color-mix(in srgb, currentColor 15%, transparent); border-radius: 18px; padding: 20px 24px; background: color-mix(in srgb, var(--slidev-theme-primary, #6366f1) 6%, transparent); }
.interdeck-interaction.is-slide { display: flex; flex-direction: column; min-height: 100%; padding: 0; border: 0; border-radius: 0; background: transparent; }
.interdeck-interaction-title { margin: 0 0 12px; font-size: 1.18em; line-height: 1.12; }
.is-slide .interdeck-live-row { margin-top: auto; }
.interdeck-live-row { display: flex; align-items: center; gap: 8px; font-size: 0.68em; opacity: .7; margin-top: 12px; }
.interdeck-live-dot { width: 8px; height: 8px; border-radius: 50%; background: #22c55e; box-shadow: 0 0 0 4px color-mix(in srgb, #22c55e 20%, transparent); }
.interdeck-timer { align-self: flex-start; margin-top: 12px; padding: 5px 10px; border-radius: 9px; color: var(--slidev-theme-primary, #6366f1); background: color-mix(in srgb, var(--slidev-theme-primary, #6366f1) 12%, transparent); font-family: ui-monospace, monospace; font-size: .9em; font-weight: 800; font-variant-numeric: tabular-nums; }
.interdeck-timer.elapsed { color: #dc2626; background: color-mix(in srgb, #dc2626 10%, transparent); }
.interdeck-bars { display: grid; gap: 7px; margin-top: 12px; font-size: .72em; }
.interdeck-bar-row { position: relative; display: flex; justify-content: space-between; gap: 20px; padding: 9px 11px; border-radius: 8px; background: color-mix(in srgb, currentColor 6%, transparent); overflow: hidden; }
.interdeck-bar-row > span, .interdeck-bar-row > strong { position: relative; z-index: 1; }
.interdeck-bar-row > strong { display: inline-flex; align-items: baseline; gap: 7px; font-variant-numeric: tabular-nums; }
.interdeck-bar-row > strong small { font-size: .68em; opacity: .65; }
.interdeck-bar-fill { position: absolute; inset: 0 auto 0 0; width: 0; border-radius: inherit; background: color-mix(in srgb, var(--slidev-theme-primary, #6366f1) 22%, transparent); transition: width .45s cubic-bezier(.2,.8,.2,1); }
.interdeck-bar-row.correct { outline: 2px solid #22c55e; }
.interdeck-bar-row.correct .interdeck-bar-fill { background: color-mix(in srgb, #22c55e 22%, transparent); }
.interdeck-image-choices { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; margin-top: 14px; }
.interdeck-image-choices article { display: grid; grid-template-columns: 1fr auto; gap: 5px; border-radius: 12px; background: color-mix(in srgb, currentColor 6%, transparent); overflow: hidden; }
.interdeck-image-choices img { grid-column: 1 / -1; width: 100%; height: 110px; object-fit: cover; }
.interdeck-image-choices span { padding: 4px 0 8px 10px; font-size: .65em; }
.interdeck-image-choices strong { padding: 4px 10px 8px 0; color: var(--slidev-theme-primary, #6366f1); }
.interdeck-matrix { position: relative; height: 250px; margin: 20px 20px 28px 34px; border-left: 2px solid currentColor; border-bottom: 2px solid currentColor; background: linear-gradient(to right, transparent 49.7%, color-mix(in srgb, currentColor 15%, transparent) 50%, transparent 50.3%), linear-gradient(to top, transparent 49.7%, color-mix(in srgb, currentColor 15%, transparent) 50%, transparent 50.3%); }
.interdeck-matrix i { position: absolute; width: 10px; height: 10px; border-radius: 50%; background: var(--slidev-theme-primary, #6366f1); transform: translate(-50%, 50%); opacity: .55; }
.interdeck-matrix .x-label { position: absolute; right: 0; bottom: -25px; font-size: .52em; }
.interdeck-matrix .y-label { position: absolute; top: 0; left: -32px; font-size: .52em; writing-mode: vertical-rl; transform: rotate(180deg); }
.interdeck-result-ranking { display: grid; gap: 8px; padding: 0; margin: 14px 0 0; list-style: none; }
.interdeck-result-ranking li { display: grid; grid-template-columns: 30px 1fr auto; align-items: center; gap: 10px; padding: 8px 11px; border-radius: 9px; background: color-mix(in srgb, currentColor 6%, transparent); }
.interdeck-result-ranking b { display: grid; place-items: center; width: 25px; height: 25px; border-radius: 50%; color: white; background: var(--slidev-theme-primary, #6366f1); font-size: .58em; }
.interdeck-result-ranking strong { font-size: .58em; opacity: .7; }
.interdeck-hotspot { position: relative; align-self: center; max-width: 100%; margin-top: 14px; overflow: hidden; }
.interdeck-hotspot img { display: block; max-width: 100%; max-height: 310px; object-fit: contain; }
.interdeck-hotspot i { position: absolute; width: 12px; height: 12px; border: 2px solid white; border-radius: 50%; background: var(--slidev-theme-primary, #6366f1); box-shadow: 0 2px 5px #0005; transform: translate(-50%, -50%); opacity: .65; }
.interdeck-survey-results { display: grid; gap: 8px; margin-top: 14px; }
.interdeck-survey-results article { display: grid; grid-template-columns: minmax(0, 1fr) auto; align-items: center; gap: 8px; padding: 9px 11px; border-radius: 9px; background: color-mix(in srgb, currentColor 6%, transparent); }
.interdeck-survey-results article > span { font-size: .65em; }
.interdeck-survey-results article > strong { color: var(--slidev-theme-primary, #6366f1); }
.interdeck-survey-results article > div { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 4px 9px; max-width: 55%; }
.interdeck-survey-results small { font-size: .48em; opacity: .75; }
.interdeck-words { display: block; flex: none; max-width: 100%; min-height: 180px; margin-top: 14px; overflow: hidden; }
.interdeck-ranked-options { display: grid; gap: 10px; padding: 0; margin: 18px 0 0; list-style: none; }
.interdeck-ranked-options li { display: grid; grid-template-columns: 34px 1fr auto; align-items: center; gap: 12px; min-height: 48px; padding: 8px 14px; border-radius: 12px; background: color-mix(in srgb, currentColor 7%, transparent); transition: transform .55s ease, background .3s ease; }
.interdeck-rank-number { display: grid; place-items: center; width: 28px; height: 28px; border-radius: 50%; color: white; background: var(--slidev-theme-primary, #6366f1); font-size: .66em; font-weight: 800; }
.interdeck-ranked-options strong { min-width: 38px; text-align: right; color: var(--slidev-theme-primary, #6366f1); }
.interdeck-vote-totals { display: inline-flex; align-items: center; gap: 7px; font-size: .76em; font-variant-numeric: tabular-nums; }
.interdeck-vote-totals strong { min-width: 0; padding: 3px 7px; border-radius: 999px; text-align: center; }
.interdeck-vote-totals .up { color: #15803d; background: color-mix(in srgb, #22c55e 14%, transparent); }
.interdeck-vote-totals .down { color: #dc2626; background: color-mix(in srgb, #ef4444 13%, transparent); }
.interdeck-rank-move { transition: transform .65s cubic-bezier(.2,.8,.2,1); }
</style>
`

export const elementVoteComponent = `<script setup>
import { computed } from 'vue'
import { useInterdeckLive } from '../composables/useInterdeckLive.js'
const props = defineProps({ deckId: String, interactionId: String, kind: { type: String, default: 'updown' }, optionsJson: { type: String, default: '[]' } })
const interaction = useInterdeckLive(props.deckId, props.interactionId)
const count = computed(() => interaction.value?.result?.count || 0)
const up = computed(() => interaction.value?.result?.up || 0)
const down = computed(() => interaction.value?.result?.down || 0)
const average = computed(() => interaction.value?.result?.average)
const rating = computed(() => average.value == null ? '—' : Number(average.value).toFixed(1))
const fallbackOptions = computed(() => {
  try { return JSON.parse(props.optionsJson || '[]') }
  catch { return [] }
})
const reactionOptions = computed(() => interaction.value?.options?.length ? interaction.value.options : fallbackOptions.value)
const reactions = computed(() => reactionOptions.value.map(option => ({
  ...option,
  count: Number(interaction.value?.result?.counts?.[option.id] || 0),
})).filter(option => option.count > 0))
const outcomeTitle = computed(() => {
  if (props.kind === 'rating') return count.value + ' rating responses'
  if (props.kind === 'reaction') return count.value + ' reaction responses'
  return count.value + ' responses · ' + up.value + ' upvotes · ' + down.value + ' downvotes'
})
</script>
<template>
  <span v-if="kind === 'rating'" class="element-vote rating" :title="outcomeTitle" :aria-label="outcomeTitle">★ {{ rating }}</span>
  <span v-else-if="kind === 'reaction' && reactions.length" class="element-reactions" :title="outcomeTitle" :aria-label="outcomeTitle">
    <span v-for="option in reactions" :key="option.id" class="element-reaction"><b>{{ option.label }}</b> {{ option.count }}</span>
  </span>
  <span v-else class="element-vote-group" :title="outcomeTitle" :aria-label="outcomeTitle">
    <span class="element-vote up">▲ {{ up }}</span>
    <span class="element-vote down">▼ {{ down }}</span>
  </span>
</template>
<style scoped>
.element-vote-group { display: inline-flex; align-items: center; gap: 5px; margin-left: 8px; vertical-align: middle; }
.element-reactions { display: inline-flex; flex-wrap: wrap; align-items: center; gap: 5px; margin-left: 8px; vertical-align: middle; }
.element-reaction { display: inline-flex; align-items: center; gap: 3px; padding: 2px 8px; border-radius: 999px; background: color-mix(in srgb, var(--slidev-theme-primary, #6366f1) 12%, transparent); font-size: .55em; font-weight: 750; font-variant-numeric: tabular-nums; }
.element-reaction b { font-size: 1.2em; line-height: 1; }
.element-vote { display: inline-flex; align-items: center; padding: 2px 8px; border-radius: 999px; font-size: .55em; font-weight: 750; font-variant-numeric: tabular-nums; }
.element-vote.rating { margin-left: 8px; vertical-align: middle; }
.element-vote.up { color: #15803d; background: color-mix(in srgb, #22c55e 14%, transparent); }
.element-vote.down { color: #dc2626; background: color-mix(in srgb, #ef4444 13%, transparent); }
.element-vote.rating { color: var(--slidev-theme-primary, #6366f1); background: color-mix(in srgb, var(--slidev-theme-primary, #6366f1) 12%, transparent); }
</style>
`

export const liveQaComponent = `<script setup>
import { computed, ref } from 'vue'
import { useInterdeckState } from '../composables/useInterdeckLive.js'

const props = defineProps({
  deckId: String,
  maxThemes: { type: Number, default: 8 },
  show: { type: String, default: 'open' },
})
const state = useInterdeckState(props.deckId)
const scrollRegion = ref(null)
const eligibleQuestions = computed(() => (state.value?.questions || []).filter(question => (
  question.moderation_status === 'approved'
  && question.lifecycle_status !== 'archived'
  && (props.show === 'all' || question.lifecycle_status === 'open')
)))
function compareQuestions(left, right) {
  return Number(right.is_pinned) - Number(left.is_pinned)
    || (right.priority_score || 0) - (left.priority_score || 0)
    || new Date(left.created_at).getTime() - new Date(right.created_at).getTime()
}
const verbatimQuestions = computed(() => [...eligibleQuestions.value].sort(compareQuestions))
const questionById = computed(() => new Map(eligibleQuestions.value.map(question => [question.id, question])))
const groupedIds = computed(() => new Set(
  (state.value?.ai_insights?.themes || []).flatMap(theme => theme.question_ids || []),
))
const themes = computed(() => {
  const groups = (state.value?.ai_insights?.themes || []).map(theme => {
    const questions = (theme.question_ids || [])
      .map(id => questionById.value.get(id))
      .filter(Boolean)
      .sort(compareQuestions)
    return {
      label: theme.label,
      summary: theme.summary,
      representativeQuestion: theme.representative_question || questions[0]?.body || theme.summary,
      questions,
      votes: questions.reduce((total, question) => total + question.votes, 0),
      priority: questions.reduce((total, question) => total + (question.priority_score || 0), 0),
      pinned: questions.some(question => question.is_pinned),
    }
  }).filter(theme => theme.questions.length)

  const ungrouped = eligibleQuestions.value
    .filter(question => !groupedIds.value.has(question.id))
    .sort(compareQuestions)
  groups.push(...ungrouped.map(question => ({
    label: state.value?.ai_configured ? 'New question' : 'Question',
    summary: '',
    representativeQuestion: question.body,
    questions: [question],
    votes: question.votes,
    priority: question.priority_score || 0,
    pinned: question.is_pinned,
  })))
  return groups
    .sort((left, right) => Number(right.pinned) - Number(left.pinned) || right.priority - left.priority || right.votes - left.votes)
    .slice(0, Math.max(1, Math.min(props.maxThemes, 8)))
})
const displayMode = computed(() => state.value?.qa_display_mode || 'verbatim')
const hasItems = computed(() => displayMode.value === 'ai_grouped' ? themes.value.length > 0 : verbatimQuestions.value.length > 0)
const questionCount = computed(() => eligibleQuestions.value.length)
function scopeLabel(question) {
  return question.slide_number ? \`Slide \${question.slide_number}\` : 'Deck-wide'
}
function scrollQuestions(event) {
  const region = scrollRegion.value
  if (!region || !['ArrowDown', 'ArrowUp', 'PageDown', 'PageUp', 'Home', 'End'].includes(event.key)) return
  event.preventDefault()
  event.stopPropagation()
  if (event.key === 'Home') region.scrollTo({ top: 0, behavior: 'smooth' })
  else if (event.key === 'End') region.scrollTo({ top: region.scrollHeight, behavior: 'smooth' })
  else region.scrollBy({
    top: (event.key === 'ArrowDown' ? 80 : event.key === 'ArrowUp' ? -80 : event.key === 'PageDown' ? region.clientHeight * .8 : -region.clientHeight * .8),
    behavior: 'smooth',
  })
}
</script>

<template>
  <section class="interdeck-live-qa" aria-live="polite">
    <header>
      <div><i /><span>{{ displayMode === 'ai_grouped' ? 'AI-GROUPED Q&A' : 'APPROVED QUESTIONS' }}</span></div>
      <strong>{{ questionCount }} question{{ questionCount === 1 ? '' : 's' }}</strong>
    </header>
    <div v-if="hasItems" ref="scrollRegion" class="qa-scroll" tabindex="0" aria-label="Scrollable questions" @keydown="scrollQuestions" @wheel.stop>
      <div v-if="displayMode === 'verbatim'" class="qa-list">
        <article v-for="(question, index) in verbatimQuestions" :key="question.id" :class="{ pinned: question.is_pinned }">
          <div class="question-rank">{{ index + 1 }}</div>
          <div class="question-copy">
            <p>{{ question.body }}</p>
            <div class="question-details">
              <strong>{{ question.display_name || 'Anonymous' }}</strong>
              <span>{{ scopeLabel(question) }}</span>
              <span v-if="question.is_novel" class="novel">New topic</span>
              <span v-else-if="question.repeat_count > 1">Asked {{ question.repeat_count }} times</span>
            </div>
          </div>
          <b class="question-votes">▲ {{ question.votes }}</b>
        </article>
      </div>
      <div v-else class="qa-list qa-groups">
        <article v-for="(theme, index) in themes" :key="theme.label + index" :class="{ pinned: theme.pinned }">
          <div class="question-rank">{{ index + 1 }}</div>
          <div class="question-copy">
            <small class="theme-label">{{ theme.label }}</small>
            <p>{{ theme.representativeQuestion }}</p>
            <div class="question-details">
              <span>{{ theme.questions.length }} source question{{ theme.questions.length === 1 ? '' : 's' }}</span>
              <span v-for="question in theme.questions" :key="question.id">{{ question.display_name || 'Anonymous' }} · {{ scopeLabel(question) }}</span>
            </div>
          </div>
          <b class="question-votes">▲ {{ theme.votes }}</b>
        </article>
      </div>
    </div>
    <div v-else class="qa-empty">
      <i>?</i><strong>Waiting for questions</strong><span>Questions appear here as soon as they pass moderation.</span>
    </div>
    <footer>
      <span>{{ displayMode === 'ai_grouped' ? 'Rephrased by Gemini · source questions retained' : 'Verbatim · pinned, novel, upvoted, then named' }}</span>
      <small>{{ hasItems ? 'Focus this list and use ↑ ↓ or Page Up/Down to scroll' : 'Live moderated Q&A' }}</small>
    </footer>
  </section>
</template>

<style scoped>
.interdeck-live-qa { display: grid; grid-template-rows: auto minmax(0, 1fr) auto; gap: 12px; width: 100%; height: 360px; min-height: 0; max-height: 360px; }
.interdeck-live-qa > header, .interdeck-live-qa > footer { display: flex; align-items: center; justify-content: space-between; gap: 18px; }
.interdeck-live-qa > header > div { display: flex; align-items: center; gap: 9px; color: var(--slidev-theme-primary, #6366f1); font-size: .58em; font-weight: 800; letter-spacing: .1em; }
.interdeck-live-qa > header i { width: 9px; height: 9px; border-radius: 50%; background: #22c55e; box-shadow: 0 0 0 5px color-mix(in srgb, #22c55e 18%, transparent); }
.interdeck-live-qa > header strong { padding: 5px 10px; border-radius: 999px; background: color-mix(in srgb, currentColor 7%, transparent); font-size: .55em; }
.qa-scroll { min-height: 0; overflow-y: auto; overscroll-behavior: contain; scroll-snap-type: y proximity; scrollbar-gutter: stable; scrollbar-color: color-mix(in srgb, var(--slidev-theme-primary, #6366f1) 55%, transparent) transparent; padding: 2px 10px 2px 2px; outline: none; touch-action: pan-y; }
.qa-scroll:focus-visible { border-radius: 12px; box-shadow: 0 0 0 3px color-mix(in srgb, var(--slidev-theme-primary, #6366f1) 22%, transparent); }
.qa-list { display: grid; grid-template-columns: 1fr; align-content: start; gap: 10px; }
.qa-list article { display: grid; grid-template-columns: 36px minmax(0, 1fr) auto; gap: 13px; align-items: start; padding: 14px 16px; border: 1px solid color-mix(in srgb, currentColor 13%, transparent); border-radius: 14px; background: color-mix(in srgb, currentColor 4%, transparent); scroll-margin-top: 2px; scroll-snap-align: start; }
.qa-list article.pinned { border-color: color-mix(in srgb, var(--slidev-theme-primary, #6366f1) 48%, transparent); box-shadow: inset 3px 0 var(--slidev-theme-primary, #6366f1); }
.question-rank { display: grid; place-items: center; width: 32px; height: 32px; border-radius: 50%; color: white; background: var(--slidev-theme-primary, #6366f1); font-size: .59em; font-weight: 850; }
.question-copy { min-width: 0; }
.question-copy > p { margin: 0; font-size: .72em; font-weight: 680; line-height: 1.32; }
.question-details { display: flex; flex-wrap: wrap; align-items: center; gap: 5px 9px; margin-top: 9px; font-size: .46em; opacity: .67; }
.question-details strong { opacity: 1; }
.question-details .novel { padding: 2px 6px; border-radius: 999px; color: #15803d; background: color-mix(in srgb, #22c55e 15%, transparent); font-weight: 800; opacity: 1; }
.question-votes { flex: none; color: var(--slidev-theme-primary, #6366f1); font-size: .58em; }
.theme-label { display: block; margin-bottom: 4px; color: var(--slidev-theme-primary, #6366f1); font-size: .47em; font-weight: 850; letter-spacing: .06em; text-transform: uppercase; }
.qa-empty { display: grid; place-items: center; align-content: center; gap: 7px; min-height: 300px; border: 2px dashed color-mix(in srgb, currentColor 12%, transparent); border-radius: 18px; text-align: center; }
.qa-empty i { display: grid; place-items: center; width: 48px; height: 48px; border-radius: 50%; color: white; background: var(--slidev-theme-primary, #6366f1); font-style: normal; font-weight: 850; }
.qa-empty strong { font-size: .8em; }
.qa-empty span { max-width: 440px; font-size: .55em; opacity: .65; }
.interdeck-live-qa > footer { padding-top: 9px; border-top: 1px solid color-mix(in srgb, currentColor 10%, transparent); font-size: .46em; opacity: .6; }
</style>
`

export const liveSummaryComponent = `<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

const props = defineProps({
  deckId: String,
  show: { type: String, default: 'responded' },
})
const report = ref(null)
const error = ref('')
const loading = ref(true)
const scrollRegion = ref(null)
let refreshTimer = 0
const interactions = computed(() => (report.value?.interactions || []).filter(item => props.show === 'all' || item.response_count > 0))
const hasQuestions = computed(() => (report.value?.questions?.approved || 0) > 0)
const labels = { 'word-cloud': 'Word cloud', 'free-text': 'Written response', 'ranked-list': 'Idea voting', 'image-choice': 'Image choice', 'image-hotspot': 'Image hotspot', updown: 'Up/down vote' }
const kindLabel = kind => labels[kind] || kind.replaceAll('-', ' ')

async function refresh() {
  try {
    const response = await fetch('/api/decks/' + props.deckId + '/live-summary', { credentials: 'include' })
    if (!response.ok) throw new Error('The live summary is temporarily unavailable.')
    report.value = await response.json()
    error.value = ''
  }
  catch (reason) { error.value = reason instanceof Error ? reason.message : String(reason) }
  finally { loading.value = false }
}
function scrollReport(event) {
  const region = scrollRegion.value
  if (!region || !['ArrowDown', 'ArrowUp', 'PageDown', 'PageUp', 'Home', 'End'].includes(event.key)) return
  event.preventDefault()
  event.stopPropagation()
  if (event.key === 'Home') region.scrollTo({ top: 0, behavior: 'smooth' })
  else if (event.key === 'End') region.scrollTo({ top: region.scrollHeight, behavior: 'smooth' })
  else region.scrollBy({ top: event.key === 'ArrowDown' ? 90 : event.key === 'ArrowUp' ? -90 : event.key === 'PageDown' ? region.clientHeight * .8 : -region.clientHeight * .8, behavior: 'smooth' })
}
onMounted(() => { refresh(); refreshTimer = window.setInterval(refresh, 5000) })
onBeforeUnmount(() => window.clearInterval(refreshTimer))
</script>

<template>
  <section class="interdeck-live-summary" aria-live="polite">
    <header><div><i /><span>ROOM SUMMARY</span></div><strong v-if="report">Results session {{ report.result_epoch }}</strong></header>
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
          <h3>{{ item.title || 'Interaction on slide ' + item.slide_number }}</h3><p>{{ item.headline }}</p>
          <div v-if="item.highlights.length" class="summary-highlights"><span v-for="highlight in item.highlights" :key="highlight">{{ highlight }}</span></div>
        </article>
        <article v-if="hasQuestions" class="questions-card">
          <div class="summary-meta"><span>Deck-wide</span><small>Q&amp;A</small><b>{{ report.questions.approved }}</b></div>
          <h3>Questions from the room</h3><p>{{ report.questions.answered }} answered · {{ report.questions.total_votes }} audience upvotes</p>
          <ol><li v-for="question in report.questions.top.slice(0, 8)" :key="question.id"><span>{{ question.body }}</span><small>{{ question.display_name || 'Anonymous' }}<template v-if="question.slide_number"> · Slide {{ question.slide_number }}</template> · ▲ {{ question.votes }}</small></li></ol>
        </article>
        <div v-if="!interactions.length && !hasQuestions" class="summary-empty">No responses have been collected in this results session yet.</div>
      </div>
    </div>
    <footer><span>Current epoch · refreshes every 5 seconds</span><small>Focus this report and use ↑ ↓ or Page Up/Down to scroll</small></footer>
  </section>
</template>

<style scoped>
.interdeck-live-summary { display: grid; grid-template-rows: auto auto minmax(0, 1fr) auto; gap: 11px; width: 100%; height: 420px; min-height: 0; max-height: 420px; }
.interdeck-live-summary > header, .interdeck-live-summary > footer { display: flex; align-items: center; justify-content: space-between; gap: 18px; }
.interdeck-live-summary > header > div { display: flex; align-items: center; gap: 9px; color: var(--slidev-theme-primary, #6366f1); font-size: .54em; font-weight: 850; letter-spacing: .12em; }
.interdeck-live-summary > header i { width: 9px; height: 9px; border-radius: 50%; background: #22c55e; box-shadow: 0 0 0 5px color-mix(in srgb, #22c55e 18%, transparent); }
.interdeck-live-summary > header strong { padding: 5px 10px; border-radius: 999px; background: color-mix(in srgb, currentColor 7%, transparent); font-size: .48em; }
.summary-metrics { display: grid; grid-template-columns: repeat(3, 1fr); gap: 9px; }.summary-metrics > div { display: flex; align-items: baseline; gap: 7px; padding: 8px 11px; border-radius: 11px; background: color-mix(in srgb, var(--slidev-theme-primary, #6366f1) 8%, transparent); }
.summary-metrics b { color: var(--slidev-theme-primary, #6366f1); font-size: .72em; }.summary-metrics span { font-size: .42em; opacity: .68; }
.summary-scroll { min-height: 0; padding: 1px 9px 1px 1px; overflow-y: auto; overscroll-behavior: contain; scroll-snap-type: y proximity; scrollbar-gutter: stable; outline: none; touch-action: pan-y; }.summary-scroll:focus-visible { border-radius: 12px; box-shadow: 0 0 0 3px color-mix(in srgb, var(--slidev-theme-primary, #6366f1) 22%, transparent); }
.summary-list { display: grid; gap: 9px; }.summary-list article { padding: 12px 14px; border: 1px solid color-mix(in srgb, currentColor 13%, transparent); border-radius: 14px; background: color-mix(in srgb, currentColor 3%, transparent); scroll-snap-align: start; }
.summary-meta { display: flex; align-items: center; gap: 8px; margin-bottom: 5px; font-size: .4em; text-transform: uppercase; letter-spacing: .06em; opacity: .65; }.summary-meta span { color: var(--slidev-theme-primary, #6366f1); font-weight: 850; }.summary-meta b { margin-left: auto; }
.summary-list h3 { margin: 0; font-size: .66em; line-height: 1.2; }.summary-list p { margin: 3px 0 0; font-size: .5em; line-height: 1.35; opacity: .78; }.summary-highlights { display: flex; flex-wrap: wrap; gap: 5px; margin-top: 8px; }.summary-highlights span { padding: 3px 7px; border-radius: 999px; background: color-mix(in srgb, var(--slidev-theme-primary, #6366f1) 9%, transparent); font-size: .38em; }
.questions-card ol { display: grid; gap: 6px; margin: 9px 0 0; padding-left: 1.2em; }.questions-card li { padding-left: 4px; font-size: .46em; }.questions-card li span, .questions-card li small { display: block; }.questions-card li small { margin-top: 2px; opacity: .58; }
.summary-empty { display: grid; min-height: 140px; place-items: center; padding: 20px; border: 2px dashed color-mix(in srgb, currentColor 13%, transparent); border-radius: 14px; font-size: .56em; text-align: center; opacity: .62; }.interdeck-live-summary > footer { padding-top: 7px; border-top: 1px solid color-mix(in srgb, currentColor 10%, transparent); font-size: .39em; opacity: .55; }
</style>
`

export const liveStateComposable = `import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

const stores = new Map()

function getStore(deckId) {
  if (stores.has(deckId)) return stores.get(deckId)
  const state = ref(null)
  const store = { state, users: 0, timer: 0, refreshing: false }
  store.refresh = async () => {
    if (store.refreshing) return
    store.refreshing = true
    try {
      const response = await fetch(\`/api/decks/\${deckId}/live-state\`, { credentials: 'include' })
      if (response.ok) state.value = await response.json()
    } catch {}
    finally { store.refreshing = false }
  }
  stores.set(deckId, store)
  return store
}

function useStore(deckId) {
  const store = getStore(deckId)
  onMounted(() => {
    store.users += 1
    if (!store.timer) {
      store.refresh()
      store.timer = window.setInterval(store.refresh, 1000)
    }
  })
  onBeforeUnmount(() => {
    store.users -= 1
    if (store.users <= 0 && store.timer) {
      window.clearInterval(store.timer)
      store.timer = 0
    }
  })
  return store
}

export function useInterdeckState(deckId) {
  const store = useStore(deckId)
  return computed(() => store.state.value)
}

export function useInterdeckLive(deckId, interactionId) {
  const store = useStore(deckId)
  return computed(() => store.state.value?.interactions?.find(item => item.id === interactionId) || null)
}
`

export const navigationBridgeComponent = `<script setup>
import { sharedState, useNav } from '@slidev/client'
import { onBeforeUnmount, onMounted, watch } from 'vue'

const nav = useNav()
// Interdeck owns presenter synchronization. Slidev's development-server
// synchronization uses a root-relative endpoint that is intentionally not
// exposed through the protected per-deck gateway.
sharedState.$paused = true

function report(commandId) {
  window.parent.postMessage({
    type: 'interdeck:navigation',
    slideNumber: nav.currentSlideNo.value,
    clickStep: nav.clicks.value,
    clicksTotal: nav.clicksTotal.value,
    commandId,
  }, window.location.origin)
}

async function onMessage(event) {
  if (event.origin !== window.location.origin || event.source !== window.parent) return
  const message = event.data
  if (!message || message.type !== 'interdeck:command') return
  if (message.action === 'next') await nav.next()
  else if (message.action === 'prev') await nav.prev()
  else if (message.action === 'go') {
    // A target slide's component-level v-click directives register only after
    // that slide mounts. Navigate to it first, then restore its click position.
    await nav.go(message.slideNumber, 0, true)
    const requestedClicks = message.clickStep || 0
    const deadline = performance.now() + 2000
    while (nav.clicksTotal.value < requestedClicks && performance.now() < deadline)
      await new Promise(resolve => setTimeout(resolve, 16))
    await nav.go(message.slideNumber, requestedClicks, true)
  }
  report(message.commandId)
}

watch([nav.currentSlideNo, nav.clicks], () => report())
onMounted(() => {
  window.addEventListener('message', onMessage)
  window.parent.postMessage({ type: 'interdeck:ready' }, window.location.origin)
})
onBeforeUnmount(() => window.removeEventListener('message', onMessage))
</script>

<template><span aria-hidden="true" /></template>
`

export const qrComponent = `<script setup>
import QrcodeVue from 'qrcode.vue'
import { computed } from 'vue'
import { useInterdeckState } from '../composables/useInterdeckLive.js'
const props = defineProps({ deckId: String, fallbackUrl: String, size: { type: Number, default: 180 } })
const state = useInterdeckState(props.deckId)
const url = computed(() => state.value?.join_code
  ? \`\${window.location.origin}/j/\${state.value.join_code}\`
  : props.fallbackUrl)
const displayUrl = computed(() => url.value.replace(/^https?:\\/\\//, ''))
</script>
<template>
  <figure class="audience-qr" :style="{ '--audience-qr-size': size + 'px' }">
    <div class="qr-shell"><QrcodeVue :value="url" :size="size" level="M" render-as="svg" /></div>
    <figcaption><strong>Join the conversation</strong><span class="audience-url">{{ displayUrl }}</span><span v-if="state?.live" class="audience-total"><b>{{ state.participant_count || 0 }}</b> {{ state.participant_count === 1 ? 'person joined' : 'people joined' }}</span></figcaption>
  </figure>
</template>
<style scoped>
.audience-qr { display: inline-flex; align-items: center; gap: clamp(18px, calc(var(--audience-qr-size) * .11), 42px); margin: 10px 0; }
.qr-shell { padding: clamp(8px, calc(var(--audience-qr-size) * .055), 18px); border-radius: clamp(12px, calc(var(--audience-qr-size) * .075), 24px); background: white; line-height: 0; box-shadow: 0 8px 30px #0002; }
figcaption { display: grid; gap: clamp(6px, calc(var(--audience-qr-size) * .032), 14px); width: max-content; font-size: clamp(20px, calc(var(--audience-qr-size) * .13), 42px); line-height: 1.2; }
figcaption > strong { font-size: 1em; }
figcaption > span { overflow-wrap: anywhere; opacity: .68; font-size: .74em; line-height: 1.35; }
.audience-url { white-space: nowrap; }
.audience-total { display: inline-flex; align-items: baseline; gap: clamp(5px, calc(var(--audience-qr-size) * .025), 10px); margin-top: clamp(5px, calc(var(--audience-qr-size) * .025), 10px); color: var(--slidev-theme-primary, #6366f1); opacity: 1; font-size: .9em; }
.audience-total b { font-size: 1.4em; line-height: 1; font-variant-numeric: tabular-nums; }
</style>
`

export const audienceCountComponent = `<script setup>
import { computed } from 'vue'
import { useInterdeckState } from '../composables/useInterdeckLive.js'
const props = defineProps({ deckId: String, label: { type: String, default: 'people joined' } })
const state = useInterdeckState(props.deckId)
const count = computed(() => state.value?.live ? Number(state.value.participant_count || 0) : 0)
</script>
<template>
  <figure :class="['audience-count', { live: state?.live }]" aria-live="polite" :aria-label="count + ' ' + label">
    <span>{{ state?.live ? 'LIVE AUDIENCE' : 'WAITING FOR PRESENTER' }}</span>
    <strong>{{ count }}</strong>
    <figcaption>{{ label }}</figcaption>
  </figure>
</template>
<style scoped>
.audience-count { display: inline-grid; justify-items: center; min-width: 250px; padding: 24px 34px; border: 1px solid color-mix(in srgb, currentColor 14%, transparent); border-radius: 18px; background: color-mix(in srgb, currentColor 4%, transparent); text-align: center; }
.audience-count > span { color: var(--slidev-theme-primary, #6366f1); font-size: .42em; font-weight: 850; letter-spacing: .12em; }
.audience-count > strong { margin: 4px 0 0; color: var(--slidev-theme-primary, #6366f1); font-size: 4em; line-height: 1; font-variant-numeric: tabular-nums; transition: transform .2s ease; }
.audience-count figcaption { margin-top: 5px; font-size: .72em; font-weight: 700; opacity: .68; }
.audience-count.live > strong { text-shadow: 0 8px 30px color-mix(in srgb, var(--slidev-theme-primary, #6366f1) 18%, transparent); }
</style>
`
