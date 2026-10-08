<script setup lang="ts">
import { defineAsyncComponent } from 'vue'

import UniversalAudienceCount from './UniversalAudienceCount.vue'
import UniversalAudienceQr from './UniversalAudienceQr.vue'
import UniversalInteraction from './UniversalInteraction.vue'
import UniversalLiveQa from './UniversalLiveQa.vue'
import UniversalLiveSummary from './UniversalLiveSummary.vue'
import type { LiveState } from '../../types'
import type { UniversalSegment } from '../../universal-deck.mjs'

const UniversalChart = defineAsyncComponent(() => import('./UniversalChart.vue'))

const emit = defineEmits<{ answerQuestion: [questionIds: string[]] }>()
defineProps<{
  segments: UniversalSegment[]
  deckId: string
  joinUrl: string
  state: LiveState | null
  clickStep: number
  presenterControls: boolean
  answeringQuestionIds: string[]
}>()
</script>

<template>
  <template v-for="(segment, index) in segments" :key="index">
    <div v-if="segment.kind === 'html'" class="universal-markdown" v-html="segment.html" />
    <UniversalInteraction
      v-else-if="segment.kind === 'interaction' && segment.definition"
      :definition="segment.definition"
      :live="segment.live"
      :display="segment.display || 'card'"
      :reveal="segment.reveal || 'all'"
      :body-html="segment.bodyHtml || ''"
      :click-step="clickStep"
      :click-start="segment.clickStart || 0"
    />
    <UniversalChart
      v-else-if="segment.kind === 'chart' && segment.spec"
      :spec="segment.spec"
      :definition="segment.definition"
      :live="segment.live"
      :click-step="clickStep"
      :click-start="segment.clickStart || 0"
    />
    <UniversalAudienceQr
      v-else-if="segment.kind === 'qr'"
      :join-url="joinUrl"
      :size="segment.size || 180"
      :state="state"
    />
    <UniversalAudienceCount
      v-else-if="segment.kind === 'audience-count'"
      :label="segment.label || 'people joined'"
      :state="state"
    />
    <UniversalLiveQa
      v-else-if="segment.kind === 'live-qa'"
      :state="state"
      :max="segment.max || 8"
      :show="segment.show || 'open'"
      :presenter-controls="presenterControls"
      :answering-question-ids="answeringQuestionIds"
      @answer="emit('answerQuestion', $event)"
    />
    <UniversalLiveSummary
      v-else-if="segment.kind === 'live-summary'"
      :deck-id="deckId"
      :state="state"
      :show="segment.show || 'responded'"
    />
  </template>
</template>
