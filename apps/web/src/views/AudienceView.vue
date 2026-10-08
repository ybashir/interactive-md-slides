<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'

import AudienceInteractionCard from '../components/AudienceInteractionCard.vue'
import { ApiError, api, post } from '../api'
import type { AudienceInteraction, LiveState } from '../types'
import { audienceCanSeeResults, mergeAudienceSnapshot } from '../audience-snapshot.mjs'

const route = useRoute()
const joinCode = String(route.params.joinCode).toUpperCase()
const state = ref<LiveState | null>(null)
const displayName = ref('')
const joined = ref(localStorage.getItem(`interdeck:joined:${joinCode}`) === 'true')
const joining = ref(false)
const activeTab = ref<'interactions' | 'questions'>('interactions')
const question = ref('')
const questionScope = ref<'general' | 'slide'>('general')
const error = ref('')
const connection = ref<'connecting' | 'connected' | 'offline'>('connecting')
let events: EventSource | undefined
let pendingQuestionRefreshAt = 0
let privateRefreshTimer: ReturnType<typeof setTimeout> | undefined
let refreshRequest = 0
let sharedGeneration = 0
let disposed = false
const ownResponses = new Map<string, Record<string, any>>()
const aiModeration = ref(false)

const pendingQuestionCount = computed(() => state.value?.questions.filter(item => item.mine && item.moderation_status === 'pending').length || 0)

onMounted(async () => {
  try { aiModeration.value = (await api<{ ai_moderation_enabled: boolean }>('/api/config')).ai_moderation_enabled } catch {}
  await refresh()
  if (joined.value && !state.value?.audience_session_active) {
    joined.value = false
    localStorage.removeItem(`interdeck:joined:${joinCode}`)
  }
  if (joined.value) connect()
})
onBeforeUnmount(() => { disposed = true; refreshRequest++; events?.close(); clearTimeout(privateRefreshTimer) })

async function joinRoom(anonymous = false) {
  joining.value = true
  error.value = ''
  try {
    refreshRequest++
    ownResponses.clear()
    state.value = await post<LiveState>(`/api/join/${joinCode}`, {
      display_name: anonymous ? null : displayName.value.trim() || null,
    })
    joined.value = true
    localStorage.setItem(`interdeck:joined:${joinCode}`, 'true')
    connect()
  }
  catch (reason) {
    error.value = reason instanceof Error ? reason.message : 'Could not join this room'
  }
  finally {
    joining.value = false
  }
}

async function refresh() {
  const request = ++refreshRequest
  const generation = sharedGeneration
  try {
    const next = await api<LiveState>(`/api/join/${joinCode}/state`)
    if (disposed || request !== refreshRequest) return
    // REST sequence numbers describe durable navigation; SSE also counts
    // transient response updates. They cannot be compared for freshness.
    if (generation !== sharedGeneration) { schedulePrivateRefresh(); return }
    if (state.value && next.result_epoch !== state.value.result_epoch) ownResponses.clear()
    rememberOwnResponses(next.interactions)
    state.value = next
  }
  catch (reason) {
    if (reason instanceof ApiError && reason.status === 404) error.value = 'This audience code does not exist.'
    else error.value = reason instanceof Error ? reason.message : 'Could not load this room'
  }
}

function connect() {
  events?.close()
  connection.value = 'connecting'
  events = new EventSource(`/api/join/${joinCode}/events`)
  events.onopen = () => {
    connection.value = 'connected'
  }
  events.onerror = () => { connection.value = 'offline' }
  events.addEventListener('state', applyStateEvent)
}

function applyStateEvent(event: MessageEvent<string>) {
  try {
    const envelope = JSON.parse(event.data)
    if (envelope?.type === 'snapshot' && envelope.state) {
      const sequence = Number(envelope.sequence || 0)
      if (sequence && sequence < Number(state.value?.sequence || 0)) return
      applySharedSnapshot(envelope.state as LiveState, sequence)
      refreshOwnPendingQuestions()
      return
    }
  }
  catch {}
  // Compatibility and recovery path for an older server during a rolling
  // deploy, a lagged broadcast receiver, or a malformed event.
  void refresh()
}

function applySharedSnapshot(shared: LiveState, sequence: number) {
  sharedGeneration++
  if (state.value) rememberOwnResponses(state.value.interactions)
  state.value = mergeAudienceSnapshot(state.value, shared, sequence, ownResponses)
  const interactions = state.value.interactions
  // Shared events contain only results visible before anyone votes. Refresh
  // authorized after-vote results through the cookie-authenticated REST route.
  if (interactions.some(item => item.response && item.result && shared.interactions.find(candidate => candidate.id === item.id)?.result?.hidden
    && audienceCanSeeResults(item, true))) schedulePrivateRefresh()
}

function schedulePrivateRefresh() {
  if (privateRefreshTimer) return
  privateRefreshTimer = setTimeout(() => { privateRefreshTimer = undefined; void refresh() }, 2000)
}

function rememberOwnResponses(interactions: AudienceInteraction[]) {
  for (const interaction of interactions) {
    if (interaction.response) ownResponses.set(interaction.id, interaction.response)
  }
}

function applySavedResponse(details: { interactionId: string; payload: Record<string, unknown>; correctOptionId?: string }) {
  ownResponses.set(details.interactionId, details.payload)
  const interaction = state.value?.interactions.find(item => item.id === details.interactionId)
  if (!interaction) return
  interaction.response = details.payload
  if (details.correctOptionId) {
    interaction.result = { ...(interaction.result || {}), correct_option_id: details.correctOptionId }
  }
  void refresh()
}

function refreshOwnPendingQuestions() {
  if (!state.value?.questions.some(question => question.mine && question.moderation_status === 'pending')) return
  const now = Date.now()
  if (now < pendingQuestionRefreshAt) return
  pendingQuestionRefreshAt = now + 5_000
  void refresh()
}

async function askQuestion() {
  if (!question.value.trim() || state.value?.input_frozen) return
  try {
    await post(`/api/join/${joinCode}/questions`, {
      body: question.value.trim(),
      scope: questionScope.value,
    })
    question.value = ''
    await refresh()
  }
  catch (reason) {
    error.value = reason instanceof Error ? reason.message : 'Could not submit your question'
  }
}

async function voteQuestion(questionId: string) {
  if (state.value?.input_frozen) return
  await post(`/api/join/${joinCode}/questions/${questionId}/vote`)
  await refresh()
}
</script>

<template>
  <main class="audience-page">
    <header class="audience-header">
      <span class="audience-wordmark">interdeck</span>
      <span :class="['connection-pill', connection]"><i /> {{ connection === 'connected' ? 'Live' : connection }}</span>
    </header>

    <section v-if="!joined" class="join-card">
      <p class="eyebrow">ROOM · {{ joinCode }}</p>
      <h1>{{ state?.deck_title || 'Join the conversation' }}</h1>
      <p>Add a name so the presenter can recognize your questions.</p>
      <p class="microcopy">Your responses are stored and may be shared by the presenter. <span v-if="aiModeration">Questions may be processed by Google Gemini.</span> <a href="/privacy">Privacy details</a></p>
      <form @submit.prevent="joinRoom(false)">
        <label for="display-name">Your name <span>optional</span></label>
        <input id="display-name" v-model="displayName" maxlength="80" placeholder="How should we address you?" autocomplete="name">
        <button class="button button-primary button-large" :disabled="joining">{{ joining ? 'Joining…' : 'Join the room' }}</button>
      </form>
      <button class="anonymous-link" :disabled="joining" @click="joinRoom(true)">Continue anonymously</button>
      <p v-if="error" class="field-error">{{ error }}</p>
    </section>

    <template v-else>
      <section class="audience-room-header">
        <div><p class="eyebrow">{{ state?.run_mode === 'rehearsal' ? 'REHEARSAL' : state?.live ? `SLIDE ${state.slide_number}` : 'THE ROOM' }}</p><h1>{{ state?.deck_title }}</h1></div>
        <span>{{ state?.participant_count || 0 }} here</span>
      </section>

      <nav class="audience-tabs" aria-label="Audience sections"><button :aria-current="activeTab === 'interactions' ? 'page' : undefined" :class="{ active: activeTab === 'interactions' }" @click="activeTab = 'interactions'">Now <span>{{ state?.interactions.length || 0 }}</span></button><button :aria-current="activeTab === 'questions' ? 'page' : undefined" :class="{ active: activeTab === 'questions' }" @click="activeTab = 'questions'">Q&A <span>{{ state?.questions.length || 0 }}</span></button></nav>

      <section v-if="activeTab === 'interactions'" class="audience-content">
        <div v-if="!state?.live" class="audience-lobby"><i /><h2>The presentation has not started yet.</h2><p>Stay here. This page will update automatically.</p></div>
        <template v-else-if="state.interactions.length">
          <AudienceInteractionCard v-for="interaction in state.interactions" :key="interaction.id" :interaction="interaction" :join-code="joinCode" @saved="applySavedResponse" />
        </template>
        <div v-else class="audience-lobby"><i class="pulse" /><h2>Nothing to answer on this slide.</h2><p>General Q&A remains open.</p><button class="text-button" @click="activeTab = 'questions'">Ask a question →</button></div>
      </section>

      <section v-else class="audience-content qna-content">
        <div v-if="!state?.live" class="audience-lobby"><i /><h2>The presentation has not started yet.</h2><p>Q&A and voting will become available when the presenter starts.</p></div>
        <template v-else>
          <p v-if="state?.input_frozen" class="input-frozen-note room">The presenter has temporarily paused audience input.</p>
          <form class="question-form" @submit.prevent="askQuestion">
            <label for="question">Ask the presenter</label>
            <div class="question-scope" aria-label="Question visibility">
              <button type="button" :class="{ active: questionScope === 'general' }" @click="questionScope = 'general'">General Q&A</button>
              <button type="button" :class="{ active: questionScope === 'slide' }" @click="questionScope = 'slide'">This slide<span v-if="state?.slide_number"> · {{ state.slide_number }}</span></button>
            </div>
            <small class="scope-help">{{ questionScope === 'general' ? 'Visible throughout the presentation.' : 'Visible to the audience only while this slide is active.' }}</small>
            <textarea id="question" v-model="question" maxlength="500" placeholder="What would you like to know?" :disabled="state?.input_frozen" />
            <div><small>{{ question.length }} / 500</small><button class="button button-primary" :disabled="question.trim().length < 3 || state?.input_frozen">Submit</button></div>
          </form>
          <p v-if="pendingQuestionCount" class="pending-note">Your question is awaiting moderation. Only you and the presenter can see it.</p>
          <div class="audience-questions">
            <article v-for="item in state?.questions" :key="item.id" :class="{ pending: item.moderation_status === 'pending' }">
              <button :disabled="item.moderation_status !== 'approved' || state?.input_frozen" @click="voteQuestion(item.id)">▲ <strong>{{ item.votes }}</strong></button>
              <div><p>{{ item.body }}</p><span>{{ item.display_name || 'Anonymous' }} · {{ item.slide_number ? `Slide ${item.slide_number}` : 'General' }} · {{ item.moderation_status === 'pending' ? 'Awaiting moderation' : item.lifecycle_status }}</span></div>
            </article>
            <p v-if="!state?.questions.length" class="empty-copy">No questions yet. Yours can be the first.</p>
          </div>
        </template>
      </section>
    </template>
  </main>
</template>
