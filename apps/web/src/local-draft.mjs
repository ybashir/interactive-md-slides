const PREFIX = 'interdeck:deck-draft:v1:'
const MAX_MARKDOWN_LENGTH = 2_000_000

export function draftKey(deckId) {
  return `${PREFIX}${deckId}`
}

export function readLocalDraft(storage, deckId) {
  try {
    const raw = storage.getItem(draftKey(deckId))
    if (!raw) return null
    const value = JSON.parse(raw)
    if (
      value?.version !== 1
      || value.deckId !== deckId
      || typeof value.title !== 'string'
      || typeof value.markdown !== 'string'
      || value.markdown.length > MAX_MARKDOWN_LENGTH
      || (value.css !== undefined && typeof value.css !== 'string')
      || !Number.isInteger(value.baseVersion ?? value.baseRevision)
      || typeof value.savedAt !== 'string'
      || !Number.isFinite(Date.parse(value.savedAt))
    ) return null
    return { ...value, css: value.css || '', baseVersion: value.baseVersion ?? value.baseRevision }
  }
  catch {
    return null
  }
}

export function writeLocalDraft(storage, { deckId, title, markdown, css = '', baseVersion }) {
  if (markdown.length > MAX_MARKDOWN_LENGTH) return false
  try {
    storage.setItem(draftKey(deckId), JSON.stringify({
      version: 1,
      deckId,
      title,
      markdown,
      css,
      baseVersion,
      savedAt: new Date().toISOString(),
    }))
    return true
  }
  catch {
    return false
  }
}

export function clearLocalDraft(storage, deckId) {
  try {
    storage.removeItem(draftKey(deckId))
  }
  catch {
    // A blocked storage API must never interfere with server persistence.
  }
}
