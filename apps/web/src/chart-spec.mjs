const SUPPORTED_TYPES = new Set(['bar', 'line', 'area', 'pie', 'donut', 'scatter', 'radar', 'heatmap'])
const SUPPORTED_ATTRIBUTES = new Set([
  'type', 'title', 'height', 'legend', 'legend-gap', 'stacked', 'values',
  'x-label', 'y-label', 'x-labels', 'y-labels', 'x-ticks', 'y-ticks',
  'grid', 'smooth', 'colors', 'source',
])
const ECHARTS_ATTRIBUTES = new Set(['height', 'source', 'reveal'])
const BLOCKED_JSON_KEYS = new Set(['__proto__', 'prototype', 'constructor'])

export function parseEChartsSpec(rawAttributes, body) {
  const attributes = typeof rawAttributes === 'string' ? parseAttributes(rawAttributes) : { ...(rawAttributes || {}) }
  const unsupportedAttribute = Object.keys(attributes).find(attribute => !ECHARTS_ATTRIBUTES.has(attribute))
  if (unsupportedAttribute) throw new Error(`Unsupported ECharts wrapper attribute \`${unsupportedAttribute}\``)

  const height = strictNumber(attributes.height, 180, 520, 320, 'ECharts `height`')
  const source = String(attributes.source || '').trim()
  if (source && !/^[a-zA-Z0-9][a-zA-Z0-9_-]{1,63}$/.test(source))
    throw new Error('ECharts `source` must be a valid interaction ID')
  const reveal = String(attributes.reveal || 'all').trim().toLowerCase()
  if (!['all', 'series'].includes(reveal))
    throw new Error('ECharts `reveal` must be `all` or `series`')

  const rawOption = String(body || '').trim()
  if (!rawOption) throw new Error('ECharts needs a JSON option object')
  if (rawOption.length > 100_000) throw new Error('ECharts JSON must be at most 100 KB')

  let option
  try {
    option = JSON.parse(rawOption)
  } catch (error) {
    throw new Error(`ECharts option is not valid JSON: ${error instanceof Error ? error.message : String(error)}`)
  }
  if (!option || Array.isArray(option) || typeof option !== 'object')
    throw new Error('ECharts option must be a JSON object')
  validateEChartsValue(option)
  if (source) validateLiveEChartsOption(option)
  return { dialect: 'echarts', height, source, reveal, option }
}

export function resolveChartOption(spec, definition, live, colors = {}, visibleSeries) {
  if (spec?.dialect !== 'echarts') {
    const resolved = deriveLiveChartSpec(spec, definition, live)
    return buildEChartsOption(resolved, colors)
  }

  const option = JSON.parse(JSON.stringify(spec.option || {}))
  if (spec.source) injectLiveDataset(option, spec.source, definition, live)
  if (option.backgroundColor == null) option.backgroundColor = 'transparent'
  if (option.aria == null) option.aria = { enabled: true }
  if (option.animationDuration == null) option.animationDuration = 450
  if (option.animationDurationUpdate == null) option.animationDurationUpdate = 350
  applySeriesReveal(option, spec, visibleSeries)
  return option
}

export function chartRevealClicks(spec) {
  if (spec?.dialect !== 'echarts' || spec.reveal !== 'series' || !Array.isArray(spec.option?.series)) return 0
  return Math.max(0, spec.option.series.length - 1)
}

export function chartAccessibleLabel(spec) {
  if (spec?.dialect !== 'echarts') return spec?.title || `${spec?.type || 'Chart'} chart`
  const title = Array.isArray(spec.option?.title) ? spec.option.title[0] : spec.option?.title
  return String(title?.text || spec.option?.aria?.label?.description || 'ECharts visualization')
}

export function parseChartSpec(rawAttributes, body) {
  const attributes = typeof rawAttributes === 'string' ? parseAttributes(rawAttributes) : { ...(rawAttributes || {}) }
  const unsupportedAttribute = Object.keys(attributes).find(attribute => !SUPPORTED_ATTRIBUTES.has(attribute))
  if (unsupportedAttribute) throw new Error(`Unsupported chart attribute \`${unsupportedAttribute}\``)
  const type = String(attributes.type || 'bar').toLowerCase()
  if (!SUPPORTED_TYPES.has(type))
    throw new Error(`Unsupported chart type \`${type}\``)

  const common = {
    type,
    title: String(attributes.title || '').trim(),
    height: clampNumber(attributes.height, 180, 520, 320),
    legend: normalizeLegend(attributes.legend),
    legendGap: clampNumber(attributes['legend-gap'], 0, 80, 28),
    stacked: booleanAttribute(attributes, 'stacked', false),
    showValues: booleanAttribute(attributes, 'values', false),
    xLabel: String(attributes['x-label'] || '').trim(),
    yLabel: String(attributes['y-label'] || '').trim(),
    showXLabels: booleanAttribute(attributes, 'x-labels', true),
    showYLabels: booleanAttribute(attributes, 'y-labels', true),
    showXTicks: booleanAttribute(attributes, 'x-ticks', false),
    showYTicks: booleanAttribute(attributes, 'y-ticks', false),
    showGrid: booleanAttribute(attributes, 'grid', true),
    smooth: booleanAttribute(attributes, 'smooth', true),
    colors: parseColors(attributes.colors),
    source: String(attributes.source || '').trim(),
    labels: [],
    series: [],
  }

  if (common.source) {
    if (!/^[a-zA-Z0-9][a-zA-Z0-9_-]{1,63}$/.test(common.source))
      throw new Error('Chart `source` must be a valid interaction ID')
    return { ...common, labels: [], series: [] }
  }

  const table = parseMarkdownTable(body)
  if (type === 'scatter') return { ...common, ...parseScatter(table) }
  if (type === 'heatmap') return { ...common, ...parseHeatmap(table) }
  return { ...common, ...parseCategorySeries(table, type) }
}

export function deriveLiveChartSpec(spec, definition, live) {
  if (!spec?.source) return spec
  const item = live || {}
  const kind = item.kind || definition?.kind || ''
  const options = item.options?.length ? item.options : definition?.options || []
  const result = item.result || {}
  const labels = options.map(option => option.label)

  if (kind === 'matrix') {
    return {
      ...spec,
      type: 'scatter',
      xLabel: item.config?.['x-label'] || definition?.config?.['x-label'] || 'X',
      yLabel: item.config?.['y-label'] || definition?.config?.['y-label'] || 'Y',
      series: [{ name: 'Responses', data: (result.points || []).map(point => [Number(point.x), Number(point.y)]) }],
    }
  }
  if (kind === 'ranked-list') {
    return {
      ...spec,
      labels,
      series: [
        { name: 'Upvotes', data: options.map(option => Number(result.scores?.[option.id]?.up || 0)) },
        { name: 'Downvotes', data: options.map(option => Number(result.scores?.[option.id]?.down || 0)) },
      ],
    }
  }
  if (kind === 'allocation') {
    return { ...spec, labels, series: [{ name: 'Average allocation', data: options.map(option => Number(result.averages?.[option.id] || 0)) }] }
  }
  if (kind === 'ranking') {
    return { ...spec, labels, series: [{ name: 'Average rank', data: options.map(option => Number(result.scores?.[option.id]?.average_rank || 0)) }] }
  }
  if (['poll', 'quiz', 'reaction', 'image-choice'].includes(kind)) {
    return { ...spec, labels, series: [{ name: 'Responses', data: options.map(option => Number(result.counts?.[option.id] || 0)) }] }
  }
  if (kind === 'rating') {
    return { ...spec, labels: ['Average'], series: [{ name: 'Rating', data: [Number(result.average || 0)] }] }
  }
  if (kind === 'number') {
    const values = [['Average', result.average], ['Median', result.median], ['Minimum', result.min], ['Maximum', result.max]].filter(([, value]) => value != null)
    return { ...spec, labels: values.map(([label]) => label), series: [{ name: item.config?.unit || 'Value', data: values.map(([, value]) => Number(value)) }] }
  }
  return { ...spec, labels: [], series: [{ name: 'Responses', data: [] }] }
}

export function buildEChartsOption(spec, colors = {}) {
  const primary = colors.primary || '#6657d9'
  const text = colors.text || '#172033'
  const muted = colors.muted || '#667085'
  const grid = colors.grid || 'rgba(102, 112, 133, .2)'
  const defaultPalette = [primary, '#36b8d4', '#22a06b', '#f5a524', '#e5484d', '#8e4ec6', '#687076', '#cf3897']
  const palette = [...(spec.colors || []), ...defaultPalette]
  const base = {
    animationDuration: 450,
    animationDurationUpdate: 350,
    color: palette,
    backgroundColor: 'transparent',
    textStyle: { color: text, fontFamily: 'inherit' },
    aria: { enabled: true },
    tooltip: { trigger: spec.type === 'pie' || spec.type === 'donut' ? 'item' : 'axis' },
    title: spec.title ? { text: spec.title, left: 'center', textStyle: { color: text, fontSize: 18 } } : undefined,
  }
  const titleTop = spec.title ? 52 : 18
  const legend = legendOption(spec, text)
  const legendTop = spec.legend === 'top' && spec.series.length > 1
  const legendBottom = spec.legend === 'bottom' && spec.series.length > 1

  if (['bar', 'line', 'area'].includes(spec.type)) {
    return {
      ...base,
      legend,
      grid: { top: legendTop ? Math.max(titleTop, 62) : titleTop, right: 24, bottom: legendBottom ? 76 : 48, left: 64, containLabel: true },
      xAxis: { type: 'category', name: spec.xLabel, nameLocation: 'middle', nameGap: spec.showXLabels ? 34 : 22, data: spec.labels, axisLabel: { show: spec.showXLabels, color: muted }, axisLine: { lineStyle: { color: grid } }, axisTick: { show: spec.showXTicks } },
      yAxis: { type: 'value', name: spec.yLabel, nameLocation: 'middle', nameGap: spec.showYLabels ? 48 : 28, axisLabel: { show: spec.showYLabels, color: muted }, axisTick: { show: spec.showYTicks }, splitLine: { show: spec.showGrid, lineStyle: { color: grid } } },
      series: spec.series.map(series => ({
        name: series.name,
        type: spec.type === 'bar' ? 'bar' : 'line',
        data: series.data,
        stack: spec.stacked ? 'total' : undefined,
        smooth: spec.type !== 'bar' && spec.smooth,
        areaStyle: spec.type === 'area' ? { opacity: .2 } : undefined,
        label: spec.showValues ? { show: true, position: spec.type === 'bar' ? 'top' : 'top' } : undefined,
        emphasis: { focus: 'series' },
      })),
    }
  }

  if (spec.type === 'pie' || spec.type === 'donut') {
    const source = spec.series[0]
    const pieLegend = legendOption(spec, text, true)
    return {
      ...base,
      legend: pieLegend,
      series: [{
        name: source.name,
        type: 'pie',
        radius: spec.type === 'donut' ? ['42%', '70%'] : '70%',
        center: ['50%', spec.legend === 'top' ? '58%' : '50%'],
        avoidLabelOverlap: true,
        data: spec.labels.map((label, index) => ({ name: label, value: source.data[index] })),
        label: { color: text, formatter: spec.showValues ? '{b}: {c}' : '{b}' },
        emphasis: { scale: true },
      }],
    }
  }

  if (spec.type === 'radar') {
    const maxima = spec.labels.map((_label, index) => Math.max(1, ...spec.series.map(series => Number(series.data[index] || 0))) * 1.1)
    return {
      ...base,
      legend,
      radar: { center: ['50%', '56%'], radius: '66%', indicator: spec.labels.map((name, index) => ({ name, max: maxima[index] })), axisName: { color: text }, splitLine: { lineStyle: { color: grid } }, splitArea: { show: false } },
      series: [{ type: 'radar', data: spec.series.map(series => ({ name: series.name, value: series.data, areaStyle: { opacity: .12 } })) }],
    }
  }

  if (spec.type === 'scatter') {
    return {
      ...base,
      legend,
      grid: { top: legendTop ? Math.max(titleTop, 62) : titleTop, right: 24, bottom: legendBottom ? 76 : 48, left: 64, containLabel: true },
      xAxis: { type: 'value', name: spec.xLabel, nameLocation: 'middle', nameGap: spec.showXLabels ? 34 : 22, axisLabel: { show: spec.showXLabels, color: muted }, axisTick: { show: spec.showXTicks }, splitLine: { show: spec.showGrid, lineStyle: { color: grid } } },
      yAxis: { type: 'value', name: spec.yLabel, nameLocation: 'middle', nameGap: spec.showYLabels ? 48 : 28, axisLabel: { show: spec.showYLabels, color: muted }, axisTick: { show: spec.showYTicks }, splitLine: { show: spec.showGrid, lineStyle: { color: grid } } },
      series: spec.series.map(series => ({ name: series.name, type: 'scatter', data: series.data, symbolSize: 13, emphasis: { focus: 'series', scale: 1.4 } })),
    }
  }

  return {
    ...base,
    grid: { top: titleTop, right: 48, bottom: 46, left: 70, containLabel: true },
    xAxis: { type: 'category', name: spec.xLabel, data: spec.xLabels, axisLabel: { show: spec.showXLabels, color: muted }, axisTick: { show: spec.showXTicks }, splitArea: { show: spec.showGrid } },
    yAxis: { type: 'category', name: spec.yLabel, data: spec.yLabels, axisLabel: { show: spec.showYLabels, color: muted }, axisTick: { show: spec.showYTicks }, splitArea: { show: spec.showGrid } },
    visualMap: { min: spec.min, max: spec.max, calculable: true, orient: 'horizontal', left: 'center', bottom: 0, textStyle: { color: text } },
    series: [{ type: 'heatmap', data: spec.data, label: { show: spec.showValues }, emphasis: { itemStyle: { shadowBlur: 8, shadowColor: '#0005' } } }],
  }
}

function legendOption(spec, text, force = false) {
  if (spec.legend === 'false' || (!force && spec.series.length < 2)) return undefined
  return {
    type: 'scroll',
    orient: 'horizontal',
    left: 'center',
    top: spec.legend === 'top' ? (spec.title ? 32 : 0) : undefined,
    bottom: spec.legend === 'bottom' ? 0 : undefined,
    width: '86%',
    itemGap: spec.legendGap,
    textStyle: { color: text },
  }
}

function parseCategorySeries(table, type) {
  if (table.headers.length < 2)
    throw new Error('Chart tables need a label column and at least one numeric series')
  if (['pie', 'donut'].includes(type) && table.headers.length !== 2)
    throw new Error('Pie and donut charts support exactly one numeric series')
  const labels = table.rows.map(row => row[0])
  const series = table.headers.slice(1).map((name, column) => ({
    name,
    data: table.rows.map((row, rowIndex) => numericCell(row[column + 1], rowIndex + 3, column + 2)),
  }))
  return { labels, series }
}

function parseScatter(table) {
  if (table.headers.length !== 3)
    throw new Error('Scatter charts need exactly three columns: Series, X, and Y')
  const grouped = new Map()
  table.rows.forEach((row, index) => {
    const name = row[0] || 'Series'
    if (!grouped.has(name)) grouped.set(name, [])
    grouped.get(name).push([numericCell(row[1], index + 3, 2), numericCell(row[2], index + 3, 3)])
  })
  return {
    xLabel: table.headers[1],
    yLabel: table.headers[2],
    series: [...grouped].map(([name, data]) => ({ name, data })),
  }
}

function parseHeatmap(table) {
  if (table.headers.length !== 3)
    throw new Error('Heatmaps need exactly three columns: X category, Y category, and Value')
  const xLabels = unique(table.rows.map(row => row[0]))
  const yLabels = unique(table.rows.map(row => row[1]))
  const values = table.rows.map((row, index) => numericCell(row[2], index + 3, 3))
  const data = table.rows.map((row, index) => [xLabels.indexOf(row[0]), yLabels.indexOf(row[1]), values[index]])
  return { xLabels, yLabels, data, min: Math.min(...values), max: Math.max(...values) }
}

function parseMarkdownTable(body) {
  const lines = String(body || '').split(/\r?\n/).map(line => line.trim()).filter(Boolean)
  if (lines.length < 3)
    throw new Error('Chart data must be a Markdown table with a header and at least one data row')
  const headers = splitTableRow(lines[0])
  const separators = splitTableRow(lines[1])
  if (headers.length < 2 || separators.length !== headers.length || !separators.every(cell => /^:?-{3,}:?$/.test(cell)))
    throw new Error('Chart data needs a valid Markdown table separator row')
  const rows = lines.slice(2).map((line, index) => {
    const cells = splitTableRow(line)
    if (cells.length !== headers.length)
      throw new Error(`Chart table row ${index + 3} has ${cells.length} columns; expected ${headers.length}`)
    if (!cells[0]) throw new Error(`Chart table row ${index + 3} needs a label`)
    return cells
  })
  if (!rows.length) throw new Error('Chart data needs at least one row')
  return { headers, rows }
}

function splitTableRow(line) {
  const source = String(line || '').replace(/^\|/, '').replace(/\|$/, '')
  const cells = []
  let current = ''
  let escaped = false
  for (const character of source) {
    if (escaped) {
      current += character
      escaped = false
    } else if (character === '\\') {
      escaped = true
    } else if (character === '|') {
      cells.push(current.trim())
      current = ''
    } else current += character
  }
  cells.push(current.trim())
  return cells
}

function numericCell(value, row, column) {
  const number = Number(String(value || '').replaceAll(',', '').replace(/%$/, ''))
  if (!Number.isFinite(number)) throw new Error(`Chart table row ${row}, column ${column} must be numeric`)
  return number
}

function parseAttributes(source) {
  const attributes = {}
  const pattern = /([a-zA-Z][a-zA-Z0-9_-]*)=(?:"([^"]*)"|'([^']*)'|([^\s]+))/g
  for (const match of String(source || '').matchAll(pattern)) attributes[match[1]] = match[2] ?? match[3] ?? match[4]
  return attributes
}

function injectLiveDataset(option, source, definition, live) {
  const derived = deriveLiveChartSpec({
    type: 'bar', title: '', height: 320, legend: 'top', legendGap: 28,
    stacked: false, showValues: false, showXLabels: true, showYLabels: true,
    showXTicks: false, showYTicks: false, showGrid: true, smooth: true,
    colors: [], source, labels: [], series: [],
  }, definition, live)

  let datasetSource
  if (derived.type === 'scatter') {
    datasetSource = [
      ['Series', derived.xLabel || 'X', derived.yLabel || 'Y'],
      ...derived.series.flatMap(series => series.data.map(point => [series.name, point[0], point[1]])),
    ]
  } else {
    datasetSource = [
      ['Category', ...derived.series.map(series => series.name)],
      ...derived.labels.map((label, index) => [label, ...derived.series.map(series => series.data[index] ?? 0)]),
    ]
  }

  if (Array.isArray(option.dataset)) {
    const first = option.dataset[0] && typeof option.dataset[0] === 'object' ? option.dataset[0] : {}
    option.dataset = [{ ...first, source: datasetSource }, ...option.dataset.slice(1)]
  } else {
    const dataset = option.dataset && typeof option.dataset === 'object' ? option.dataset : {}
    option.dataset = { ...dataset, source: datasetSource }
  }
}

function validateEChartsValue(value, depth = 0, state = { nodes: 0 }) {
  state.nodes += 1
  if (state.nodes > 20_000) throw new Error('ECharts option is too complex')
  if (depth > 20) throw new Error('ECharts option nesting is too deep')
  if (typeof value === 'string') {
    if (value.length > 10_000) throw new Error('ECharts option contains an excessively long string')
    const normalized = value.trim().toLowerCase()
    const privateAsset = normalized.startsWith('image:///api/decks/')
    if (/^(?:https?:|data:|javascript:|file:|\/\/)/.test(normalized) || (normalized.startsWith('image://') && !privateAsset))
      throw new Error('ECharts options cannot load external resources')
    return
  }
  if (Array.isArray(value)) {
    if (value.length > 5_000) throw new Error('ECharts option arrays may contain at most 5,000 items')
    value.forEach(item => validateEChartsValue(item, depth + 1, state))
    return
  }
  if (!value || typeof value !== 'object') return
  for (const [key, child] of Object.entries(value)) {
    if (BLOCKED_JSON_KEYS.has(key)) throw new Error(`ECharts option key \`${key}\` is not allowed`)
    validateEChartsValue(child, depth + 1, state)
  }
}

function validateLiveEChartsOption(option) {
  if (!Array.isArray(option.series) || !option.series.length)
    throw new Error('A live ECharts option needs at least one `series` entry')
  if (option.series.some(series => series && typeof series === 'object' && Object.hasOwn(series, 'data')))
    throw new Error('A live ECharts series must omit `data`; Interdeck supplies it through the dataset')
  if (Array.isArray(option.dataset))
    throw new Error('A live ECharts option supports one injected dataset object')
  if (option.dataset && typeof option.dataset === 'object' && (Object.hasOwn(option.dataset, 'source') || Object.hasOwn(option.dataset, 'transform')))
    throw new Error('A live ECharts dataset must omit `source` and `transform`; Interdeck supplies its source')
}

function applySeriesReveal(option, spec, visibleSeries) {
  if (spec.reveal !== 'series' || !Array.isArray(option.series) || !option.series.length) return
  const visibleCount = Math.max(1, Math.min(option.series.length, Number.isFinite(visibleSeries) ? Math.floor(visibleSeries) : option.series.length))
  const visible = option.series.slice(0, visibleCount).map((series, index) => ({
    ...series,
    id: series?.id || `interdeck-series-${index}`,
  }))
  option.series = visible

  const names = new Set(visible.map(series => series?.name).filter(Boolean))
  if (!names.size) return
  const legends = Array.isArray(option.legend) ? option.legend : [option.legend]
  for (const legend of legends) {
    if (!legend || !Array.isArray(legend.data)) continue
    legend.data = legend.data.filter(item => names.has(typeof item === 'string' ? item : item?.name))
  }
}

function normalizeLegend(value) {
  const normalized = String(value || 'top').toLowerCase()
  if (normalized === 'true' || normalized === 'top') return 'top'
  if (normalized === 'false' || normalized === 'bottom') return normalized
  throw new Error('Chart `legend` must be `top`, `bottom`, or `false`')
}

function booleanAttribute(attributes, key, fallback) {
  if (attributes[key] == null || attributes[key] === '') return fallback
  if (attributes[key] === 'true') return true
  if (attributes[key] === 'false') return false
  throw new Error(`Chart \`${key}\` must be \`true\` or \`false\``)
}

function parseColors(value) {
  if (!value) return []
  const colors = String(value).split('|').map(color => color.trim()).filter(Boolean)
  if (colors.length > 8 || colors.some(color => !/^#(?:[0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(color)))
    throw new Error('Chart `colors` must contain at most eight pipe-separated hex colors')
  return colors
}

function clampNumber(value, minimum, maximum, fallback) {
  const number = Number(value)
  return Number.isFinite(number) ? Math.min(maximum, Math.max(minimum, number)) : fallback
}

function strictNumber(value, minimum, maximum, fallback, label) {
  if (value == null || value === '') return fallback
  const number = Number(value)
  if (!Number.isFinite(number) || number < minimum || number > maximum)
    throw new Error(`${label} must be between ${minimum} and ${maximum}`)
  return number
}

function unique(values) {
  return [...new Set(values)]
}
