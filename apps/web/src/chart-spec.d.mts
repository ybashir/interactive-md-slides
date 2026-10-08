export interface ChartSeries {
  name: string
  data: Array<number | [number, number]>
}

export interface TableChartSpec {
  type: 'bar' | 'line' | 'area' | 'pie' | 'donut' | 'scatter' | 'radar' | 'heatmap'
  title: string
  height: number
  legend: 'top' | 'bottom' | 'false'
  legendGap: number
  stacked: boolean
  showValues: boolean
  showXLabels: boolean
  showYLabels: boolean
  showXTicks: boolean
  showYTicks: boolean
  showGrid: boolean
  smooth: boolean
  colors: string[]
  source: string
  labels?: string[]
  series: ChartSeries[]
  xLabel?: string
  yLabel?: string
  xLabels?: string[]
  yLabels?: string[]
  data?: Array<[number, number, number]>
  min?: number
  max?: number
}

export interface NativeEChartsSpec {
  dialect: 'echarts'
  height: number
  source: string
  reveal: 'all' | 'series'
  option: Record<string, any>
}

export type ChartSpec = TableChartSpec | NativeEChartsSpec

export function parseChartSpec(attributes: string | Record<string, string>, body: string): TableChartSpec
export function parseEChartsSpec(attributes: string | Record<string, string>, body: string): NativeEChartsSpec
export function deriveLiveChartSpec(spec: TableChartSpec, definition?: Record<string, any>, live?: Record<string, any>): TableChartSpec
export function buildEChartsOption(spec: TableChartSpec, colors?: Record<string, string>): Record<string, any>
export function resolveChartOption(spec: ChartSpec, definition?: Record<string, any>, live?: Record<string, any>, colors?: Record<string, string>, visibleSeries?: number): Record<string, any>
export function chartAccessibleLabel(spec: ChartSpec): string
export function chartRevealClicks(spec: ChartSpec): number
