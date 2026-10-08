<script setup lang="ts">
import * as echarts from 'echarts'
import type { ECharts } from 'echarts'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import { chartAccessibleLabel, chartRevealClicks, resolveChartOption } from '../../chart-spec.mjs'
import type { ChartSpec } from '../../chart-spec.mjs'

const props = defineProps<{ spec: ChartSpec; definition?: Record<string, any>; live?: Record<string, any>; clickStep?: number; clickStart?: number }>()
const container = ref<HTMLElement | null>(null)
const accessibleLabel = computed(() => chartAccessibleLabel(props.spec))
const visibleSeries = computed(() => {
  const extraClicks = chartRevealClicks(props.spec)
  if (!extraClicks) return undefined
  const revealed = Math.max(0, Number(props.clickStep || 0) - Number(props.clickStart || 0) + 1)
  return 1 + Math.min(extraClicks, revealed)
})
let chart: ECharts | null = null
let resizeObserver: ResizeObserver | null = null
let previousSignature = ''
let previousVisibleSeries: number | undefined

function colors(): Record<string, string> {
  if (!container.value) return {}
  const style = window.getComputedStyle(container.value)
  return {
    primary: style.getPropertyValue('--slidev-theme-primary').trim() || '#6657d9',
    text: style.color || '#172033',
  }
}

function render() {
  if (!container.value) return
  if (!chart) chart = echarts.init(container.value, undefined, { renderer: 'svg' })
  const signature = JSON.stringify(props.spec)
  const mergeSeries = signature === previousSignature && visibleSeries.value !== previousVisibleSeries
  chart.setOption(
    resolveChartOption(props.spec, props.definition, props.live, colors(), visibleSeries.value),
    mergeSeries ? { notMerge: false, replaceMerge: ['series'] } : { notMerge: true },
  )
  previousSignature = signature
  previousVisibleSeries = visibleSeries.value
  chart.resize()
}

onMounted(() => {
  render()
  resizeObserver = new ResizeObserver(() => chart?.resize())
  resizeObserver.observe(container.value!)
})

watch(() => [props.spec, props.definition, props.live, props.clickStep], () => nextTick(render), { deep: true })

onBeforeUnmount(() => {
  resizeObserver?.disconnect()
  chart?.dispose()
})
</script>

<template>
  <figure class="universal-chart" :style="{ height: `${props.spec.height}px` }">
    <div ref="container" role="img" :aria-label="accessibleLabel" />
  </figure>
</template>

<style scoped>
.universal-chart { width: 100%; min-width: 0; margin: 8px 0; }
.universal-chart > div { width: 100%; height: 100%; }
</style>
