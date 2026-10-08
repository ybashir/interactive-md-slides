import assert from 'node:assert/strict'
import test from 'node:test'

import { buildSlidevExportUrl, readConfiguredTheme, repairUnsafeHeadmatterTitle, setConfiguredTheme } from './slidev-config.mjs'

test('reads quoted, full-package, commented, and whitespace-padded themes', () => {
  assert.equal(readConfiguredTheme('# Deck'), 'default')
  assert.equal(readConfiguredTheme('---\ntheme: seriph \n---\n# Deck'), 'seriph')
  assert.equal(readConfiguredTheme('---\ntheme: "@slidev/theme-apple-basic" # approved\n---\n# Deck'), '@slidev/theme-apple-basic')
})

test('sets a theme without discarding other headmatter', () => {
  const updated = setConfiguredTheme('---\ntitle: Demo\ntheme: default\n---\n\n# Deck', 'bricks')
  assert.match(updated, /^---\ntitle: Demo\ntheme: bricks\n---/)
  assert.match(updated, /# Deck$/)

  const inserted = setConfiguredTheme('# Deck', 'shibainu')
  assert.match(inserted, /^---\ntheme: shibainu\n---\n\n# Deck$/)
})

test('quotes a YAML-significant generated deck title without changing the body', () => {
  const source = '---\ntheme: default\ntitle: The Climate Challenge: Crisis & Action\n---\n\n# Climate\n'
  const repaired = repairUnsafeHeadmatterTitle(source)

  assert.equal(
    repaired,
    '---\ntheme: default\ntitle: "The Climate Challenge: Crisis & Action"\n---\n\n# Climate\n',
  )
})

test('repairs a partially quoted generated title and restores key spacing', () => {
  const source = '---\ntheme: default\ntitle:"\\"The Climate Challenge: Navigating Global\\" Crisis & Action"\n---\n\n# Climate\n'
  const repaired = repairUnsafeHeadmatterTitle(source)

  assert.equal(
    repaired,
    '---\ntheme: default\ntitle: "The Climate Challenge: Navigating Global Crisis & Action"\n---\n\n# Climate\n',
  )
})

test('builds an exact-revision Slidev browser export URL without dropping access', () => {
  assert.equal(
    buildSlidevExportUrl('/slidev/preview/deck-id/?access=signed-token', 'https://slides.example.com/editor'),
    'https://slides.example.com/slidev/preview/deck-id/export?access=signed-token',
  )
})
