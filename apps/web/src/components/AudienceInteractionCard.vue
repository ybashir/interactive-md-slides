<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import { put } from '../api'
import type { AudienceInteraction } from '../types'
import PackedWordCloud from './player/PackedWordCloud.vue'

const props = defineProps<{ interaction: AudienceInteraction; joinCode: string }>()
const emit = defineEmits<{
  saved: [details: { interactionId: string; payload: Record<string, unknown>; correctOptionId?: string }]
}>()
const text = ref('')
const sending = ref(false)
const error = ref('')
const online = ref(navigator.onLine)
const saveStatus = ref<'idle' | 'sending' | 'saved' | 'error'>('idle')
const rankedVotes = ref<Record<string, number>>({})
const wordCloudEntries = ref<string[]>([])
const multipleSelectionDraft = ref<string[]>([])
const timerRemaining = ref<number | null>(null)
const numberValue = ref('')
const allocationDraft = ref<Record<string, number>>({})
const matrixX = ref(5)
const matrixY = ref(5)
const rankingDraft = ref<string[]>([])
const hotspotX = ref(0.5)
const hotspotY = ref(0.5)
const surveyDraft = ref<Record<string, string | number>>({})
let countdown: number | undefined
const WORD_CLOUD_MAX_ENTRY_LENGTH = 40
const WORD_CLOUD_MAX_ENTRIES = 12

const isMultiplePoll = computed(() => props.interaction.kind === 'poll'
  && (props.interaction.config.multiple === 'true' || Boolean(props.interaction.config.max)))
const wordCloudAllowsMultiple = computed(() => props.interaction.kind === 'word-cloud'
  && props.interaction.config.entries !== 'one')

const selectedOptions = computed<string[]>(() => {
  if (isMultiplePoll.value) return multipleSelectionDraft.value
  const response = props.interaction.response
  if (!response) return []
  if (response.option_id) return [response.option_id]
  return response.option_ids || []
})
const rankedOptions = computed(() => {
  if (props.interaction.kind !== 'ranked-list') return []
  const revealed = props.interaction.phase === 'ranked'
    ? props.interaction.options.length
    : props.interaction.revealed_count || 0
  const available = props.interaction.options.slice(0, revealed)
  const ranking = props.interaction.phase === 'ranked' ? props.interaction.result.ranking : null
  if (!Array.isArray(ranking)) return available
  const byId = new Map(available.map(option => [option.id, option]))
  return ranking.map(id => byId.get(id)).filter(Boolean) as typeof available
})
const maxSelections = computed(() => Number(props.interaction.config.max || props.interaction.options.length))
const ratingValues = computed(() => {
  const min = Number(props.interaction.config.min || 1)
  const max = Number(props.interaction.config.max || 5)
  return Array.from({ length: Math.max(0, max - min + 1) }, (_, index) => min + index)
})
const quizCorrectOption = computed(() => props.interaction.result.correct_option_id as string | undefined)
const quizAnsweredCorrectly = computed(() => Boolean(
  props.interaction.response
  && quizCorrectOption.value
  && props.interaction.response.option_id === quizCorrectOption.value,
))
const timerLabel = computed(() => {
  const seconds = timerRemaining.value ?? Number(props.interaction.config.timer || 0)
  const minutes = Math.floor(seconds / 60)
  return `${minutes}:${String(Math.max(0, seconds % 60)).padStart(2, '0')}`
})
const timerIsRunning = computed(() => props.interaction.timer_running && Number(timerRemaining.value) > 0)
const allocationTarget = computed(() => Number(props.interaction.config.total || 100))
const allocationTotal = computed(() => Object.values(allocationDraft.value).reduce((sum, value) => sum + Number(value || 0), 0))
const rankingOptions = computed(() => {
  const byId = new Map(props.interaction.options.map(option => [option.id, option]))
  return rankingDraft.value.map(id => byId.get(id)).filter(Boolean) as typeof props.interaction.options
})
const surveyQuestions = computed<any[]>(() => props.interaction.config.questions || [])
const surveyComplete = computed(() => surveyQuestions.value.every((question) => {
  if (!question.required) return true
  const answer = surveyDraft.value[question.id]
  return answer !== undefined && answer !== null && String(answer).trim() !== ''
}))

const pausedNote = computed(() => {
  if (props.interaction.accepting_responses || props.interaction.phase === 'ranked') return ''
  return props.interaction.presenter_closed
    ? 'The presenter has closed this interaction. Your saved answer is kept.'
    : 'Audience input is temporarily paused.'
})

watch(() => props.interaction.response, (response) => {
  rankedVotes.value = { ...(response?.votes || {}) }
  const savedWordCloudEntries = Array.isArray(response?.texts)
    ? response.texts.filter((entry: unknown): entry is string => typeof entry === 'string' && Boolean(entry.trim()))
    : typeof response?.text === 'string' && response.text.trim()
      ? [response.text.trim()]
      : []
  wordCloudEntries.value = wordCloudAllowsMultiple.value
    ? savedWordCloudEntries.slice(0, WORD_CLOUD_MAX_ENTRIES)
    : savedWordCloudEntries.slice(-1)
  multipleSelectionDraft.value = response?.option_ids ? [...response.option_ids] : []
  if (response?.value != null) numberValue.value = String(response.value)
  allocationDraft.value = Object.fromEntries(props.interaction.options.map(option => [
    option.id,
    Number(response?.allocations?.[option.id] || 0),
  ]))
  matrixX.value = Number(response?.x ?? midpoint('x'))
  matrixY.value = Number(response?.y ?? midpoint('y'))
  rankingDraft.value = response?.ranking ? [...response.ranking] : props.interaction.options.map(option => option.id)
  hotspotX.value = Number(response?.x ?? 0.5)
  hotspotY.value = Number(response?.y ?? 0.5)
  surveyDraft.value = { ...(response?.answers || {}) }
}, { immediate: true, deep: true })
watch(
  () => [props.interaction.timer_remaining_seconds, props.interaction.timer_running] as const,
  ([remaining]) => {
    timerRemaining.value = remaining ?? Number(props.interaction.config.timer || 0)
  },
  { immediate: true },
)

onMounted(() => {
  window.addEventListener('online', updateConnectivity)
  window.addEventListener('offline', updateConnectivity)
  countdown = window.setInterval(() => {
    if (props.interaction.timer_running && timerRemaining.value && timerRemaining.value > 0) {
      timerRemaining.value -= 1
    }
  }, 1000)
})
onBeforeUnmount(() => {
  window.removeEventListener('online', updateConnectivity)
  window.removeEventListener('offline', updateConnectivity)
  window.clearInterval(countdown)
})

function updateConnectivity() {
  online.value = navigator.onLine
  if (!online.value) saveStatus.value = 'error'
}

async function respond(payload: Record<string, unknown>) {
  if (!props.interaction.accepting_responses) return false
  if (!navigator.onLine) {
    online.value = false
    saveStatus.value = 'error'
    error.value = 'You are offline. Reconnect, then try again.'
    return false
  }
  sending.value = true
  saveStatus.value = 'sending'
  error.value = ''
  try {
    const saved = await put<{ correct_option_id?: string }>(`/api/join/${props.joinCode}/interactions/${props.interaction.id}`, {
      payload,
      idempotency_key: crypto.randomUUID(),
    })
    saveStatus.value = 'saved'
    emit('saved', {
      interactionId: props.interaction.id,
      payload,
      correctOptionId: saved.correct_option_id,
    })
    return true
  }
  catch (reason) {
    saveStatus.value = 'error'
    error.value = reason instanceof Error ? reason.message : 'Could not save your response'
    return false
  }
  finally {
    sending.value = false
  }
}

async function submitText() {
  const entry = text.value.trim()
  if (!entry) return
  if (props.interaction.kind !== 'word-cloud') {
    if (await respond({ text: entry })) text.value = ''
    return
  }
  if (!wordCloudAllowsMultiple.value) {
    const next = [entry]
    if (await respond({ texts: next })) {
      wordCloudEntries.value = next
      text.value = ''
    }
    return
  }
  if (wordCloudEntries.value.length >= WORD_CLOUD_MAX_ENTRIES) {
    error.value = `You can add up to ${WORD_CLOUD_MAX_ENTRIES} word-cloud entries.`
    return
  }
  if (wordCloudEntries.value.some(existing => existing.toLocaleLowerCase() === entry.toLocaleLowerCase())) {
    error.value = 'You already added that word or phrase.'
    return
  }
  const next = [...wordCloudEntries.value, entry]
  if (await respond({ texts: next })) {
    wordCloudEntries.value = next
    text.value = ''
  }
}

async function removeWordCloudEntry(index: number) {
  const previous = [...wordCloudEntries.value]
  const next = previous.filter((_, entryIndex) => entryIndex !== index)
  wordCloudEntries.value = next
  if (!await respond({ texts: next })) wordCloudEntries.value = previous
}

async function chooseOption(optionId: string) {
  if (!isMultiplePoll.value) {
    respond({ option_id: optionId })
    return
  }
  const previous = [...multipleSelectionDraft.value]
  const selected = new Set(selectedOptions.value)
  if (selected.has(optionId)) {
    if (selected.size === 1) return
    selected.delete(optionId)
  }
  else {
    if (selected.size >= maxSelections.value) {
      error.value = `Choose no more than ${maxSelections.value} options`
      return
    }
    selected.add(optionId)
  }
  multipleSelectionDraft.value = [...selected]
  if (!await respond({ option_ids: multipleSelectionDraft.value })) {
    multipleSelectionDraft.value = previous
  }
}

function voteOnRankedOption(optionId: string, value: 1 | -1) {
  rankedVotes.value = { ...rankedVotes.value, [optionId]: value }
  respond({ votes: rankedVotes.value })
}

function midpoint(axis: 'x' | 'y') {
  const min = Number(props.interaction.config[`${axis}-min`] || 0)
  const max = Number(props.interaction.config[`${axis}-max`] || 10)
  return (min + max) / 2
}

function audienceImageUrl(url?: string) {
  if (!url) return ''
  const match = url.match(/^\/api\/decks\/[0-9a-f-]+\/assets\/([0-9a-f-]+)\/content$/i)
  return match ? `/api/join/${props.joinCode}/assets/${match[1]}` : url
}

function submitNumber() {
  if (numberValue.value === '') return
  respond({ value: Number(numberValue.value) })
}

function submitAllocation() {
  if (allocationTotal.value !== allocationTarget.value) return
  respond({ allocations: allocationDraft.value })
}

function submitMatrix() {
  respond({ x: matrixX.value, y: matrixY.value })
}

function moveRanking(index: number, direction: -1 | 1) {
  const target = index + direction
  if (target < 0 || target >= rankingDraft.value.length) return
  const next = [...rankingDraft.value]
  ;[next[index], next[target]] = [next[target], next[index]]
  rankingDraft.value = next
}

function submitRanking() {
  respond({ ranking: rankingDraft.value })
}

function chooseHotspot(event: MouseEvent) {
  const bounds = (event.currentTarget as HTMLElement).getBoundingClientRect()
  hotspotX.value = Math.min(1, Math.max(0, (event.clientX - bounds.left) / bounds.width))
  hotspotY.value = Math.min(1, Math.max(0, (event.clientY - bounds.top) / bounds.height))
}

function submitHotspot() {
  respond({ x: hotspotX.value, y: hotspotY.value })
}

function surveyRatingValues(question: { min?: number; max?: number }) {
  const min = Number(question.min ?? 1)
  const max = Number(question.max ?? 5)
  return Array.from({ length: max - min + 1 }, (_, index) => min + index)
}

function submitSurvey() {
  if (!surveyComplete.value) return
  respond({ answers: surveyDraft.value })
}

function normalizedPointStyle(point: { x: number; y: number }) {
  return { left: `${Number(point.x) * 100}%`, top: `${Number(point.y) * 100}%` }
}

function matrixPointStyle(point: { x: number; y: number }) {
  const xMin = Number(props.interaction.config['x-min'] || 0)
  const xMax = Number(props.interaction.config['x-max'] || 10)
  const yMin = Number(props.interaction.config['y-min'] || 0)
  const yMax = Number(props.interaction.config['y-max'] || 10)
  return {
    left: `${(Number(point.x) - xMin) / (xMax - xMin) * 100}%`,
    bottom: `${(Number(point.y) - yMin) / (yMax - yMin) * 100}%`,
  }
}
</script>

<template>
  <article :class="['audience-interaction-card', { 'element-card': interaction.element }]">
    <div class="interaction-kicker">
      <span>{{ interaction.element ? 'VOTE ON THIS IDEA' : interaction.kind.replace('-', ' ').toUpperCase() }}</span>
      <span class="response-save-state" aria-live="polite">
        <template v-if="saveStatus === 'sending'">Sending…</template>
        <template v-else-if="saveStatus === 'error' && !online">Offline · not saved</template>
        <template v-else-if="saveStatus === 'error'">Retry needed</template>
        <template v-else-if="saveStatus === 'saved' || interaction.response">✓ Saved</template>
      </span>
    </div>
    <h2>{{ interaction.title }}</h2>
    <div v-if="interaction.config.timer" :class="['audience-timer', { elapsed: timerRemaining === 0 }]" role="timer" aria-live="off">
      <span>{{ timerIsRunning ? 'Time remaining' : timerRemaining === 0 ? 'Time is up' : 'Countdown' }}</span>
      <strong>{{ timerLabel }}</strong>
    </div>

    <template v-if="interaction.kind === 'poll' || interaction.kind === 'quiz'">
      <p v-if="isMultiplePoll" class="interaction-instruction">Choose up to {{ maxSelections }} options.</p>
      <div class="audience-options">
        <button
          v-for="option in interaction.options"
          :key="option.id"
          :class="{
            selected: selectedOptions.includes(option.id),
            correct: interaction.kind === 'quiz' && Boolean(interaction.response) && quizCorrectOption === option.id,
            incorrect: interaction.kind === 'quiz' && selectedOptions.includes(option.id) && Boolean(quizCorrectOption) && quizCorrectOption !== option.id,
          }"
          :disabled="sending || !interaction.accepting_responses"
          :aria-pressed="selectedOptions.includes(option.id)"
          @click="chooseOption(option.id)"
        >
          <span>{{ option.label }}</span>
          <strong>{{ interaction.result.counts?.[option.id] || 0 }}</strong>
        </button>
      </div>
      <p v-if="interaction.kind === 'quiz' && interaction.response && quizCorrectOption" :class="['quiz-feedback', { correct: quizAnsweredCorrectly }]">
        {{ quizAnsweredCorrectly ? 'Correct' : 'Not quite — the correct answer is highlighted.' }}
      </p>
    </template>

    <div v-else-if="interaction.kind === 'image-choice'" class="image-choice-options">
      <button
        v-for="option in interaction.options"
        :key="option.id"
        :class="{ selected: selectedOptions.includes(option.id) }"
        :disabled="sending || !interaction.accepting_responses"
        :aria-pressed="selectedOptions.includes(option.id)"
        @click="respond({ option_id: option.id })"
      >
        <img :src="audienceImageUrl(option.image_url)" :alt="option.label">
        <span>{{ option.label }}</span>
        <strong>{{ interaction.result.counts?.[option.id] || 0 }}</strong>
      </button>
    </div>

    <div v-else-if="interaction.kind === 'reaction'" class="reaction-options">
      <button
        v-for="option in interaction.options"
        :key="option.id"
        :class="{ selected: selectedOptions.includes(option.id) }"
        :disabled="sending || !interaction.accepting_responses"
        :aria-pressed="selectedOptions.includes(option.id)"
        @click="respond({ option_id: option.id })"
      ><span>{{ option.label }}</span><strong>{{ interaction.result.counts?.[option.id] || 0 }}</strong></button>
    </div>

    <div v-else-if="interaction.kind === 'vote' || interaction.kind === 'updown'" class="vote-buttons">
      <button class="up" :class="{ selected: interaction.response?.value === 1 }" :disabled="sending || !interaction.accepting_responses" @click="respond({ value: 1 })">▲ <strong>{{ interaction.result.up || 0 }}</strong><span>Upvote</span></button>
      <button class="down" :class="{ selected: interaction.response?.value === -1 }" :disabled="sending || !interaction.accepting_responses" @click="respond({ value: -1 })">▼ <strong>{{ interaction.result.down || 0 }}</strong><span>Downvote</span></button>
    </div>

    <div v-else-if="interaction.kind === 'rating'" class="rating-buttons">
      <button
        v-for="rating in ratingValues"
        :key="rating"
        :class="{ selected: interaction.response?.value === rating }"
        :disabled="sending || !interaction.accepting_responses"
        @click="respond({ value: rating })"
      >{{ rating }}</button>
      <p v-if="interaction.result.average">Room average · {{ Number(interaction.result.average).toFixed(1) }}</p>
    </div>

    <form v-else-if="interaction.kind === 'number'" class="number-response" @submit.prevent="submitNumber">
      <div>
        <input v-model="numberValue" type="number" :min="interaction.config.min || 0" :max="interaction.config.max || 100" :step="interaction.config.step || 1" :disabled="sending || !interaction.accepting_responses" inputmode="decimal">
        <span v-if="interaction.config.unit">{{ interaction.config.unit }}</span>
        <button :disabled="sending || numberValue === '' || !interaction.accepting_responses">Submit</button>
      </div>
      <p v-if="interaction.result.average != null">Room estimate · average {{ Number(interaction.result.average).toFixed(1) }}, median {{ Number(interaction.result.median).toFixed(1) }}</p>
    </form>

    <form v-else-if="interaction.kind === 'allocation'" class="allocation-response" @submit.prevent="submitAllocation">
      <label v-for="option in interaction.options" :key="option.id">
        <span>{{ option.label }} <small v-if="interaction.result.averages?.[option.id] != null">room avg {{ Number(interaction.result.averages[option.id]).toFixed(1) }}</small></span>
        <input v-model.number="allocationDraft[option.id]" type="number" min="0" :max="allocationTarget" step="1" :disabled="sending || !interaction.accepting_responses" inputmode="numeric">
      </label>
      <div :class="['allocation-total', { valid: allocationTotal === allocationTarget }]">
        <span>{{ allocationTotal }} / {{ allocationTarget }} allocated</span>
        <button :disabled="sending || allocationTotal !== allocationTarget || !interaction.accepting_responses">Submit</button>
      </div>
    </form>

    <form v-else-if="interaction.kind === 'matrix'" class="matrix-response" @submit.prevent="submitMatrix">
      <label>
        <span>{{ interaction.config['x-label'] || 'Horizontal' }} · {{ matrixX }}</span>
        <input v-model.number="matrixX" type="range" :min="interaction.config['x-min'] || 0" :max="interaction.config['x-max'] || 10" :step="interaction.config.step || 1" :disabled="sending || !interaction.accepting_responses">
      </label>
      <label>
        <span>{{ interaction.config['y-label'] || 'Vertical' }} · {{ matrixY }}</span>
        <input v-model.number="matrixY" type="range" :min="interaction.config['y-min'] || 0" :max="interaction.config['y-max'] || 10" :step="interaction.config.step || 1" :disabled="sending || !interaction.accepting_responses">
      </label>
      <div v-if="interaction.result.points?.length" class="audience-matrix-plot" aria-label="Room responses on the matrix">
        <i v-for="(point, index) in interaction.result.points" :key="index" :style="matrixPointStyle(point)" />
      </div>
      <button :disabled="sending || !interaction.accepting_responses">Place my point</button>
      <p v-if="interaction.result.average_x != null">Room center · {{ Number(interaction.result.average_x).toFixed(1) }}, {{ Number(interaction.result.average_y).toFixed(1) }}</p>
    </form>

    <form v-else-if="interaction.kind === 'ranking'" class="ranking-response" @submit.prevent="submitRanking">
      <p class="interaction-instruction">Put every option in your preferred order.</p>
      <ol>
        <li v-for="(option, index) in rankingOptions" :key="option.id">
          <b>{{ index + 1 }}</b>
          <span>{{ option.label }}</span>
          <div>
            <button type="button" :disabled="index === 0 || sending || !interaction.accepting_responses" :aria-label="`Move ${option.label} up`" @click="moveRanking(index, -1)">↑</button>
            <button type="button" :disabled="index === rankingOptions.length - 1 || sending || !interaction.accepting_responses" :aria-label="`Move ${option.label} down`" @click="moveRanking(index, 1)">↓</button>
          </div>
        </li>
      </ol>
      <button :disabled="sending || !interaction.accepting_responses">Save my ranking</button>
      <p v-if="interaction.result.ranking?.length">Room order updates from {{ interaction.result.count }} complete rankings.</p>
    </form>

    <form v-else-if="interaction.kind === 'image-hotspot'" class="hotspot-response" @submit.prevent="submitHotspot">
      <button type="button" class="hotspot-image" :disabled="sending || !interaction.accepting_responses" :aria-label="`Place a pin on ${interaction.config.alt}`" @click="chooseHotspot">
        <img :src="audienceImageUrl(interaction.config.image)" :alt="interaction.config.alt">
        <i v-for="(point, index) in interaction.result.points || []" :key="`room-${index}`" class="room-point" :style="normalizedPointStyle(point)" />
        <i class="my-point" :style="normalizedPointStyle({ x: hotspotX, y: hotspotY })" />
      </button>
      <details>
        <summary>Fine-tune pin position</summary>
        <label><span>Horizontal · {{ Math.round(hotspotX * 100) }}%</span><input v-model.number="hotspotX" type="range" min="0" max="1" step="0.01"></label>
        <label><span>Vertical · {{ Math.round(hotspotY * 100) }}%</span><input v-model.number="hotspotY" type="range" min="0" max="1" step="0.01"></label>
      </details>
      <button :disabled="sending || !interaction.accepting_responses">Place my pin</button>
    </form>

    <form v-else-if="interaction.kind === 'survey'" class="survey-response" @submit.prevent="submitSurvey">
      <fieldset v-for="(surveyQuestion, index) in surveyQuestions" :key="surveyQuestion.id">
        <legend><b>{{ index + 1 }}</b>{{ surveyQuestion.label }} <small v-if="!surveyQuestion.required">optional</small></legend>
        <div v-if="surveyQuestion.type === 'rating'" class="survey-rating">
          <button v-for="value in surveyRatingValues(surveyQuestion)" :key="value" type="button" :class="{ selected: surveyDraft[surveyQuestion.id] === value }" :disabled="sending || !interaction.accepting_responses" :aria-pressed="surveyDraft[surveyQuestion.id] === value" @click="surveyDraft[surveyQuestion.id] = value">{{ value }}</button>
          <span v-if="interaction.result.questions?.[surveyQuestion.id]?.average != null">room avg {{ Number(interaction.result.questions[surveyQuestion.id].average).toFixed(1) }}</span>
        </div>
        <div v-else-if="surveyQuestion.type === 'choice'" class="survey-choices">
          <button v-for="choice in surveyQuestion.options" :key="choice.id" type="button" :class="{ selected: surveyDraft[surveyQuestion.id] === choice.id }" :disabled="sending || !interaction.accepting_responses" :aria-pressed="surveyDraft[surveyQuestion.id] === choice.id" @click="surveyDraft[surveyQuestion.id] = choice.id">
            <span>{{ choice.label }}</span><strong>{{ interaction.result.questions?.[surveyQuestion.id]?.counts?.[choice.id] || 0 }}</strong>
          </button>
        </div>
        <textarea v-else v-model="surveyDraft[surveyQuestion.id]" :maxlength="surveyQuestion.max || 500" :disabled="sending || !interaction.accepting_responses" placeholder="Your response" />
      </fieldset>
      <button :disabled="sending || !surveyComplete || !interaction.accepting_responses">Submit survey</button>
      <p v-if="!interaction.result.hidden">{{ interaction.result.count || 0 }} complete responses</p>
    </form>

    <div v-else-if="interaction.kind === 'ranked-list'" class="ranked-audience-list">
      <p v-if="interaction.phase !== 'ranked'" class="ranked-phase-note live">Vote as each idea appears. You can change your votes until the presenter closes voting.</p>
      <p v-else class="ranked-phase-note final">Voting is closed. Here is the room’s final ranking.</p>
      <TransitionGroup name="audience-rank" tag="ol">
        <li v-for="(option, index) in rankedOptions" :key="option.id">
          <span v-if="interaction.phase === 'ranked'" class="audience-rank-number">{{ index + 1 }}</span>
          <span v-else class="audience-rank-number neutral">•</span>
          <strong>{{ option.label }}</strong>
          <div v-if="interaction.phase === 'voting'" class="ranked-vote-buttons">
            <button :class="{ selected: rankedVotes[option.id] === 1 }" :disabled="sending || !interaction.accepting_responses" :aria-label="`Upvote ${option.label}`" @click="voteOnRankedOption(option.id, 1)">▲</button>
            <button :class="{ selected: rankedVotes[option.id] === -1 }" :disabled="sending || !interaction.accepting_responses" :aria-label="`Downvote ${option.label}`" @click="voteOnRankedOption(option.id, -1)">▼</button>
          </div>
          <span v-else-if="interaction.phase === 'ranked'" class="audience-vote-totals">
            <b class="up">▲ {{ interaction.result.scores?.[option.id]?.up || 0 }}</b>
            <b class="down">▼ {{ interaction.result.scores?.[option.id]?.down || 0 }}</b>
          </span>
        </li>
      </TransitionGroup>
    </div>

    <form v-else-if="interaction.kind === 'word-cloud' || interaction.kind === 'free-text'" class="text-response" @submit.prevent="submitText">
      <div><input v-model="text" :maxlength="interaction.kind === 'word-cloud' ? WORD_CLOUD_MAX_ENTRY_LENGTH : 1000" :placeholder="interaction.kind === 'word-cloud' ? (wordCloudAllowsMultiple ? 'Add a word or short phrase' : 'Your word or short phrase') : 'Share your response'" :disabled="sending || !interaction.accepting_responses"><button :disabled="sending || !text.trim() || !interaction.accepting_responses">{{ interaction.kind === 'word-cloud' ? (wordCloudAllowsMultiple ? 'Add' : wordCloudEntries.length ? 'Update' : 'Submit') : 'Send' }}</button></div>
      <small v-if="interaction.kind === 'word-cloud'" class="word-cloud-policy">{{ wordCloudAllowsMultiple ? `Up to ${WORD_CLOUD_MAX_ENTRIES} responses, ${WORD_CLOUD_MAX_ENTRY_LENGTH} characters each.` : `One response, up to ${WORD_CLOUD_MAX_ENTRY_LENGTH} characters. Submitting again replaces it.` }}</small>
      <div v-if="interaction.kind === 'word-cloud' && wordCloudEntries.length" class="word-cloud-submissions">
        <small>{{ wordCloudAllowsMultiple ? 'Your responses' : 'Your response' }}</small>
        <div>
          <span v-for="(entry, index) in wordCloudEntries" :key="`${entry}-${index}`">
            {{ entry }}
            <button type="button" :aria-label="`Remove ${entry}`" :disabled="sending || !interaction.accepting_responses" @click="removeWordCloudEntry(index)">×</button>
          </span>
        </div>
      </div>
      <PackedWordCloud v-if="interaction.kind === 'word-cloud' && interaction.result.words?.length" :words="interaction.result.words" class="audience-word-cloud" />
    </form>

    <p v-if="pausedNote" class="input-frozen-note">{{ pausedNote }}</p>
    <p v-if="error" class="field-error">{{ error }}</p>
    <p v-if="interaction.kind === 'ranked-list' && interaction.phase !== 'ranked'" class="response-total">{{ interaction.result.count || 0 }} {{ interaction.result.count === 1 ? 'voter' : 'voters' }} · scores reveal when voting closes</p>
    <p v-else-if="interaction.kind === 'ranked-list'" class="response-total">{{ interaction.result.count || 0 }} {{ interaction.result.count === 1 ? 'voter' : 'voters' }}</p>
    <p v-else-if="interaction.result.hidden" class="response-total">{{ interaction.config.results === 'presenter' || interaction.config.results === 'hidden' ? 'Aggregate results are presenter-only' : interaction.config.results === 'manual' ? 'Results appear when the presenter reveals them' : interaction.config.results === 'on-close' ? 'Results appear when voting closes' : 'Results appear after you respond' }}</p>
    <p v-else-if="interaction.kind === 'word-cloud'" class="response-total">{{ interaction.result.submission_count || 0 }} {{ interaction.result.submission_count === 1 ? 'word' : 'words' }} from {{ interaction.result.count || 0 }} {{ interaction.result.count === 1 ? 'person' : 'people' }}</p>
    <p v-else class="response-total">{{ interaction.result.count || 0 }} {{ interaction.result.count === 1 ? 'response' : 'responses' }}</p>
  </article>
</template>
