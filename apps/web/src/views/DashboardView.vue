<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { ApiError, api, post } from '../api'
import { clearLocalDraft } from '../local-draft.mjs'
import type { DeckDetail, DeckSummary, User } from '../types'

const router = useRouter()
const user = ref<User | null>(null)
const decks = ref<DeckSummary[]>([])
const title = ref('')
const fileInput = ref<HTMLInputElement | null>(null)
const loading = ref(true)
const creating = ref(false)
const importing = ref(false)
const creatingSample = ref(false)
const deletingDeckId = ref<string | null>(null)
const endingDeckId = ref<string | null>(null)
const error = ref('')

onMounted(load)

async function load() {
  try {
    const [me, deckList] = await Promise.all([
      api<{ user: User }>('/api/auth/me'),
      api<DeckSummary[]>('/api/decks'),
    ])
    user.value = me.user
    decks.value = deckList
  }
  catch (reason) {
    if (reason instanceof ApiError && reason.status === 401) {
      router.replace('/login')
      return
    }
    error.value = reason instanceof Error ? reason.message : 'Could not load your decks'
  }
  finally {
    loading.value = false
  }
}

async function createDeck() {
  if (!title.value.trim() || creating.value) return
  creating.value = true
  error.value = ''
  try {
    const deck = await post<DeckDetail>('/api/decks', { title: title.value })
    await router.push(`/decks/${deck.id}`)
  }
  catch (reason) {
    error.value = reason instanceof Error ? reason.message : 'Could not create the deck'
  }
  finally {
    creating.value = false
  }
}

async function importDeck(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file || importing.value) return
  importing.value = true
  error.value = ''
  try {
    if (file.size > 2_000_000) throw new Error('The Markdown file must be smaller than 2 MB')
    const markdown = await file.text()
    const deckTitle = inferDeckTitle(markdown, file.name)
    const deck = await post<DeckDetail>('/api/decks', { title: deckTitle, markdown })
    await router.push(`/decks/${deck.id}`)
  }
  catch (reason) {
    error.value = reason instanceof Error ? reason.message : 'Could not import the deck'
  }
  finally {
    importing.value = false
    input.value = ''
  }
}

async function createSeaCreaturesSample() {
  if (creatingSample.value) return
  creatingSample.value = true
  error.value = ''
  try {
    const deck = await post<DeckDetail>('/api/decks', {
      title: 'Amazing Sea Creatures',
      sample: 'amazing-sea-creatures',
    })
    await router.push(`/decks/${deck.id}`)
  }
  catch (reason) {
    error.value = reason instanceof Error ? reason.message : 'Could not create the sample deck'
  }
  finally {
    creatingSample.value = false
  }
}

async function deleteDeck(deck: DeckSummary) {
  if (deck.is_live || deletingDeckId.value) return
  const confirmed = window.confirm(
    `Permanently delete “${deck.title}”?\n\nIts slides, assets, audience responses, and Q&A history will be removed. This cannot be undone.`,
  )
  if (!confirmed) return
  deletingDeckId.value = deck.id
  error.value = ''
  try {
    await api(`/api/decks/${deck.id}`, { method: 'DELETE' })
    clearLocalDraft(window.localStorage, deck.id)
    decks.value = decks.value.filter(item => item.id !== deck.id)
  }
  catch (reason) {
    error.value = reason instanceof Error ? reason.message : 'Could not delete the deck'
  }
  finally {
    deletingDeckId.value = null
  }
}

async function endLiveDeck(deck: DeckSummary) {
  if (!deck.is_live || endingDeckId.value) return
  if (!window.confirm(`End the live presentation “${deck.title}”?`)) return
  endingDeckId.value = deck.id
  error.value = ''
  try {
    await post(`/api/decks/${deck.id}/stop`)
    decks.value = decks.value.map(item => item.id === deck.id ? { ...item, is_live: false } : item)
  }
  catch (reason) {
    error.value = reason instanceof Error ? reason.message : 'Could not end the presentation'
  }
  finally {
    endingDeckId.value = null
  }
}

function inferDeckTitle(markdown: string, filename: string) {
  const configured = markdown.match(/^title:\s*["']?(.+?)["']?\s*$/m)?.[1]
  const heading = markdown.match(/^#\s+(.+)$/m)?.[1]
  return (configured || heading || filename.replace(/\.md$/i, '') || 'Imported deck').slice(0, 120)
}

async function logout() {
  await post('/api/auth/logout')
  await router.push('/login')
}
</script>

<template>
  <main class="dashboard-page">
    <header class="topbar">
      <RouterLink to="/" class="wordmark"><span class="wordmark-icon">I</span> interdeck</RouterLink>
      <div class="dashboard-nav-actions">
        <a class="help-entry-link" href="/help" target="_blank" rel="noopener noreferrer">Help <span aria-hidden="true">↗</span></a>
        <div v-if="user" class="user-menu">
          <img v-if="user.picture_url" :src="user.picture_url" alt="" referrerpolicy="no-referrer">
          <span><strong>{{ user.display_name }}</strong><small>{{ user.email }}</small></span>
          <button class="icon-button" title="Sign out" @click="logout">↗</button>
        </div>
      </div>
    </header>

    <section class="dashboard-content">
      <form class="new-deck-card dashboard-new-deck" @submit.prevent="createDeck">
        <span class="new-deck-plus">+</span>
        <label for="deck-title">Start a new deck</label>
        <div class="new-deck-input">
          <input id="deck-title" v-model="title" maxlength="120" placeholder="e.g. Engineering All Hands" autocomplete="off">
          <button class="button button-primary" :disabled="creating || !title.trim()">{{ creating ? 'Creating…' : 'Create' }}</button>
        </div>
        <small>Slidev Markdown · Live audience built in</small>
        <div class="new-deck-secondary">
          <input ref="fileInput" type="file" accept=".md,text/markdown,text/plain" @change="importDeck">
          <button type="button" :disabled="importing" @click="fileInput?.click()">{{ importing ? 'Importing…' : 'Import Slidev .md' }}</button>
          <span>or</span>
          <button type="button" :disabled="creatingSample" @click="createSeaCreaturesSample">{{ creatingSample ? 'Preparing…' : 'Use Amazing Sea Creatures sample' }}</button>
        </div>
      </form>

      <div v-if="error" class="notice notice-error">{{ error }}</div>
      <div class="section-heading"><h2>Your decks</h2><span>{{ decks.length }} total</span></div>
      <div v-if="loading" class="loading-grid"><i v-for="item in 3" :key="item" /></div>
      <div v-else-if="decks.length" class="deck-grid">
        <article v-for="deck in decks" :key="deck.id" class="deck-card" @click="router.push(`/decks/${deck.id}`)">
          <div class="deck-thumbnail">
            <span v-if="deck.is_live" class="live-pill"><i /> LIVE</span>
            <div class="mini-slide"><small>INTERDECK</small><strong>{{ deck.title }}</strong><span>Join · {{ deck.join_code }}</span></div>
          </div>
          <div class="deck-meta">
            <div><h3>{{ deck.title }}</h3><p>Edited {{ new Date(deck.updated_at).toLocaleDateString() }}</p></div>
            <div class="deck-card-actions">
              <button
                v-if="deck.is_live"
                class="deck-end-live"
                :disabled="endingDeckId === deck.id"
                :aria-label="`End live presentation ${deck.title}`"
                @click.stop="endLiveDeck(deck)"
              >{{ endingDeckId === deck.id ? 'Ending…' : 'End live' }}</button>
              <button
                v-else
                class="deck-delete"
                :disabled="deletingDeckId === deck.id"
                :title="`Delete ${deck.title}`"
                :aria-label="`Delete ${deck.title}`"
                @click.stop="deleteDeck(deck)"
              >{{ deletingDeckId === deck.id ? 'Deleting…' : 'Delete' }}</button>
              <button class="icon-button" aria-label="Open deck">→</button>
            </div>
          </div>
        </article>
      </div>
      <div v-else class="empty-state">
        <div class="empty-orb">✦</div>
        <h2>Your first room is waiting.</h2>
        <p>Name a deck above. We will start it with a title slide and a concise guide to editing, interactions, previewing, and presenting.</p>
      </div>
    </section>
  </main>
</template>
