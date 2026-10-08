export interface LocalDraft {
  version: 1
  deckId: string
  title: string
  markdown: string
  css: string
  baseVersion: number
  savedAt: string
}

interface DraftStorage {
  getItem(key: string): string | null
  setItem(key: string, value: string): void
  removeItem(key: string): void
}

export function draftKey(deckId: string): string
export function readLocalDraft(storage: DraftStorage, deckId: string): LocalDraft | null
export function writeLocalDraft(storage: DraftStorage, draft: Omit<LocalDraft, 'version' | 'savedAt'>): boolean
export function clearLocalDraft(storage: DraftStorage, deckId: string): void
