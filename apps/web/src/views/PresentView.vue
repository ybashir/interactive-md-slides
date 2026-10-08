<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import QrcodeVue from 'qrcode.vue'

import { api, patch, post } from '../api'
import type { DeckDetail, LiveState, Question } from '../types'

const route = useRoute()
const router = useRouter()
const deckId = route.params.id as string
const requestedMode = route.query.mode === 'rehearsal' ? 'rehearsal' : 'live'
const consoleOnly = computed(() => route.query.view === 'console')
const deck = ref<DeckDetail | null>(null)
const state = ref<LiveState | null>(null)
const accessUrl = ref('')
const presentationFrame = ref<HTMLIFrameElement | null>(null)
const slideNumber = ref(1)
const clickStep = ref(0)
const loading = ref(true)
const error = ref('')
const actionError = ref('')
const sidePanel = ref<'room' | 'questions' | 'insights'>('room')
const questionFilter = ref<'active' | 'pending' | 'pinned' | 'rejected' | 'archived'>('active')
const questionFilters = ['active', 'pending', 'pinned', 'rejected', 'archived'] as const
let events: EventSource | undefined
let bridgeReady = false
let bridgeSynchronized = false
let initialSyncCommand = ''
let navigationInFlight = false
let pendingNavigation: { slideNumber: number; clickStep: number; visibleElementInteractionIds: string[] } | undefined
let countdown: number | undefined
let stateRefreshInFlight = false
let stateRefreshQueued = false
let windowChannel: BroadcastChannel | undefined

const joinUrl = computed(() => deck.value ? `${window.location.origin}/j/${deck.value.join_code}` : '')
const iframeUrl = computed(() => {
  if (!accessUrl.value) return ''
  const url = new URL(accessUrl.value, window.location.origin)
  url.searchParams.set('embedded', 'true')
  return `${url.pathname}${url.search}`
})
const interactionActionPending = ref(false)
const inputControlPending = ref(false)
const qaSettingPending = ref(false)
const insightRequestPending = ref(false)
const analyzedApprovedQuestionCount = computed(() => (state.value?.questions || []).filter(question =>
  question.analysis_status === 'complete'
  && question.moderation_status === 'approved'
  && question.lifecycle_status !== 'archived',
).length)
const insightsPending = computed(() => insightRequestPending.value
  || state.value?.ai_insights_status === 'queued'
  || state.value?.ai_insights_status === 'running')
const ungroupedQuestionCount = computed(() => Math.max(
  0,
  analyzedApprovedQuestionCount.value - (state.value?.ai_insights?.question_count || 0),
))
const visibleQuestions = computed(() => (state.value?.questions || []).filter((question) => {
  if (questionFilter.value === 'pending') return question.moderation_status === 'pending'
  if (questionFilter.value === 'pinned') return question.is_pinned && question.lifecycle_status !== 'archived'
  if (questionFilter.value === 'rejected') return question.moderation_status === 'rejected' && question.lifecycle_status !== 'archived'
  if (questionFilter.value === 'archived') return question.lifecycle_status === 'archived'
  return question.lifecycle_status !== 'archived' && question.moderation_status !== 'rejected'
}))

onMounted(async () => {
  window.addEventListener('keydown', onKeydown)
  if (!consoleOnly.value) window.addEventListener('message', onSlidevMessage)
  windowChannel = new BroadcastChannel(`interdeck-presenter-${deckId}`)
  windowChannel.addEventListener('message', onPresenterWindowMessage)
  countdown = window.setInterval(tickTimers, 1000)
  try {
    deck.value = await api<DeckDetail>(`/api/decks/${deckId}`)
    document.title = consoleOnly.value
      ? `Presenter controls · ${deck.value.title}`
      : deck.value.title
    if (!consoleOnly.value) {
      const endpoint = requestedMode === 'rehearsal' ? 'rehearse' : 'present'
      const started = await post<{ url: string }>(`/api/decks/${deckId}/${endpoint}`)
      accessUrl.value = started.url
    }
    await refreshState()
    if (!consoleOnly.value) windowChannel?.postMessage({ type: 'interdeck:presentation-started' })
    events = new EventSource(`/api/join/${deck.value.join_code}/events`)
    events.onopen = () => { void refreshState() }
    events.addEventListener('state', refreshState)
  }
  catch (reason) {
    error.value = reason instanceof Error ? reason.message : 'Could not start the presentation'
  }
  finally {
    loading.value = false
  }
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown)
  window.removeEventListener('message', onSlidevMessage)
  windowChannel?.removeEventListener('message', onPresenterWindowMessage)
  windowChannel?.close()
  events?.close()
  window.clearInterval(countdown)
})

function tickTimers() {
  for (const interaction of state.value?.interactions || []) {
    if (!interaction.timer_running || !interaction.timer_remaining_seconds) continue
    interaction.timer_remaining_seconds -= 1
    if (interaction.timer_remaining_seconds <= 0) interaction.timer_running = false
  }
}

function timerSeconds(interaction: NonNullable<LiveState['interactions'][number]>) {
  return interaction.timer_remaining_seconds ?? Number(interaction.config.timer || 0)
}

function formatTimer(seconds: number) {
  return `${Math.floor(seconds / 60)}:${String(Math.max(0, seconds % 60)).padStart(2, '0')}`
}

async function refreshState() {
  if (stateRefreshInFlight) {
    stateRefreshQueued = true
    return
  }
  stateRefreshInFlight = true
  try {
    const next = await api<LiveState>(`/api/decks/${deckId}/live-state`)
    state.value = next
    if (!next.live) {
      pendingNavigation = undefined
      bridgeSynchronized = false
    }
    if (!bridgeSynchronized && next.slide_number) {
      slideNumber.value = next.slide_number
      clickStep.value = next.click_step
    }
  }
  finally {
    stateRefreshInFlight = false
    if (stateRefreshQueued) {
      stateRefreshQueued = false
      void refreshState()
    }
  }
}

function sendSlidevCommand(action: 'next' | 'prev' | 'go', details: Record<string, unknown> = {}) {
  if (!bridgeReady || !presentationFrame.value?.contentWindow) return
  presentationFrame.value.contentWindow.postMessage({
    type: 'interdeck:command',
    action,
    ...details,
  }, window.location.origin)
}

function go(direction: 1 | -1) {
  const action = direction > 0 ? 'next' : 'prev'
  if (consoleOnly.value) windowChannel?.postMessage({ type: 'interdeck:stage-command', action })
  else sendSlidevCommand(action)
}

function openPresenterConsole() {
  const target = router.resolve({
    path: `/decks/${deckId}/present`,
    query: { ...(requestedMode === 'rehearsal' ? { mode: 'rehearsal' } : {}), view: 'console' },
  })
  const presenterConsole = window.open(
    target.href,
    `interdeck-presenter-${deckId}`,
    'popup,width=480,height=900,resizable=yes,scrollbars=yes',
  )
  presenterConsole?.focus()
}

function onPresenterWindowMessage(event: MessageEvent) {
  const message = event.data
  if (!message || typeof message !== 'object') return
  if (!consoleOnly.value && message.type === 'interdeck:stage-command') {
    if (message.action === 'next' || message.action === 'prev') sendSlidevCommand(message.action)
    return
  }
  if (!consoleOnly.value && message.type === 'interdeck:presentation-ended') {
    void router.push(`/decks/${deckId}`)
    return
  }
  if (consoleOnly.value && message.type === 'interdeck:presentation-started') {
    void refreshState()
  }
}

function onSlidevMessage(event: MessageEvent) {
  if (event.origin !== window.location.origin || event.source !== presentationFrame.value?.contentWindow) return
  const message = event.data
  if (!message || typeof message !== 'object') return
  if (message.type === 'interdeck:ready') {
    bridgeReady = true
    bridgeSynchronized = false
    initialSyncCommand = crypto.randomUUID()
    sendSlidevCommand('go', {
      slideNumber: slideNumber.value,
      clickStep: clickStep.value,
      commandId: initialSyncCommand,
    })
    return
  }
  if (message.type === 'interdeck:open-presenter-console') {
    openPresenterConsole()
    return
  }
  if (message.type !== 'interdeck:navigation') return
  if (!bridgeSynchronized) {
    if (message.commandId !== initialSyncCommand) return
    bridgeSynchronized = true
  }
  const nextSlide = Number(message.slideNumber)
  const nextClick = Number(message.clickStep)
  const visibleElementInteractionIds = Array.isArray(message.visibleElementInteractionIds)
    ? message.visibleElementInteractionIds.filter((id: unknown): id is string => typeof id === 'string')
    : []
  if (!Number.isInteger(nextSlide) || !Number.isInteger(nextClick)) return
  if (nextSlide === slideNumber.value && nextClick === clickStep.value) {
    queueNavigationPersistence(nextSlide, nextClick, visibleElementInteractionIds)
    return
  }
  slideNumber.value = nextSlide
  clickStep.value = nextClick
  queueNavigationPersistence(nextSlide, nextClick, visibleElementInteractionIds)
}

async function queueNavigationPersistence(nextSlide: number, nextClick: number, visibleElementInteractionIds: string[]) {
  if (!state.value?.live) return
  pendingNavigation = { slideNumber: nextSlide, clickStep: nextClick, visibleElementInteractionIds }
  if (navigationInFlight) return
  navigationInFlight = true
  try {
    while (pendingNavigation) {
      const navigation = pendingNavigation
      pendingNavigation = undefined
      await post(`/api/decks/${deckId}/navigate`, {
        slide_number: navigation.slideNumber,
        click_step: navigation.clickStep,
        visible_element_interaction_ids: navigation.visibleElementInteractionIds,
      })
    }
  }
  catch (reason) {
    actionError.value = reason instanceof Error ? reason.message : 'Could not synchronize the audience view'
  }
  finally {
    navigationInFlight = false
  }
}

function onKeydown(event: KeyboardEvent) {
  const target = event.target instanceof HTMLElement ? event.target : null
  if (target?.closest('input, textarea, select, button, a, [contenteditable="true"]')) return
  if (!consoleOnly.value && event.key.toLowerCase() === 'c') {
    event.preventDefault()
    openPresenterConsole()
    return
  }
  if (event.key === 'ArrowRight' || event.key === 'PageDown' || event.key === ' ') {
    event.preventDefault()
    go(1)
  }
  if (event.key === 'ArrowLeft' || event.key === 'PageUp') {
    event.preventDefault()
    go(-1)
  }
}

async function moderate(question: Question, moderation_status: 'pending' | 'approved' | 'rejected') {
  await patch(`/api/decks/${deckId}/questions/${question.id}`, { moderation_status })
  await refreshState()
}

async function updateQaDisplayMode(displayMode: 'verbatim' | 'ai_grouped') {
  if (!state.value || qaSettingPending.value || state.value.qa_display_mode === displayMode) return
  qaSettingPending.value = true
  actionError.value = ''
  try {
    await patch(`/api/decks/${deckId}/qa-settings`, { display_mode: displayMode })
    await refreshState()
  }
  catch (reason) {
    actionError.value = reason instanceof Error ? reason.message : 'Could not update the Q&A display mode'
  }
  finally {
    qaSettingPending.value = false
  }
}

async function generateQaInsights() {
  if (!state.value?.ai_configured || insightsPending.value) return
  insightRequestPending.value = true
  actionError.value = ''
  try {
    await post(`/api/decks/${deckId}/qa-insights`)
    await refreshState()
  }
  catch (reason) {
    actionError.value = reason instanceof Error ? reason.message : 'Could not generate the Q&A brief'
  }
  finally {
    insightRequestPending.value = false
  }
}

async function markAnswered(question: Question) {
  await patch(`/api/decks/${deckId}/questions/${question.id}`, {
    lifecycle_status: question.lifecycle_status === 'answered' ? 'open' : 'answered',
  })
  await refreshState()
}

async function toggleQuestionPin(question: Question) {
  await patch(`/api/decks/${deckId}/questions/${question.id}`, { is_pinned: !question.is_pinned })
  await refreshState()
}

async function archiveQuestion(question: Question) {
  await patch(`/api/decks/${deckId}/questions/${question.id}`, { lifecycle_status: 'archived', is_pinned: false })
  await refreshState()
}

function moderationFlags(question: Question) {
  if (!question.moderation_labels) return []
  const labels: string[] = []
  if (question.moderation_labels.safety_flagged) labels.push('Safety')
  if (question.moderation_labels.spam) labels.push('Spam')
  if (question.moderation_labels.contains_pii) labels.push('Personal info')
  if (question.moderation_labels.off_topic) labels.push('Off-topic')
  return labels
}

function duplicateQuestion(question: Question) {
  return state.value?.questions.find(item => item.id === question.duplicate_of)
}

async function resetResults() {
  const scope = state.value?.run_mode === 'rehearsal' ? 'rehearsal' : 'live'
  if (!window.confirm(`Start a new isolated ${scope} result epoch? Existing responses will remain archived.`)) return
  await post(`/api/decks/${deckId}/reset`)
  clickStep.value = 0
  sendSlidevCommand('go', { slideNumber: slideNumber.value, clickStep: 0 })
  await refreshState()
}

async function controlInput() {
  if (!state.value || inputControlPending.value) return
  inputControlPending.value = true
  actionError.value = ''
  try {
    await post(`/api/decks/${deckId}/input-control`, {
      action: state.value.input_frozen ? 'resume' : 'freeze',
    })
    await refreshState()
  }
  catch (reason) {
    actionError.value = reason instanceof Error ? reason.message : 'Could not change audience input state'
  }
  finally {
    inputControlPending.value = false
  }
}

async function controlInteraction(
  interactionId: string,
  action: 'open' | 'close' | 'reset' | 'reveal' | 'hide' | 'timer-start' | 'timer-pause' | 'timer-reset',
) {
  if (interactionActionPending.value) return
  interactionActionPending.value = true
  actionError.value = ''
  try {
    await post(`/api/decks/${deckId}/interactions/${interactionId}/control`, { action })
    if (action === 'reset') {
      clickStep.value = 0
      sendSlidevCommand('go', { slideNumber: slideNumber.value, clickStep: 0 })
    }
    await refreshState()
  }
  catch (reason) {
    actionError.value = reason instanceof Error ? reason.message : 'Could not update the interaction'
  }
  finally {
    interactionActionPending.value = false
  }
}

async function endPresentation() {
  if (!window.confirm('End this live presentation?')) return
  await post(`/api/decks/${deckId}/stop`)
  windowChannel?.postMessage({ type: 'interdeck:presentation-ended' })
  if (consoleOnly.value) {
    window.close()
    if (!window.closed) await router.push(`/decks/${deckId}`)
  }
  else {
    await router.push(`/decks/${deckId}`)
  }
}

</script>

<template>
  <main :class="['present-page', consoleOnly ? 'console-only' : 'stage-only']">
    <div v-if="loading" class="fullpage-loading"><i /> Preparing the live room…</div>
    <div v-else-if="error" class="fullpage-error"><h1>Presentation could not start</h1><p>{{ error }}</p><button class="button" @click="router.push(`/decks/${deckId}`)">Back to editor</button></div>
    <template v-else>
      <section v-if="!consoleOnly" class="present-stage">
        <iframe v-if="iframeUrl" ref="presentationFrame" :src="iframeUrl" title="Presentation display" allow="fullscreen; display-capture" />
        <div v-if="state && !state.live" class="stage-ended-notice">
          <strong>This presentation has ended</strong>
          <span>The display stopped because this run was ended from another window.</span>
          <button type="button" @click="router.push(`/decks/${deckId}`)">Back to editor</button>
        </div>
        <div class="stage-recovery-controls" aria-label="Presentation recovery controls">
          <button type="button" title="Open presenter controls (C)" @click="openPresenterConsole">Presenter controls</button>
          <button type="button" class="end" :disabled="!state?.live" @click="endPresentation">End presentation</button>
        </div>
      </section>

      <aside v-else class="present-console">
        <header>
          <RouterLink to="/" class="wordmark"><span class="wordmark-icon">I</span> interdeck</RouterLink>
          <span :class="['live-pill', { rehearsal: state?.run_mode === 'rehearsal', waiting: !state?.live }]" ><i /> {{ !state?.live ? 'ENDED' : state?.run_mode === 'rehearsal' ? 'REHEARSAL' : 'LIVE' }}</span>
        </header>
        <div class="present-tabs">
          <button :class="{ active: sidePanel === 'room' }" @click="sidePanel = 'room'">Room</button>
          <button :class="{ active: sidePanel === 'questions' }" @click="sidePanel = 'questions'">Q&A <span>{{ state?.questions.length || 0 }}</span></button>
          <button :class="{ active: sidePanel === 'insights' }" @click="sidePanel = 'insights'">Insights <i v-if="state?.ai_insights" /></button>
        </div>

        <div v-if="sidePanel === 'room'" class="console-scroll">
          <section class="audience-code-card">
            <div class="mini-qr"><QrcodeVue v-if="joinUrl" :value="joinUrl" :size="112" level="M" render-as="svg" /></div>
            <div><small>AUDIENCE CODE</small><strong>{{ deck?.join_code }}</strong><a :href="joinUrl" target="_blank">Open audience view ↗</a></div>
          </section>
          <div class="room-stat-grid"><article><strong>{{ state?.participant_count || 0 }}</strong><span>Connected</span></article><article><strong>{{ state?.interactions.length || 0 }}</strong><span>Active now</span></article></div>
          <section class="active-interactions">
            <h3>Current slide</h3>
            <article v-for="interaction in state?.interactions" :key="interaction.id" :class="{ 'ranked-console-item': interaction.kind === 'ranked-list' }">
              <div><span>{{ interaction.kind }}<template v-if="interaction.phase"> · {{ interaction.phase }}</template><template v-else-if="interaction.presenter_closed"> · closed</template></span><strong>{{ interaction.title }}</strong></div><b>{{ interaction.result.count || 0 }}</b>
              <div v-if="interaction.kind === 'ranked-list'" class="ranked-console-actions">
                <p v-if="interaction.phase !== 'ranked'">Voting is open · {{ interaction.revealed_count || 0 }} / {{ interaction.options.length }} items revealed</p>
                <p v-else>Voting closed · final order shown</p>
                <button
                  v-if="interaction.phase !== 'ranked'"
                  :disabled="(interaction.revealed_count || 0) < interaction.options.length || interactionActionPending"
                  @click="controlInteraction(interaction.id, 'close')"
                >{{ interactionActionPending ? 'Closing…' : 'Close voting' }}</button>
              </div>
              <button
                v-else
                class="interaction-lock-button"
                :disabled="interactionActionPending || state?.input_frozen"
                :title="state?.input_frozen ? 'All audience input is frozen' : 'Only this interaction stops accepting answers'"
                @click="controlInteraction(interaction.id, interaction.presenter_closed ? 'open' : 'close')"
              >{{ state?.input_frozen ? 'Paused globally' : interaction.presenter_closed ? 'Open input' : 'Close input' }}</button>
              <div v-if="interaction.kind !== 'ranked-list' && (interaction.config.results === 'manual' || interaction.config.results === 'on-close')" class="interaction-result-controls">
                <span>{{ interaction.results_revealed ? 'Results visible' : 'Results hidden' }}</span>
                <button :disabled="interactionActionPending" @click="controlInteraction(interaction.id, interaction.results_revealed ? 'hide' : 'reveal')">{{ interaction.results_revealed ? 'Hide results' : 'Reveal results' }}</button>
              </div>
              <div v-if="interaction.config.timer" class="interaction-timer-controls">
                <strong>{{ formatTimer(timerSeconds(interaction)) }}</strong>
                <button :disabled="interactionActionPending" @click="controlInteraction(interaction.id, interaction.timer_running ? 'timer-pause' : 'timer-start')">{{ interaction.timer_running ? 'Pause' : timerSeconds(interaction) < Number(interaction.config.timer) && timerSeconds(interaction) > 0 ? 'Resume' : 'Start' }}</button>
                <button :disabled="interactionActionPending" @click="controlInteraction(interaction.id, 'timer-reset')">Reset</button>
              </div>
            </article>
            <p v-if="!state?.interactions.length">No interactions on this slide. General Q&A is still open.</p>
            <p v-if="actionError" class="field-error">{{ actionError }}</p>
          </section>
          <button :class="['button', 'button-block', state?.input_frozen ? 'button-primary' : 'button-danger']" :disabled="inputControlPending" @click="controlInput">{{ inputControlPending ? 'Updating…' : state?.input_frozen ? 'Resume audience input' : 'Freeze all audience input' }}</button>
          <button class="button button-ghost button-block" @click="resetResults">Reset result epoch</button>
        </div>

        <div v-else-if="sidePanel === 'questions'" class="console-scroll questions-console">
          <section class="qa-display-control">
            <div><strong>Projected Q&A</strong><span>Changes the live Q&A slide immediately.</span></div>
            <div role="group" aria-label="Projected Q&A display mode">
              <button :class="{ active: state?.qa_display_mode === 'verbatim' }" :disabled="qaSettingPending" @click="updateQaDisplayMode('verbatim')">Verbatim</button>
              <button :class="{ active: state?.qa_display_mode === 'ai_grouped' }" :disabled="qaSettingPending || !state?.ai_configured" :title="state?.ai_configured ? 'Group related questions with Gemini' : 'Gemini moderation is not configured'" @click="updateQaDisplayMode('ai_grouped')">AI grouped</button>
            </div>
          </section>
          <div class="question-filters">
            <button v-for="filter in questionFilters" :key="filter" :class="{ active: questionFilter === filter }" @click="questionFilter = filter">{{ filter }}</button>
          </div>
          <article v-for="question in visibleQuestions" :key="question.id" :class="['present-question', question.moderation_status, { pinned: question.is_pinned }]">
            <div class="question-meta"><span>{{ question.display_name || 'Anonymous' }} · {{ question.slide_number ? `Slide ${question.slide_number}` : 'General' }}<template v-if="question.topic"> · {{ question.topic }}</template></span><b>▲ {{ question.votes }}</b></div>
            <p>{{ question.body }}</p>
            <div v-if="question.analysis_status === 'queued'" class="ai-review-state"><i /> AI review queued</div>
            <div v-else-if="question.analysis_status === 'failed'" class="ai-review-state failed">AI review unavailable · use manual controls</div>
            <div v-else-if="question.analysis_status === 'complete'" class="question-analysis">
              <div class="analysis-labels">
                <span v-if="!moderationFlags(question).length" class="analysis-clean">No risk flags</span>
                <span v-for="flag in moderationFlags(question)" :key="flag" class="analysis-flag">{{ flag }}</span>
                <span v-if="question.moderation_source === 'ai'" class="analysis-auto">AI decision</span>
              </div>
              <small v-if="question.moderation_reason">{{ question.moderation_reason }}</small>
              <small v-if="duplicateQuestion(question)" class="duplicate-note">Possible duplicate: “{{ duplicateQuestion(question)?.body }}”</small>
            </div>
            <div v-if="question.moderation_status === 'pending'" class="moderation-actions"><button @click="moderate(question, 'approved')">Approve</button><button @click="moderate(question, 'rejected')">Reject</button></div>
            <div v-else-if="question.moderation_status === 'approved' && question.lifecycle_status !== 'archived'" class="question-actions">
              <button @click="toggleQuestionPin(question)">{{ question.is_pinned ? 'Unpin' : 'Pin' }}</button>
              <button @click="markAnswered(question)">{{ question.lifecycle_status === 'answered' ? 'Reopen' : 'Mark answered' }}</button>
              <button @click="archiveQuestion(question)">Archive</button>
            </div>
            <div v-else-if="question.moderation_status === 'rejected' && question.lifecycle_status !== 'archived'" class="question-actions">
              <button @click="moderate(question, 'pending')">Reconsider</button>
              <button @click="archiveQuestion(question)">Archive</button>
            </div>
            <span v-else-if="question.lifecycle_status === 'archived'" class="answered-label">Archived</span>
            <span v-else class="answered-label">Rejected</span>
          </article>
          <div v-if="!visibleQuestions.length" class="console-empty">No questions in this view.</div>
        </div>

        <div v-else class="console-scroll insights-console">
          <section v-if="!state?.ai_configured" class="ai-empty">
            <strong>AI assistance is not configured</strong>
            <p>Manual moderation is fully available. Add <code>GEMINI_MODERATOR_KEY</code> to the app service when you want automatic moderation, themes, and summaries.</p>
          </section>
          <template v-else>
            <section class="insight-actions">
              <div>
                <strong>Presenter brief</strong>
                <span v-if="insightsPending">Gemini is grouping the approved questions now.</span>
                <span v-else-if="state?.ai_insights_status === 'failed'">The last attempt failed. Moderation is unaffected; you can retry.</span>
                <span v-else-if="ungroupedQuestionCount > 0 && state?.ai_insights">{{ ungroupedQuestionCount }} newer {{ ungroupedQuestionCount === 1 ? 'question is' : 'questions are' }} not in this brief.</span>
                <span v-else>Theme generation runs only when you request it.</span>
              </div>
              <button :disabled="insightsPending || analyzedApprovedQuestionCount === 0" @click="generateQaInsights">
                {{ insightsPending ? 'Generating…' : state?.ai_insights ? 'Refresh brief' : 'Generate brief' }}
              </button>
            </section>
            <p v-if="actionError" class="field-error">{{ actionError }}</p>
            <template v-if="state?.ai_insights">
              <div class="insight-heading"><span>GENERATED BRIEF</span><small>{{ state.ai_insights.question_count }} analyzed questions</small></div>
              <p class="insight-summary">{{ state.ai_insights.summary }}</p>
              <section class="theme-list">
                <h3>Themes</h3>
                <article v-for="theme in state.ai_insights.themes" :key="theme.label">
                  <div><strong>{{ theme.label }}</strong><span>{{ theme.question_ids.length }}</span></div>
                  <p class="theme-representative">{{ theme.representative_question || theme.questions?.[0]?.body || theme.summary }}</p>
                  <ul v-if="theme.questions?.length" class="theme-questions">
                    <li v-for="question in theme.questions" :key="question.id">
                      <p>{{ question.body }}</p>
                      <div><strong>{{ question.asker }}</strong><span>{{ question.scope === 'slide' ? `Slide ${question.slide_number}` : 'Deck-wide' }}</span></div>
                    </li>
                  </ul>
                </article>
              </section>
              <section v-if="state.ai_insights.suggested_answers.length" class="talking-points">
                <h3>Suggested talking points</h3>
                <ol><li v-for="answer in state.ai_insights.suggested_answers" :key="answer">{{ answer }}</li></ol>
              </section>
              <small class="insight-footnote">Generated {{ new Date(state.ai_insights.generated_at).toLocaleTimeString() }} · treat as a briefing aid, not a factual source.</small>
            </template>
            <section v-else class="ai-empty">
              <strong>No brief generated yet</strong>
              <p>Approved questions are available immediately. Generate a themed, bundled presenter brief only when it would be useful.</p>
            </section>
          </template>
        </div>

        <footer><button class="button button-danger button-block" :disabled="!state?.live" @click="endPresentation">{{ state?.live ? 'End presentation' : 'Presentation ended' }}</button></footer>
      </aside>
    </template>
  </main>
</template>
