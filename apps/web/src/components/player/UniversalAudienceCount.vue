<script setup lang="ts">
import type { LiveState } from '../../types'
defineProps<{ label: string; state: LiveState | null }>()
</script>

<template>
  <figure :class="['universal-audience-count', { live: state?.live }]" aria-live="polite">
    <span>{{ state?.live ? 'LIVE AUDIENCE' : 'WAITING FOR PRESENTER' }}</span>
    <strong>{{ state?.live ? state.participant_count || 0 : 0 }}</strong>
    <b>{{ label }}</b>
  </figure>
</template>

<style scoped>
.universal-audience-count { display: inline-grid; justify-items: center; min-width: 220px; padding: 24px 30px; border: 2px solid color-mix(in srgb, currentColor 14%, transparent); border-radius: 20px; background: color-mix(in srgb, var(--slidev-theme-primary, #6366f1) 5%, transparent); }
.universal-audience-count span { color: var(--slidev-theme-primary, #6366f1); font-size: .48em; font-weight: 850; letter-spacing: .12em; }
.universal-audience-count strong { color: var(--slidev-theme-primary, #6366f1); font-size: 3.5em; line-height: 1.05; font-variant-numeric: tabular-nums; }
.universal-audience-count b { font-size: .8em; opacity: .72; }
</style>
