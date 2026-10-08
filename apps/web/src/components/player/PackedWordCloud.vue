<script setup lang="ts">
import VueWordCloud from 'vuewordcloud'
import { computed, shallowRef, watch } from 'vue'

const props = defineProps<{
  words: Array<{ text: string; count: number }>
  height?: number | string
  width?: string
  spacing?: number
}>()

const palette = ['#2563eb', '#7c3aed', '#db2777', '#059669', '#d97706', '#dc2626']
const rotations = [0, 0, 0, 0, 0, 90, -90]
const cloudWords = shallowRef<Array<Record<string, string | number>>>([])
const signature = computed(() => JSON.stringify(
  props.words.map(word => [String(word.text || ''), Number(word.count || 0)]),
))
const cloudStyle = computed(() => ({
  height: typeof props.height === 'number' ? `${props.height}px` : props.height || '320px',
  width: props.width || '100%',
}))

function stableHash(text: string) {
  let hash = 2166136261
  for (let index = 0; index < text.length; index += 1) {
    hash = Math.imul(hash ^ text.charCodeAt(index), 16777619)
  }
  return hash >>> 0
}

watch(signature, () => {
  cloudWords.value = props.words
    .slice(0, 50)
    .map((word) => {
      const text = Array.from(String(word.text || '').trim()).slice(0, 40).join('')
      const hash = stableHash(text)
      return {
        text,
        weight: Math.min(100_000, Math.max(1, Number(word.count || 0))),
        color: palette[hash % palette.length],
        rotation: rotations[(hash >>> 8) % rotations.length],
        rotationUnit: 'deg',
        fontFamily: 'Inter',
        fontWeight: 700,
      }
    })
    .filter(word => word.text)
}, { immediate: true })
</script>

<template>
  <VueWordCloud
    class="packed-word-cloud"
    :style="cloudStyle"
    :words="cloudWords"
    :font-size-ratio="3.5"
    :spacing="spacing ?? 0.16"
    :animation-duration="350"
    :animation-overlap="1"
    font-family="Inter"
    font-weight="700"
    role="img"
    :aria-label="`Word cloud with ${cloudWords.length} entries`"
  >
    <template #default="{ text, weight }">
      <span :title="`${text} · ${weight} ${weight === 1 ? 'person' : 'people'}`">{{ text }}</span>
    </template>
  </VueWordCloud>
</template>

<style scoped>
.packed-word-cloud { display: block; min-height: 180px; overflow: hidden; contain: layout paint; }
.packed-word-cloud :deep(span) { max-width: 100%; overflow-wrap: anywhere; }
</style>
