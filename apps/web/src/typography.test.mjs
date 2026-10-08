import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const styles = await readFile(new URL('./styles.css', import.meta.url), 'utf8')

test('keeps application typography above the compact readability floor', () => {
  const remSizes = [...styles.matchAll(/font-size:\s*(\.\d+)rem/g)]
    .map(match => Number(match[1]))

  assert.ok(remSizes.length > 100, 'expected to audit the application typography rules')
  assert.equal(remSizes.filter(size => size < 0.75).length, 0)
})

test('gives the slide assistant body-sized text and sufficient working space', () => {
  assert.match(styles, /--font-interface:\s*\.875rem/)
  assert.match(styles, /\.assistant-turn[^}]*font-size:\s*var\(--font-interface\)/s)
  assert.match(styles, /\.assistant-compose textarea[^}]*font-size:\s*var\(--font-interface\)/s)
  assert.match(styles, /\.slide-assistant[^}]*height:\s*min\(390px, 48vh\)/s)
})

test('keeps secondary and cancel actions readable on every dark application surface', () => {
  assert.match(styles, /\.button-ghost[^}]*color:\s*var\(--button-ghost-color, var\(--ink\)\)/s)
  for (const surface of ['assistant-proposal', 'present-page']) {
    const rule = styles.match(new RegExp(`\\.${surface} \\{([^}]*)\\}`))?.[1] || ''
    assert.match(rule, /--button-ghost-color:\s*#f5f7fb/)
    assert.match(rule, /--button-ghost-border:\s*#[a-fA-F0-9]{6}/)
    assert.match(rule, /--button-ghost-hover-background:\s*#[a-fA-F0-9]{8}/)
  }
})
