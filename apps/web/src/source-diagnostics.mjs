import { parseChartSpec, parseEChartsSpec } from './chart-spec.mjs'

const VALID_ID = /^[a-zA-Z0-9][a-zA-Z0-9_-]{1,63}$/

function attribute(source, name) {
  const match = source.match(new RegExp(`\\b${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)'|([^\\s}]+))`))
  return match ? (match[1] ?? match[2] ?? match[3]) : null
}

function marker(message, line, source, token = '') {
  const offset = token ? Math.max(0, source.indexOf(token)) : 0
  return {
    message,
    severity: 'error',
    startLine: line,
    startColumn: offset + 1,
    endLine: line,
    endColumn: offset + Math.max(1, token.length) + 1,
  }
}

export function diagnoseSource(markdown) {
  const lines = markdown.replace(/\r\n/g, '\n').split('\n')
  const diagnostics = []
  const identifiers = []

  lines.forEach((line, index) => {
    const lineNumber = index + 1
    const slide = line.match(/<!--\s*interdeck-slide:\s*([^\s>]+)\s*-->/)
    if (slide) identifiers.push({ scope: 'slide', id: slide[1], line: lineNumber, source: line })

    const block = line.match(/:::interact\{([^}]*)\}/)
    if (block) {
      const id = attribute(block[1], 'id')
      const type = attribute(block[1], 'type')
      if (!id) diagnostics.push(marker('Interaction is missing a stable `id` attribute.', lineNumber, line, ':::interact'))
      else identifiers.push({ scope: 'interaction', id, line: lineNumber, source: line })
      if (!type) diagnostics.push(marker('Interaction is missing a `type` attribute.', lineNumber, line, ':::interact'))
    }

    const inlineChart = line.match(/^::chart\{([^}]*)\}\s*$/)
    if (inlineChart) {
      try {
        parseChartSpec(inlineChart[1], '')
      } catch (error) {
        diagnostics.push(marker(error instanceof Error ? error.message : String(error), lineNumber, line, '::chart'))
      }
    }

    const element = !block && line.match(/\{([^}\n]*\binteract\s*=\s*[^}\n]+)\}/)
    if (element) {
      const id = attribute(element[1], 'id')
      if (!id) diagnostics.push(marker('Element interaction is missing a stable `id` attribute.', lineNumber, line, 'interact'))
      else identifiers.push({ scope: 'interaction', id, line: lineNumber, source: line })
    }
  })

  for (const chart of markdown.matchAll(/:::chart\{([^}]*)\}\s*([\s\S]*?)\s*:::/g)) {
    try {
      parseChartSpec(chart[1], chart[2])
    } catch (error) {
      const lineNumber = markdown.slice(0, chart.index).split('\n').length
      diagnostics.push(marker(
        error instanceof Error ? error.message : String(error),
        lineNumber,
        lines[lineNumber - 1] || ':::chart',
        ':::chart',
      ))
    }
  }

  for (const chart of markdown.matchAll(/:::echarts(?:\{([^}]*)\})?\s*([\s\S]*?)\s*:::/g)) {
    try {
      parseEChartsSpec(chart[1] || '', chart[2])
    } catch (error) {
      const lineNumber = markdown.slice(0, chart.index).split('\n').length
      diagnostics.push(marker(
        error instanceof Error ? error.message : String(error),
        lineNumber,
        lines[lineNumber - 1] || ':::echarts',
        ':::echarts',
      ))
    }
  }

  const grouped = new Map()
  for (const identifier of identifiers) {
    const key = `${identifier.scope}:${identifier.id}`
    grouped.set(key, [...(grouped.get(key) || []), identifier])
    if (!VALID_ID.test(identifier.id)) {
      diagnostics.push(marker(
        `${identifier.scope === 'slide' ? 'Slide' : 'Interaction'} ID \`${identifier.id}\` must be 2–64 letters, numbers, underscores, or hyphens.`,
        identifier.line,
        identifier.source,
        identifier.id,
      ))
    }
  }
  for (const occurrences of grouped.values()) {
    if (occurrences.length < 2) continue
    for (const occurrence of occurrences) {
      diagnostics.push(marker(
        `Duplicate ${occurrence.scope} ID \`${occurrence.id}\`. IDs must be unique within a deck.`,
        occurrence.line,
        occurrence.source,
        occurrence.id,
      ))
    }
  }

  return diagnostics.sort((left, right) => left.startLine - right.startLine || left.startColumn - right.startColumn)
}
