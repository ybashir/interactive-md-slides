import assert from 'node:assert/strict'
import test from 'node:test'

import { clearLocalDraft, draftKey, readLocalDraft, writeLocalDraft } from './local-draft.mjs'

function fakeStorage() {
  const values = new Map()
  return {
    getItem: key => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: key => values.delete(key),
  }
}

test('stores, reads, and clears a versioned deck-local recovery draft', () => {
  const storage = fakeStorage()
  assert.equal(writeLocalDraft(storage, {
    deckId: 'deck-1', title: 'Recovered title', markdown: '# Unsaved', css: '.slidev-layout { color: red; }', baseVersion: 4,
  }), true)

  const draft = readLocalDraft(storage, 'deck-1')
  assert.equal(draft?.title, 'Recovered title')
  assert.equal(draft?.markdown, '# Unsaved')
  assert.equal(draft?.css, '.slidev-layout { color: red; }')
  assert.equal(draft?.baseVersion, 4)
  assert.equal(draft?.version, 1)
  assert.ok(Number.isFinite(Date.parse(draft?.savedAt || '')))

  clearLocalDraft(storage, 'deck-1')
  assert.equal(readLocalDraft(storage, 'deck-1'), null)
})

test('does not expose corrupt, cross-deck, or excessively large drafts', () => {
  const storage = fakeStorage()
  storage.setItem(draftKey('deck-1'), '{broken')
  assert.equal(readLocalDraft(storage, 'deck-1'), null)

  storage.setItem(draftKey('deck-1'), JSON.stringify({
    version: 1, deckId: 'deck-2', title: '', markdown: '# Wrong deck', baseRevision: 1, savedAt: new Date().toISOString(),
  }))
  assert.equal(readLocalDraft(storage, 'deck-1'), null)

  assert.equal(writeLocalDraft(storage, {
    deckId: 'deck-1', title: '', markdown: 'x'.repeat(2_000_001), baseVersion: 1,
  }), false)
})

test('reads recovery drafts created before the CSS editor was added', () => {
  const storage = fakeStorage()
  storage.setItem(draftKey('deck-1'), JSON.stringify({
    version: 1, deckId: 'deck-1', title: '', markdown: '# Existing', baseVersion: 2, savedAt: new Date().toISOString(),
  }))
  assert.equal(readLocalDraft(storage, 'deck-1')?.css, '')
})

test('storage failures never escape into the editing flow', () => {
  const blocked = {
    getItem() { throw new Error('blocked') },
    setItem() { throw new Error('blocked') },
    removeItem() { throw new Error('blocked') },
  }
  assert.equal(readLocalDraft(blocked, 'deck-1'), null)
  assert.equal(writeLocalDraft(blocked, {
    deckId: 'deck-1', title: '', markdown: '# Draft', baseVersion: 1,
  }), false)
  assert.doesNotThrow(() => clearLocalDraft(blocked, 'deck-1'))
})
