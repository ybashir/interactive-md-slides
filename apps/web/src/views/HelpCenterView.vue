<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'

import { commonInteractionOptions, helpArticles, interactionReferences } from '../help-content'

const query = ref('')
const copiedCode = ref('')
const searchInput = ref<HTMLInputElement | null>(null)
let previousDocumentTitle = ''

const normalizedQuery = computed(() => query.value.trim().toLowerCase())
const filteredArticles = computed(() => {
  if (!normalizedQuery.value) return helpArticles
  const terms = normalizedQuery.value.split(/\s+/).filter(Boolean)
  return helpArticles.filter((article) => {
    const interactionReferenceText = article.id === 'interaction-options'
      ? interactionReferences.flatMap(reference => [
          reference.name,
          reference.type,
          reference.syntax,
          reference.body,
          ...reference.options.flatMap(option => [option.name, option.values, option.defaultValue, option.description]),
        ])
      : []
    const haystack = [
      article.title,
      article.summary,
      ...article.keywords,
      ...article.blocks.flatMap(block => [block.text || '', block.code || '', ...(block.items || [])]),
      ...interactionReferenceText,
    ].join(' ').toLowerCase()
    return terms.every(term => haystack.includes(term))
  })
})

function clearSearch() {
  query.value = ''
  nextTick(() => searchInput.value?.focus())
}

async function copyCode(code: string) {
  await navigator.clipboard.writeText(code)
  copiedCode.value = code
  window.setTimeout(() => {
    if (copiedCode.value === code) copiedCode.value = ''
  }, 1_500)
}

function focusSearch(event: KeyboardEvent) {
  if (event.key !== '/' || event.metaKey || event.ctrlKey || event.altKey) return
  const target = event.target as HTMLElement
  if (['INPUT', 'TEXTAREA'].includes(target.tagName)) return
  event.preventDefault()
  searchInput.value?.focus()
}

onMounted(() => {
  previousDocumentTitle = document.title
  document.title = 'Help Center · Interdeck'
  window.addEventListener('keydown', focusSearch)
})
onBeforeUnmount(() => {
  document.title = previousDocumentTitle
  window.removeEventListener('keydown', focusSearch)
})
</script>

<template>
  <main id="top" class="help-page">
    <header class="help-topbar">
      <RouterLink to="/" class="wordmark"><span class="wordmark-icon">I</span> interdeck</RouterLink>
      <span>Help Center</span>
      <RouterLink to="/" class="help-back">Back to decks</RouterLink>
    </header>

    <section class="help-hero">
      <p class="eyebrow">INTERDECK GUIDE</p>
      <h1>Make the room<br><em>part of the story.</em></h1>
      <p>Practical guidance for authoring Slidev decks, adding live interactions, presenting confidently, and understanding the room.</p>
      <label class="help-search">
        <span aria-hidden="true">⌕</span>
        <input ref="searchInput" v-model="query" type="search" placeholder="Search interactions, charts, Q&A, styling…" aria-label="Search the Help Center">
        <kbd>/</kbd>
      </label>
    </section>

    <div class="help-layout">
      <aside class="help-contents" aria-label="Help topics">
        <strong>Contents</strong>
        <nav>
          <a v-for="article in helpArticles" :key="article.id" :href="`#${article.id}`">{{ article.title }}</a>
        </nav>
      </aside>

      <section class="help-results" aria-live="polite">
        <div class="help-results-heading">
          <span>{{ filteredArticles.length }} {{ filteredArticles.length === 1 ? 'topic' : 'topics' }}</span>
          <button v-if="query" type="button" @click="clearSearch">Clear search</button>
        </div>

        <article v-for="article in filteredArticles" :id="article.id" :key="article.id" class="help-article">
          <header>
            <h2>{{ article.title }}</h2>
            <p>{{ article.summary }}</p>
          </header>
          <template v-for="(block, index) in article.blocks" :key="index">
            <p v-if="block.type === 'paragraph'" class="help-paragraph">{{ block.text }}</p>
            <ul v-else-if="block.type === 'list'" class="help-list">
              <li v-for="item in block.items" :key="item">{{ item }}</li>
            </ul>
            <aside v-else-if="block.type === 'note'" class="help-note"><strong>Good to know</strong><span>{{ block.text }}</span></aside>
            <div v-else-if="block.type === 'code' && block.code" class="help-code">
              <div><span>{{ block.language || 'text' }}</span><button type="button" @click="copyCode(block.code)">{{ copiedCode === block.code ? 'Copied' : 'Copy' }}</button></div>
              <pre><code>{{ block.code }}</code></pre>
            </div>
          </template>
          <template v-if="article.id === 'interaction-options'">
            <section class="interaction-reference-common" aria-labelledby="common-interaction-options">
              <div class="interaction-reference-heading">
                <div>
                  <span class="interaction-reference-kind">Every slide-level interaction</span>
                  <h3 id="common-interaction-options">Common attributes</h3>
                </div>
              </div>
              <div class="help-option-table-wrap">
                <table class="help-option-table">
                  <thead><tr><th>Attribute</th><th>Accepted values</th><th>Default</th><th>Meaning</th></tr></thead>
                  <tbody>
                    <tr v-for="option in commonInteractionOptions" :key="option.name">
                      <th scope="row"><code>{{ option.name }}</code></th>
                      <td>{{ option.values }}</td>
                      <td><code>{{ option.defaultValue }}</code></td>
                      <td>{{ option.description }}</td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </section>

            <section v-for="reference in interactionReferences" :id="reference.id" :key="reference.id" class="interaction-reference-card">
              <div class="interaction-reference-heading">
                <div>
                  <span class="interaction-reference-kind">{{ reference.type }}</span>
                  <h3>{{ reference.name }}</h3>
                </div>
                <button type="button" @click="copyCode(reference.syntax)">{{ copiedCode === reference.syntax ? 'Copied' : 'Copy syntax' }}</button>
              </div>
              <code class="interaction-reference-syntax">{{ reference.syntax }}</code>
              <p>{{ reference.body }}</p>
              <div v-if="reference.options.length" class="help-option-table-wrap">
                <table class="help-option-table">
                  <thead><tr><th>Attribute</th><th>Accepted values</th><th>Default</th><th>Meaning</th></tr></thead>
                  <tbody>
                    <tr v-for="option in reference.options" :key="option.name">
                      <th scope="row"><code>{{ option.name }}</code></th>
                      <td>{{ option.values }}</td>
                      <td><code>{{ option.defaultValue }}</code></td>
                      <td>{{ option.description }}</td>
                    </tr>
                  </tbody>
                </table>
              </div>
              <p v-else class="interaction-reference-empty">No additional attributes beyond the common attributes and body format above.</p>
            </section>
          </template>
          <a class="help-top-link" href="#top">Back to top ↑</a>
        </article>

        <div v-if="!filteredArticles.length" class="help-empty">
          <span>⌕</span>
          <h2>No matching guide yet</h2>
          <p>Try a feature name such as “poll”, “frontmatter”, “Agent”, or “Q&A”.</p>
          <button type="button" class="button button-primary" @click="clearSearch">Show every topic</button>
        </div>
      </section>
    </div>
  </main>
</template>
