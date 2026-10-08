<script setup lang="ts">
import { computed, ref } from 'vue'
import type { LiveState, Question } from '../../types'

const props = defineProps<{
  state: LiveState | null
  max: number
  show: string
  presenterControls: boolean
  answeringQuestionIds: string[]
}>()
const emit = defineEmits<{ answer: [questionIds: string[]] }>()
const scrollRegion = ref<HTMLElement | null>(null)
const answeringIds = computed(() => new Set(props.answeringQuestionIds))
const questions = computed(() => (props.state?.questions || [])
  .filter(question => question.moderation_status === 'approved'
    && question.lifecycle_status !== 'archived'
    && (props.show === 'all' || question.lifecycle_status === 'open'))
  .sort(compareQuestions))
const displayMode = computed(() => props.state?.qa_display_mode || 'verbatim')
const questionById = computed(() => new Map(questions.value.map(question => [question.id, question])))
const groups = computed(() => {
  const grouped = new Set<string>()
  const values = (props.state?.ai_insights?.themes || []).map(theme => {
    const items = (theme.question_ids || []).map(id => questionById.value.get(id)).filter(Boolean) as Question[]
    items.forEach(question => grouped.add(question.id))
    return {
      label: theme.label,
      question: theme.representative_question || items[0]?.body || theme.summary,
      items,
      votes: items.reduce((sum, question) => sum + question.votes, 0),
      priority: items.reduce((sum, question) => sum + (question.priority_score || 0), 0),
    }
  }).filter(group => group.items.length)
  values.push(...questions.value.filter(question => !grouped.has(question.id)).map(question => ({
    label: 'New question', question: question.body, items: [question], votes: question.votes, priority: question.priority_score || 0,
  })))
  return values.sort((a, b) => b.priority - a.priority || b.votes - a.votes).slice(0, Math.max(1, Math.min(props.max, 8)))
})

function compareQuestions(left: Question, right: Question) {
  return Number(right.is_pinned) - Number(left.is_pinned)
    || (right.priority_score || 0) - (left.priority_score || 0)
    || new Date(left.created_at).getTime() - new Date(right.created_at).getTime()
}
function scope(question: Question) {
  return question.slide_number ? `Slide ${question.slide_number}` : 'Deck-wide'
}
function markAnswered(items: Question[]) {
  const ids = items.map(item => item.id).filter(id => !answeringIds.value.has(id))
  if (props.presenterControls && ids.length) emit('answer', ids)
}
function isAnswering(items: Question[]) {
  return items.some(item => answeringIds.value.has(item.id))
}
function scrollQuestions(event: KeyboardEvent) {
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
  <section class="universal-live-qa" aria-live="polite">
    <header><div><i /><span>{{ displayMode === 'ai_grouped' ? 'AI-GROUPED Q&A' : 'APPROVED QUESTIONS' }}</span></div><strong>{{ questions.length }} question{{ questions.length === 1 ? '' : 's' }}</strong></header>
    <div ref="scrollRegion" class="qa-scroll" tabindex="0" aria-label="Scrollable approved questions" @keydown="scrollQuestions" @wheel.stop>
      <div v-if="displayMode === 'verbatim'" class="qa-list">
        <article v-for="(question, index) in questions" :key="question.id" :class="{ pinned: question.is_pinned }">
          <b>{{ index + 1 }}</b><div><p>{{ question.body }}</p><span>{{ question.display_name || 'Anonymous' }} · {{ scope(question) }}<template v-if="question.repeat_count && question.repeat_count > 1"> · asked {{ question.repeat_count }} times</template></span></div>
          <div class="qa-card-actions"><strong>▲ {{ question.votes }}</strong><button v-if="presenterControls" :disabled="isAnswering([question])" :aria-label="`Mark question ${index + 1} answered`" title="Mark answered and remove from this slide" @click.stop="markAnswered([question])">{{ isAnswering([question]) ? '…' : '✓ Done' }}</button></div>
        </article>
      </div>
      <div v-else class="qa-list">
        <article v-for="(group, index) in groups" :key="`${group.label}-${index}`">
          <b>{{ index + 1 }}</b><div><small>{{ group.label }}</small><p>{{ group.question }}</p><span>{{ group.items.map(item => item.display_name || 'Anonymous').join(', ') }}</span></div>
          <div class="qa-card-actions"><strong>▲ {{ group.votes }}</strong><button v-if="presenterControls" :disabled="isAnswering(group.items)" :aria-label="`Mark question group ${index + 1} answered`" title="Mark every question in this group answered" @click.stop="markAnswered(group.items)">{{ isAnswering(group.items) ? '…' : '✓ Done' }}</button></div>
        </article>
      </div>
      <div v-if="!questions.length" class="qa-empty">Approved questions will appear here automatically.</div>
    </div>
  </section>
</template>

<style scoped>
.universal-live-qa { display: grid; grid-template-rows: auto minmax(0, 1fr); width: 100%; height: 360px; min-height: 0; max-height: 360px; }
.universal-live-qa > header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 16px; font-size: .58em; }
.universal-live-qa > header div { display: flex; align-items: center; gap: 9px; color: var(--slidev-theme-primary); font-weight: 850; letter-spacing: .12em; }
.universal-live-qa > header i { width: 9px; height: 9px; border-radius: 50%; background: #22c55e; box-shadow: 0 0 0 5px #22c55e20; }
.universal-live-qa > header strong { padding: 6px 12px; border-radius: 999px; background: color-mix(in srgb, currentColor 7%, transparent); }
.qa-scroll { min-height: 0; padding: 2px 10px 2px 2px; overflow-y: auto; overscroll-behavior: contain; scroll-snap-type: y proximity; scrollbar-gutter: stable; scrollbar-color: color-mix(in srgb, var(--slidev-theme-primary) 55%, transparent) transparent; outline: none; touch-action: pan-y; }
.qa-scroll:focus-visible { border-radius: 12px; box-shadow: 0 0 0 3px color-mix(in srgb, var(--slidev-theme-primary) 22%, transparent); }
.qa-list { display: grid; gap: 12px; }
.qa-list article { display: grid; grid-template-columns: 42px minmax(0, 1fr) auto; align-items: start; gap: 16px; padding: 15px 18px; border: 2px solid color-mix(in srgb, currentColor 12%, transparent); border-radius: 16px; background: color-mix(in srgb, currentColor 3%, transparent); scroll-margin-top: 2px; scroll-snap-align: start; }
.qa-list article > b { display: grid; place-items: center; width: 36px; height: 36px; border-radius: 50%; color: white; background: var(--slidev-theme-primary); font-size: .6em; }
.qa-list p { margin: 0 0 7px; font-size: .82em; font-weight: 720; line-height: 1.32; }
.qa-list span, .qa-list small { display: block; font-size: .48em; opacity: .66; }
.qa-list small { margin-bottom: 5px; color: var(--slidev-theme-primary); font-weight: 800; letter-spacing: .08em; text-transform: uppercase; }
.qa-card-actions { display: grid; justify-items: end; gap: 8px; min-width: 74px; }
.qa-card-actions strong { color: var(--slidev-theme-primary); font-size: .58em; white-space: nowrap; }
.qa-card-actions button { padding: 5px 9px; color: white; border: 0; border-radius: 999px; background: var(--slidev-theme-primary); font: 750 12px/1.2 Inter, ui-sans-serif, system-ui, sans-serif; white-space: nowrap; cursor: pointer; opacity: 0; pointer-events: none; transform: translateY(-2px); transition: opacity .15s ease, transform .15s ease; }
.qa-list article:hover .qa-card-actions button, .qa-card-actions button:focus-visible { opacity: 1; pointer-events: auto; transform: translateY(0); }
.qa-card-actions button:focus-visible { outline: 3px solid color-mix(in srgb, var(--slidev-theme-primary) 28%, transparent); outline-offset: 2px; }
.qa-card-actions button:disabled { cursor: wait; opacity: .58; }
@media (hover: none) { .qa-card-actions button { opacity: 1; pointer-events: auto; transform: none; } }
.qa-empty { display: grid; place-items: center; min-height: 220px; color: currentColor; border: 2px dashed color-mix(in srgb, currentColor 14%, transparent); border-radius: 18px; font-size: .7em; opacity: .55; }
</style>
