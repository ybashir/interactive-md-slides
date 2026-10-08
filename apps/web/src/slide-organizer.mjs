import { findCurrentSlideContent } from './current-slide-source.mjs'

const MARKER = /<!--\s*interdeck-slide:\s*([a-z0-9][a-z0-9_-]*)\s*-->/i
const ATTRIBUTE_ID = /\bid\s*=\s*(["'])([^"']+)\1/gi

function uniqueCopyName(base, used) {
  let candidate = `${base}-copy`
  let suffix = 2
  while (used.has(candidate)) candidate = `${base}-copy-${suffix++}`
  used.add(candidate)
  return candidate
}

function titleFromSource(source, fallback) {
  const heading = source.match(/^\s*#{1,3}\s+(.+?)\s*$/m)?.[1]
  return heading?.replace(/\s+\{[^}]*\}\s*$/, '').trim() || fallback
}

export function listOrganizedSlides(source, parsedSlides = []) {
  const parsedTitles = new Map(parsedSlides.map(slide => [slide.key, slide.title]))
  const slides = []
  for (let slideNumber = 1; ; slideNumber += 1) {
    const slice = findCurrentSlideContent(source, slideNumber)
    if (!slice || slice.slideNumber !== slideNumber) break
    const blockStart = slideNumber === 1 ? slice.markerStart : slice.boundaryStart
    if (blockStart === null) break
    const block = source.slice(blockStart, slice.end)
    slides.push({
      key: slice.slideKey,
      number: slideNumber,
      title: titleFromSource(block, parsedTitles.get(slice.slideKey) || `Slide ${slideNumber}`),
      blockStart,
      blockEnd: slice.end,
      interactionCount: Array.from(block.matchAll(ATTRIBUTE_ID)).length,
      fixed: slideNumber === 1,
    })
  }
  return slides
}

export function reorderOrganizedSlides(source, orderedKeys) {
  const slides = listOrganizedSlides(source)
  if (slides.length < 2) return source
  const movable = slides.slice(1)
  if (orderedKeys.length !== movable.length) return source
  const byKey = new Map(movable.map(slide => [slide.key, source.slice(slide.blockStart, slide.blockEnd)]))
  if (new Set(orderedKeys).size !== movable.length || orderedKeys.some(key => !byKey.has(key))) return source
  return `${source.slice(0, movable[0].blockStart)}${orderedKeys.map(key => byKey.get(key)).join('')}`
}

export function deleteOrganizedSlide(source, slideKey) {
  const slide = listOrganizedSlides(source).find(item => item.key === slideKey)
  if (!slide || slide.fixed) return source
  return `${source.slice(0, slide.blockStart)}${source.slice(slide.blockEnd)}`
}

export function duplicateOrganizedSlide(source, slideKey) {
  const slides = listOrganizedSlides(source)
  const slide = slides.find(item => item.key === slideKey)
  if (!slide || slide.fixed) return { source, slideKey: null }

  const usedSlideKeys = new Set(slides.map(item => item.key))
  const newSlideKey = uniqueCopyName(slide.key, usedSlideKeys)
  const usedIds = new Set(Array.from(source.matchAll(ATTRIBUTE_ID), match => match[2]))
  const idCopies = new Map()
  let block = source.slice(slide.blockStart, slide.blockEnd)
  block = block.replace(MARKER, `<!-- interdeck-slide: ${newSlideKey} -->`)
  block = block.replace(ATTRIBUTE_ID, (_match, quote, id) => {
    const next = idCopies.get(id) || uniqueCopyName(id, usedIds)
    idCopies.set(id, next)
    return `id=${quote}${next}${quote}`
  })
  for (const [oldId, newId] of idCopies) {
    const sourceAttribute = new RegExp(`\\bsource\\s*=\\s*(["'])${escapeRegExp(oldId)}\\1`, 'gi')
    block = block.replace(sourceAttribute, (_match, quote) => `source=${quote}${newId}${quote}`)
  }
  const separator = source.slice(slide.blockEnd).startsWith('\n') || block.endsWith('\n') ? '' : '\n'
  const nextSource = `${source.slice(0, slide.blockEnd)}${separator}${block}${source.slice(slide.blockEnd)}`
  return { source: nextSource, slideKey: newSlideKey }
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}
