const SLIDE_MARKER = /^<!--\s*interdeck-slide:\s*([a-z0-9][a-z0-9_-]*)\s*-->[ \t]*(?:\r?\n|$)/gmi
const DELIMITER = /^---[ \t]*$/gm

function markerMatches(source) {
  return Array.from(source.matchAll(SLIDE_MARKER), match => ({
    key: match[1],
    start: match.index,
    contentStart: (match.index || 0) + match[0].length,
  }))
}

function resemblesFrontmatter(source) {
  const lines = source.split(/\r?\n/).filter(line => line.trim() && !line.trim().startsWith('#'))
  if (!lines.length) return false
  return lines.every(line => (
    /^\s/.test(line)
    || /^[A-Za-z_][\w-]*\s*:/.test(line)
    || /^\s*-\s+/.test(line)
    || /^\s*[}\]]\s*,?\s*$/.test(line)
  ))
}

function boundaryDetailsBeforeMarker(source, rangeStart, markerStart) {
  const region = source.slice(rangeStart, markerStart)
  const delimiters = Array.from(region.matchAll(DELIMITER), match => ({
    start: rangeStart + (match.index || 0),
    end: rangeStart + (match.index || 0) + match[0].length,
  }))
  const closing = delimiters.at(-1)
  if (!closing || source.slice(closing.end, markerStart).trim()) return null

  const opening = delimiters.at(-2)
  if (opening && resemblesFrontmatter(source.slice(opening.end, closing.start))) {
    return {
      boundaryStart: opening.start,
      frontmatterStart: opening.start,
      frontmatterEnd: closing.end,
    }
  }
  return { boundaryStart: closing.start, frontmatterStart: null, frontmatterEnd: null }
}

function boundaryBeforeMarker(source, rangeStart, markerStart) {
  return boundaryDetailsBeforeMarker(source, rangeStart, markerStart)?.boundaryStart ?? markerStart
}

function splitFocusedSource(value) {
  const delimiters = Array.from(String(value).matchAll(DELIMITER), match => ({
    start: match.index || 0,
    end: (match.index || 0) + match[0].length,
  }))
  const opening = delimiters[0]
  const closing = delimiters[1]
  if (!opening || opening.start !== 0 || !closing || !resemblesFrontmatter(value.slice(opening.end, closing.start)))
    return { frontmatter: '', content: value }
  return {
    frontmatter: value.slice(0, closing.end),
    content: value.slice(closing.end).replace(/^(?:[ \t]*\r?\n){1,2}/, ''),
  }
}

/**
 * Return a safe focused representation of a stable Interdeck slide. Deck
 * headmatter, slide markers, and adjacent boundaries remain protected. For
 * slides after the title slide, per-slide frontmatter is included so layout,
 * class, background, and other Slidev properties remain editable.
 */
export function findCurrentSlideContent(source, slideNumber) {
  const markers = markerMatches(source)
  const index = Math.max(0, Math.min(markers.length - 1, Number(slideNumber || 1) - 1))
  const marker = markers[index]
  if (!marker) return null

  const next = markers[index + 1]
  const end = next
    ? boundaryBeforeMarker(source, marker.contentStart, next.start)
    : source.length
  const boundary = index > 0
    ? boundaryDetailsBeforeMarker(source, markers[index - 1].contentStart, marker.start)
    : null
  const frontmatter = boundary?.frontmatterStart !== null && boundary?.frontmatterStart !== undefined
    ? source.slice(boundary.frontmatterStart, boundary.frontmatterEnd)
    : ''
  const content = source.slice(marker.contentStart, end)

  return {
    slideNumber: index + 1,
    slideKey: marker.key,
    start: frontmatter ? boundary.frontmatterStart : marker.contentStart,
    end,
    markerStart: marker.start,
    contentStart: marker.contentStart,
    boundaryStart: boundary?.boundaryStart ?? null,
    frontmatterStart: boundary?.frontmatterStart ?? null,
    frontmatterEnd: boundary?.frontmatterEnd ?? null,
    value: frontmatter ? `${frontmatter}\n\n${content}` : content,
  }
}

export function findSlideNumberByKey(source, slideKey) {
  if (!slideKey) return null
  const index = markerMatches(source).findIndex(marker => marker.key === slideKey)
  return index >= 0 ? index + 1 : null
}

export function findProposalSlide(source, proposedSource, preferredKey, currentSlideNumber) {
  const before = markerMatches(source)
  const after = markerMatches(proposedSource)
  const existingKeys = new Set(before.map(slide => slide.key))
  const added = after.filter(slide => !existingKeys.has(slide.key))
  const currentKey = before[Number(currentSlideNumber || 1) - 1]?.key
  const focus = added.find(slide => slide.key === preferredKey) || added[0]
    || after.find(slide => slide.key === preferredKey)
    || after.find(slide => slide.key === currentKey)
    || after[Math.max(0, Math.min(after.length - 1, Number(currentSlideNumber || 1) - 1))]
  return focus ? { slideKey: focus.key, slideNumber: after.indexOf(focus) + 1 } : null
}

export function replaceCurrentSlideContent(source, slideNumber, value) {
  const slice = findCurrentSlideContent(source, slideNumber)
  if (!slice) return source
  if (slice.slideNumber === 1 || slice.boundaryStart === null)
    return `${source.slice(0, slice.contentStart)}${value}${source.slice(slice.end)}`

  const focused = splitFocusedSource(value)
  const marker = source.slice(slice.markerStart, slice.contentStart)
  const boundary = focused.frontmatter || '---'
  return `${source.slice(0, slice.boundaryStart)}${boundary}\n\n${marker}${focused.content}${source.slice(slice.end)}`
}
