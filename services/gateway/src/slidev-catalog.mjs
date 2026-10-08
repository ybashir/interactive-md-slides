import { parse } from 'yaml'
import { parseSync } from '@slidev/parser'

export const slidevCatalog = Object.freeze({
  slidevVersion: '52.19.1',
  themes: Object.freeze([
    theme('default', 'Default', 'Minimal, versatile Slidev styling.', '@slidev/theme-default'),
    theme('seriph', 'Seriph', 'Formal typography with a serif-led visual system.', '@slidev/theme-seriph'),
    theme('apple-basic', 'Apple Basic', 'Clean black-and-white Keynote-inspired styling.', '@slidev/theme-apple-basic'),
    theme('bricks', 'Bricks', 'Playful, geometric building-block styling.', '@slidev/theme-bricks'),
    theme('shibainu', 'Shibainu', 'Friendly illustrated styling from the official gallery.', '@slidev/theme-shibainu'),
  ]),
  addons: Object.freeze([]),
  coreFeatures: Object.freeze([
    'Markdown, safe HTML, deck CSS, and private assets',
    'Common built-in layouts and per-slide frontmatter',
    'Fade, directional transitions, v-click, and v-clicks',
    'All Interdeck slide-level and element-level interactions',
    'Always-on multi-deck preview and presentation player',
    'Full upstream Slidev runtime for explicit creator export',
  ]),
})

export class SlidevCompatibilityError extends Error {
  constructor(code, publicMessage, details = '') {
    super(details || publicMessage)
    this.name = 'SlidevCompatibilityError'
    this.code = code
    this.publicMessage = publicMessage
    this.statusCode = 422
  }
}

export function validateSlidevCompatibility(markdown) {
  // Uploaded decks have no accompanying source files. File-backed snippets and
  // imported decks would otherwise let a creator ask the exporter to read its
  // filesystem, including another creator's generated deck.
  const parsed = parseSync(String(markdown), 'slides.md')
  let fence = null
  const fileSnippet = String(markdown).split(/\r?\n/).some(line => {
    const marker = line.match(/^[\t >]*(`{3,}|~{3,})(.*)$/)
    if (marker) {
      if (!fence) fence = { character: marker[1][0], length: marker[1].length }
      else if (marker[1][0] === fence.character && marker[1].length >= fence.length && !marker[2].trim()) fence = null
      return false
    }
    return !fence && /^[\t >]*<<</.test(line)
  })
  if (fileSnippet || parsed.headmatter?.src != null || parsed.slides.some(slide => slide.frontmatter?.src != null)) {
    throw new SlidevCompatibilityError('local_file_import',
      'Local file imports and snippets are not supported. Paste the content into the deck or upload an image.');
  }
  const headmatter = readHeadmatter(markdown)
  const configuredTheme = headmatter.theme ?? 'default'
  if (typeof configuredTheme !== 'string' || !configuredTheme.trim()) {
    throw new SlidevCompatibilityError(
      'invalid_theme',
      'The deck theme must be a name such as "default" or "seriph".',
    )
  }

  const selectedTheme = slidevCatalog.themes.find(item => item.aliases.includes(configuredTheme.trim()))
  if (!selectedTheme) {
    const supported = slidevCatalog.themes.map(item => item.id).join(', ')
    throw new SlidevCompatibilityError(
      'unsupported_theme',
      `Theme "${configuredTheme}" is not enabled on Interdeck. Choose one of: ${supported}.`,
    )
  }

  const addons = normalizeAddons(headmatter.addons)
  const unsupportedAddons = addons.filter(name => !slidevCatalog.addons.some(item => item.aliases.includes(name)))
  if (unsupportedAddons.length) {
    throw new SlidevCompatibilityError(
      'unsupported_addon',
      `Addon${unsupportedAddons.length === 1 ? '' : 's'} ${unsupportedAddons.map(name => `"${name}"`).join(', ')} cannot run on Interdeck yet. Addons execute code, so they must be reviewed and preinstalled first.`,
    )
  }

  return { theme: selectedTheme.id, addons }
}

function theme(id, label, description, packageName) {
  return Object.freeze({
    id,
    label,
    description,
    packageName,
    aliases: Object.freeze([id, packageName]),
  })
}

function readHeadmatter(markdown) {
  const match = String(markdown).match(/^---[\t ]*\r?\n([\s\S]*?)\r?\n---(?:[\t ]*\r?\n|$)/)
  if (!match) return {}
  try {
    const value = parse(match[1])
    if (value == null) return {}
    if (typeof value !== 'object' || Array.isArray(value))
      throw new Error('Headmatter must be a YAML object')
    if (Object.keys(value).some(key => !/^[a-zA-Z][a-zA-Z0-9_-]*$/.test(key)))
      throw new Error('Headmatter contains a malformed property name')
    return value
  }
  catch (error) {
    throw new SlidevCompatibilityError(
      'invalid_headmatter',
      'The deck headmatter is not valid YAML. Fix the configuration between the opening --- markers.',
      error instanceof Error ? error.message : String(error),
    )
  }
}

function normalizeAddons(value) {
  if (value == null) return []
  if (!Array.isArray(value) || value.some(item => typeof item !== 'string' || !item.trim())) {
    throw new SlidevCompatibilityError(
      'invalid_addons',
      'The addons setting must be a YAML list of addon names.',
    )
  }
  return value.map(item => item.trim())
}
