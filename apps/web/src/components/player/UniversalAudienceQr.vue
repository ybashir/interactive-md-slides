<script setup lang="ts">
import QrcodeVue from 'qrcode.vue'
import { computed } from 'vue'
import type { LiveState } from '../../types'

const props = defineProps<{ joinUrl: string; size: number; state: LiveState | null }>()
const displayUrl = computed(() => props.joinUrl.replace(/^https?:\/\//, ''))
</script>

<template>
  <figure class="universal-audience-qr" :style="{ '--audience-qr-size': `${size}px` }">
    <div class="qr-shell"><QrcodeVue :value="joinUrl" :size="size" level="M" render-as="svg" /></div>
    <figcaption>
      <strong>Join the conversation</strong>
      <span class="audience-url">{{ displayUrl }}</span>
      <span v-if="state?.live" class="audience-total"><b>{{ state.participant_count || 0 }}</b> {{ state.participant_count === 1 ? 'person joined' : 'people joined' }}</span>
    </figcaption>
  </figure>
</template>

<style scoped>
.universal-audience-qr { display: inline-flex; align-items: center; gap: clamp(18px, calc(var(--audience-qr-size) * .11), 42px); margin: 10px 0; }
.qr-shell { padding: clamp(8px, calc(var(--audience-qr-size) * .055), 18px); border-radius: clamp(12px, calc(var(--audience-qr-size) * .075), 24px); background: white; line-height: 0; box-shadow: 0 8px 30px #0002; }
figcaption { display: grid; gap: clamp(6px, calc(var(--audience-qr-size) * .032), 14px); width: max-content; font-size: clamp(20px, calc(var(--audience-qr-size) * .13), 42px); line-height: 1.2; }
figcaption > strong { font-size: 1em; }
figcaption > span { opacity: .68; font-size: .74em; line-height: 1.35; }
.audience-url { white-space: nowrap; }
.audience-total { display: inline-flex; align-items: baseline; gap: clamp(5px, calc(var(--audience-qr-size) * .025), 10px); margin-top: clamp(5px, calc(var(--audience-qr-size) * .025), 10px); color: var(--slidev-theme-primary, #6366f1); opacity: 1; font-size: .9em; }
.audience-total b { font-size: 1.4em; line-height: 1; font-variant-numeric: tabular-nums; }
</style>

