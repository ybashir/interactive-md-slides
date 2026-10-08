import assert from 'node:assert/strict'
import test from 'node:test'

import { SlidevCompatibilityError, slidevCatalog, validateSlidevCompatibility } from './slidev-catalog.mjs'

test('catalog includes every official Slidev theme', () => {
  assert.deepEqual(
    slidevCatalog.themes.map(theme => theme.id),
    ['default', 'seriph', 'apple-basic', 'bricks', 'shibainu'],
  )
})

test('rejects filesystem snippets and imported slides before a worker is started', () => {
  for (const source of ['# Deck\n<<< ../../../.env', '---\nsrc: ../other-deck/slides.md\n---', '# Deck\n> <<< /etc/passwd']) {
    assert.throws(() => validateSlidevCompatibility(source), error => error.code === 'local_file_import');
  }
})

test('accepts short and full names for approved themes', () => {
  assert.deepEqual(validateSlidevCompatibility('# Deck'), { theme: 'default', addons: [] })
  assert.deepEqual(
    validateSlidevCompatibility('---\ntheme: seriph\n---\n\n# Deck'),
    { theme: 'seriph', addons: [] },
  )
  assert.deepEqual(
    validateSlidevCompatibility('---\ntheme: "@slidev/theme-apple-basic"\n---\n\n# Deck'),
    { theme: 'apple-basic', addons: [] },
  )
})

test('rejects unreviewed themes and addons with safe diagnostics', () => {
  assert.throws(
    () => validateSlidevCompatibility('---\ntheme: community-theme\n---\n# Deck'),
    error => error instanceof SlidevCompatibilityError
      && error.code === 'unsupported_theme'
      && error.publicMessage.includes('community-theme'),
  )
  assert.throws(
    () => validateSlidevCompatibility('---\naddons:\n  - excalidraw\n---\n# Deck'),
    error => error instanceof SlidevCompatibilityError
      && error.code === 'unsupported_addon'
      && error.publicMessage.includes('reviewed and preinstalled'),
  )
})

test('reports malformed YAML without exposing parser internals', () => {
  assert.throws(
    () => validateSlidevCompatibility('---\ntheme: [seriph\n---\n# Deck'),
    error => error instanceof SlidevCompatibilityError
      && error.code === 'invalid_headmatter'
      && error.publicMessage === 'The deck headmatter is not valid YAML. Fix the configuration between the opening --- markers.',
  )
})

test('a different fence marker cannot hide a file import after the real closing fence', () => {
  assert.throws(() => validateSlidevCompatibility('```\n~~~\n```\n<<< /private/file.txt'), error => error.code === 'local_file_import')
  assert.doesNotThrow(() => validateSlidevCompatibility('````\n```\n<<< example.txt\n```\n````'))
})
