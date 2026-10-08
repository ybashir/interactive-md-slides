<script setup lang="ts">
import { computed } from 'vue'
import { RouterView, useRoute } from 'vue-router'

const route = useRoute()
const immersive = computed(() => route.path.includes('/present') || route.path.startsWith('/j/'))
</script>

<template>
  <div :class="['app-shell', { immersive }]">
    <RouterView />
  </div>
</template>
