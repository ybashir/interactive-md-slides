<script setup lang="ts">
import { computed, defineAsyncComponent, h, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import QrcodeVue from 'qrcode.vue'

import { ApiError, api, patch, post, put } from '../api'
import type MarkdownEditorComponent from '../components/MarkdownEditor.vue'
import { clearLocalDraft, readLocalDraft, writeLocalDraft } from '../local-draft.mjs'
import { findCurrentSlideContent, findSlideNumberByKey, replaceCurrentSlideContent } from '../current-slide-source.mjs'
import { deleteOrganizedSlide, duplicateOrganizedSlide, listOrganizedSlides, reorderOrganizedSlides } from '../slide-organizer.mjs'
import { buildSlidevExportUrl, readConfiguredTheme, repairUnsafeHeadmatterTitle, setConfiguredTheme } from '../slidev-config.mjs'
import { diagnoseSource } from '../source-diagnostics.mjs'
import type { DeckAsset, DeckDetail } from '../types'
import type { LocalDraft } from '../local-draft.mjs'

const MarkdownEditor = defineAsyncComponent({ loader: () => import('../components/MarkdownEditor.vue'), loadingComponent: { render: () => h('div', { class: 'markdown-editor-loading', role: 'status' }, 'Loading editor…') } })

interface SlidevTheme {
  id: string
  label: string
  description: string
}

interface SlidevCatalog {
  slidev_version: string
  themes: SlidevTheme[]
  addons: SlidevTheme[]
  core_features: string[]
  policy: string
}

interface SlidevAccess {
  url: string
  version: number
}

interface SlidevPreflight {
  ok: boolean
  superseded?: boolean
  restart_required?: boolean
  version: number
  slidev_version?: string
  theme?: string
  startup_ms?: number
  renderer?: string
}

interface DraftSaveResponse {
  version: number
  verified_version: number | null
  is_verified: boolean
}

interface PresenterCheck {
  id: string
  label: string
  detail: string
  status: 'pass' | 'warn' | 'fail'
}

interface AssistantTurn {
  role: 'user' | 'assistant'
  text: string
}

interface AssistantOperation {
  kind: 'replace' | 'append'
  old_text: string
  new_text: string
}

interface AssistantResponse {
  proposal_id: string
  action: 'propose_patch' | 'clarify_deck_request' | 'refuse_out_of_scope'
  message: string
  summary: string
  focus_slide_key?: string
  operations: AssistantOperation[]
  proposed_markdown?: string
  base_version: number
  source_hash: string
}

interface AssistantProposal extends AssistantResponse {
  baseMarkdown: string
}

type SnippetKind = 'chart' | 'poll' | 'multi-poll' | 'quiz' | 'reaction' | 'word-cloud' | 'free-text' | 'rating' | 'number' | 'allocation' | 'matrix' | 'image-choice' | 'ranking' | 'hotspot' | 'survey' | 'ranked' | 'element-updown' | 'element-reaction' | 'element-rating' | 'live-qa' | 'live-summary' | 'audience-count'

const interactionTypes: Array<{ kind: SnippetKind; label: string; description: string }> = [
  { kind: 'chart', label: 'Chart', description: 'Add a live-quality SVG chart from a Markdown table.' },
  { kind: 'poll', label: 'Single-choice poll', description: 'Audience selects one option.' },
  { kind: 'multi-poll', label: 'Multiple-choice poll', description: 'Audience selects more than one option.' },
  { kind: 'quiz', label: 'Quiz', description: 'A scored question with a correct answer.' },
  { kind: 'reaction', label: 'Emoji reaction', description: 'Quick sentiment using familiar reactions.' },
  { kind: 'word-cloud', label: 'Word cloud', description: 'Short answers displayed by frequency.' },
  { kind: 'free-text', label: 'Free text', description: 'Collect a longer written response.' },
  { kind: 'rating', label: 'Rating scale', description: 'Choose a value on a configurable scale.' },
  { kind: 'number', label: 'Numeric estimate', description: 'Submit a number within a range.' },
  { kind: 'allocation', label: 'Point allocation', description: 'Distribute 100 points across options.' },
  { kind: 'matrix', label: '2×2 matrix', description: 'Place a response on two numeric axes.' },
  { kind: 'image-choice', label: 'Image choice', description: 'Choose between visual options.' },
  { kind: 'ranking', label: 'Audience ranking', description: 'Put every option in order.' },
  { kind: 'hotspot', label: 'Image hotspot', description: 'Place a pin on an image.' },
  { kind: 'survey', label: 'Multi-question survey', description: 'Combine choice, rating, and text questions.' },
  { kind: 'ranked', label: 'Live ranked list', description: 'Reveal ideas, vote, then reorder on close.' },
  { kind: 'live-qa', label: 'Live moderated Q&A', description: 'Project approved questions in a live, scrollable list.' },
  { kind: 'live-summary', label: 'Live interaction summary', description: 'Summarize this results session in a scrollable report.' },
  { kind: 'audience-count', label: 'Live audience count', description: 'Show how many audience members have joined.' },
  { kind: 'element-updown', label: 'Element up/down vote', description: 'Add positive and negative voting to an element.' },
  { kind: 'element-reaction', label: 'Element emoji reaction', description: 'Add a single-select emoji reaction set to an element.' },
  { kind: 'element-rating', label: 'Element rating', description: 'Add a rating scale to one Markdown element.' },
]

const route = useRoute()
const router = useRouter()
const deckId = route.params.id as string
const deck = ref<DeckDetail | null>(null)
const markdown = ref('')
const css = ref('')
const title = ref('')
const sourceKind = ref<'slide' | 'markdown' | 'css' | 'organizer'>('slide')
const currentSlideNumber = ref(1)
const previewUrl = ref('')
const previewFrame = ref<HTMLIFrameElement | null>(null)
const previewState = ref<'idle' | 'checking' | 'restarting' | 'ready' | 'error'>('idle')
const previewError = ref('')
const previewVersion = ref<number | null>(null)
const saveState = ref<'idle' | 'dirty' | 'saving' | 'saved' | 'error'>('idle')
const exportPending = ref(false)
const resetPending = ref(false)
const endPresentationPending = ref(false)
const qaSettingPending = ref(false)
const noticeMessage = ref('')
const error = ref('')
const editor = ref<InstanceType<typeof MarkdownEditorComponent> | null>(null)
const recoverableDraft = ref<LocalDraft | null>(null)
const interactionMenuOpen = ref(false)
const addMenuLevel = ref<'root' | 'interactions'>('root')
const settingsOpen = ref(false)
const interactionMenu = ref<HTMLElement | null>(null)
const interactionTrigger = ref<HTMLButtonElement | null>(null)
const settingsMenu = ref<HTMLElement | null>(null)
const settingsTrigger = ref<HTMLButtonElement | null>(null)
const workspace = ref<HTMLElement | null>(null)
const sourcePaneWidth = ref<number | null>(null)
const sourceCollapsed = ref(false)
const slidevCatalog = ref<SlidevCatalog | null>(null)
const assetsOpen = ref(false)
const assetsLoading = ref(false)
const assetUploading = ref(false)
const assets = ref<DeckAsset[]>([])
const assetError = ref('')
const preflightOpen = ref(false)
const audiencePreviewOpen = ref(false)
const assistantOpen = ref(true)
const assistantInput = ref('')
const assistantPending = ref(false)
const assistantError = ref('')
const assistantTurns = ref<AssistantTurn[]>([])
const assistantProposal = ref<AssistantProposal | null>(null)
const assistantConversation = ref<HTMLElement | null>(null)
const draggedSlideKey = ref('')
const dragTargetSlideKey = ref('')
const organizerUndoStack = ref<string[]>([])
const organizerRedoStack = ref<string[]>([])
let saveTimer: number | undefined
let previewQueued = false
let savedMarkdown = ''
let savedCss = ''
let savedTitle = ''
let interactionTypeahead = ''
let interactionTypeaheadTimer: number | undefined
let noticeTimer: number | undefined
let previewNavigationSequence = 0
let pendingPreviewNavigation: { commandId: string; slideKey: string; slideNumber: number } | null = null
let paneResizeStartX = 0
let paneResizeStartWidth = 0

const EDITOR_LAYOUT_KEY = 'interdeck:editor-layout:v1'
const ASSISTANT_MEMORY_KEY = `interdeck:slide-assistant:${deckId}:v1`
const MIN_SOURCE_PANE_WIDTH = 400
const MIN_PREVIEW_PANE_WIDTH = 480
const PREVIEW_REQUEST_TIMEOUT_MS = 15_000

const joinUrl = computed(() => deck.value ? `${window.location.origin}/j/${deck.value.join_code}` : '')
const statusLabel = computed(() => ({
  idle: 'Ready', dirty: 'Draft unsaved', saving: 'Saving draft…', saved: 'Draft saved', error: 'Draft save failed',
})[saveState.value])
const recoverableDraftTime = computed(() => recoverableDraft.value
  ? new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(recoverableDraft.value.savedAt))
  : '')
const recoverableDraftIsStale = computed(() => (
  !!recoverableDraft.value
  && !!deck.value
  && recoverableDraft.value.baseVersion < deck.value.version
))
const sourceDiagnostics = computed(() => diagnoseSource(markdown.value))
const currentSlideContent = computed(() => findCurrentSlideContent(markdown.value, currentSlideNumber.value))
const currentSlideKey = computed(() => currentSlideContent.value?.slideKey || '')
const organizedSlides = computed(() => listOrganizedSlides(markdown.value, deck.value?.slides || []))
const editorSource = computed({
  get: () => sourceKind.value === 'css'
    ? css.value
    : sourceKind.value === 'slide'
      ? currentSlideContent.value?.value || ''
      : markdown.value,
  set: value => {
    if (sourceKind.value === 'css') css.value = value
    else if (sourceKind.value === 'slide') markdown.value = replaceCurrentSlideContent(markdown.value, currentSlideNumber.value, value)
    else markdown.value = value
  },
})
const editorLanguage = computed(() => sourceKind.value === 'css' ? 'css' : 'markdown')
const editorDiagnostics = computed(() => sourceKind.value === 'markdown' ? sourceDiagnostics.value : [])
const editorAriaLabel = computed(() => ({
  slide: `Slide ${currentSlideNumber.value} Markdown content editor`,
  markdown: 'Complete Slidev Markdown editor',
  css: 'Deck-wide CSS editor',
  organizer: 'Deck Markdown backing the slide organizer',
})[sourceKind.value])
const sourceScopeDescription = computed(() => sourceKind.value === 'slide'
  ? currentSlideNumber.value === 1
    ? 'Slide 1 content · deck headmatter, boundary, and ID protected'
    : `Slide ${currentSlideContent.value?.slideNumber || currentSlideNumber.value} source · boundary and ID protected`
  : sourceKind.value === 'markdown'
    ? 'Complete deck source'
    : sourceKind.value === 'organizer'
      ? 'Reorder, duplicate, or delete slides'
      : 'Deck-wide stylesheet')
const selectedTheme = computed(() => {
  const configured = readConfiguredTheme(markdown.value)
  return slidevCatalog.value?.themes.find(theme => configured === theme.id || configured === `@slidev/theme-${theme.id}`)?.id || configured
})
const previewMatchesVersion = computed(() => (
  previewState.value === 'ready'
  && previewVersion.value !== null
  && previewVersion.value === deck.value?.version
))
const previewMatchesDeck = computed(() => (
  previewMatchesVersion.value
  && deck.value?.is_verified === true
  && !['dirty', 'saving', 'error'].includes(saveState.value)
))
const repairableHeadmatter = computed(() => {
  if (!previewError.value.toLowerCase().includes('headmatter')) return ''
  const repaired = repairUnsafeHeadmatterTitle(markdown.value)
  return repaired === markdown.value ? '' : repaired
})
const canExport = computed(() => previewMatchesDeck.value)
const sourcePaneStyle = computed(() => sourcePaneWidth.value === null
  ? {}
  : { '--source-pane-width': `${sourcePaneWidth.value}px` })
const sensitiveInteractionCount = computed(() => deck.value?.interactions.filter(interaction => (
  ['free-text', 'word-cloud', 'survey'].includes(interaction.kind)
)).length || 0)
const presenterChecks = computed<PresenterCheck[]>(() => [
  {
    id: 'saved',
    label: 'Draft saved',
    detail: saveState.value === 'saved' || saveState.value === 'idle'
      ? `Version ${deck.value?.version || '—'} is safely stored in the current deck.`
      : 'Wait for the current draft to finish saving.',
    status: saveState.value === 'saved' || saveState.value === 'idle' ? 'pass' : 'fail',
  },
  {
    id: 'build',
    label: 'Current deck ready',
    detail: previewMatchesDeck.value
      ? `Version ${previewVersion.value} passed validation and is loaded by the universal player.`
      : 'The current deck does not have a matching validated preview.',
    status: previewMatchesDeck.value ? 'pass' : 'fail',
  },
  {
    id: 'slides',
    label: 'Slide manifest ready',
    detail: `${deck.value?.slides.length || 0} slides have stable IDs.`,
    status: (deck.value?.slides.length || 0) > 0 ? 'pass' : 'fail',
  },
  {
    id: 'interactions',
    label: 'Interaction manifest ready',
    detail: `${deck.value?.interactions.length || 0} interactions passed stable-ID validation.`,
    status: 'pass',
  },
  {
    id: 'audience',
    label: 'Audience entry point ready',
    detail: `Room ${deck.value?.join_code || '—'} resolves on this production origin.`,
    status: joinUrl.value ? 'pass' : 'fail',
  },
  {
    id: 'retention',
    label: 'Results are persistent',
    detail: `Responses append to result epoch ${deck.value?.result_epoch || '—'} until you explicitly reset it.`,
    status: 'pass',
  },
  {
    id: 'privacy',
    label: sensitiveInteractionCount.value ? 'Free-text data notice' : 'No free-text collection detected',
    detail: sensitiveInteractionCount.value
      ? `${sensitiveInteractionCount.value} interaction(s) may collect audience text. Only collect what you need and export it carefully.`
      : 'This deck has no word-cloud, free-text, or survey interactions.',
    status: sensitiveInteractionCount.value ? 'warn' : 'pass',
  },
])
const presenterReady = computed(() => presenterChecks.value.every(check => check.status !== 'fail'))
const previewBusy = computed(() => previewState.value === 'checking' || previewState.value === 'restarting')
const previewStatusLabel = computed(() => {
  if (previewState.value === 'restarting') return 'Refreshing universal player…'
  if (previewState.value === 'checking') return `Validating update…`
  if (previewState.value === 'error' && previewVersion.value !== null)
    return `Update failed · showing last working version ${previewVersion.value}`
  if (previewState.value === 'error') return `Live preview needs attention`
  if (previewVersion.value !== null && (saveState.value === 'dirty' || previewVersion.value !== deck.value?.version))
    return `Saving changes · showing version ${previewVersion.value}`
  if (previewVersion.value !== null) return `Version ${previewVersion.value} live · universal player`
  return 'Loading universal player'
})

function maximumSourcePaneWidth() {
  return Math.max(MIN_SOURCE_PANE_WIDTH, (workspace.value?.clientWidth || window.innerWidth) - MIN_PREVIEW_PANE_WIDTH - 10)
}

function clampSourcePaneWidth(width: number) {
  return Math.min(maximumSourcePaneWidth(), Math.max(MIN_SOURCE_PANE_WIDTH, Math.round(width)))
}

function persistEditorLayout() {
  try {
    window.localStorage.setItem(EDITOR_LAYOUT_KEY, JSON.stringify({
      sourcePaneWidth: sourcePaneWidth.value,
      sourceCollapsed: sourceCollapsed.value,
    }))
  }
  catch {
    // Layout preferences must never interrupt editing.
  }
}

function restoreEditorLayout() {
  try {
    const stored = JSON.parse(window.localStorage.getItem(EDITOR_LAYOUT_KEY) || 'null') as {
      sourcePaneWidth?: unknown
      sourceCollapsed?: unknown
    } | null
    if (typeof stored?.sourcePaneWidth === 'number' && Number.isFinite(stored.sourcePaneWidth))
      sourcePaneWidth.value = clampSourcePaneWidth(stored.sourcePaneWidth)
    if (typeof stored?.sourceCollapsed === 'boolean') sourceCollapsed.value = stored.sourceCollapsed
  }
  catch {
    // Ignore corrupt or unavailable browser storage.
  }
}

function clampStoredPaneWidth() {
  if (sourcePaneWidth.value !== null && window.innerWidth > 980)
    sourcePaneWidth.value = clampSourcePaneWidth(sourcePaneWidth.value)
}

function beginPaneResize(event: PointerEvent) {
  if (sourceCollapsed.value || window.innerWidth <= 980 || event.button !== 0) return
  const sourcePanel = workspace.value?.querySelector<HTMLElement>('.source-panel')
  if (!sourcePanel) return
  event.preventDefault()
  paneResizeStartX = event.clientX
  paneResizeStartWidth = sourcePanel.getBoundingClientRect().width
  document.body.classList.add('resizing-editor-pane')
  window.addEventListener('pointermove', resizePaneFromPointer)
  window.addEventListener('pointerup', endPaneResize)
}

function resizePaneFromPointer(event: PointerEvent) {
  sourcePaneWidth.value = clampSourcePaneWidth(paneResizeStartWidth + event.clientX - paneResizeStartX)
}

function endPaneResize() {
  const wasResizing = document.body.classList.contains('resizing-editor-pane')
  document.body.classList.remove('resizing-editor-pane')
  window.removeEventListener('pointermove', resizePaneFromPointer)
  window.removeEventListener('pointerup', endPaneResize)
  if (wasResizing) persistEditorLayout()
}

function resizeSourcePane(delta: number) {
  const currentWidth = sourcePaneWidth.value
    || workspace.value?.querySelector<HTMLElement>('.source-panel')?.getBoundingClientRect().width
    || MIN_SOURCE_PANE_WIDTH
  sourcePaneWidth.value = clampSourcePaneWidth(currentWidth + delta)
  persistEditorLayout()
}

function toggleSourcePane() {
  if (!sourceCollapsed.value) {
    const currentWidth = workspace.value?.querySelector<HTMLElement>('.source-panel')?.getBoundingClientRect().width
    if (currentWidth) sourcePaneWidth.value = clampSourcePaneWidth(currentWidth)
  }
  sourceCollapsed.value = !sourceCollapsed.value
  persistEditorLayout()
}

onMounted(() => {
  restoreEditorLayout()
  restoreAssistantMemory()
  window.addEventListener('beforeunload', warnBeforeUnload)
  window.addEventListener('keydown', closeMenusOnEscape)
  window.addEventListener('message', receivePreviewMessage)
  window.addEventListener('resize', clampStoredPaneWidth)
  document.addEventListener('pointerdown', closeMenusOnOutsideClick)
  load()
})
onBeforeUnmount(() => {
  endPaneResize()
  window.clearTimeout(saveTimer)
  window.clearTimeout(interactionTypeaheadTimer)
  window.clearTimeout(noticeTimer)
  window.removeEventListener('beforeunload', warnBeforeUnload)
  window.removeEventListener('keydown', closeMenusOnEscape)
  window.removeEventListener('message', receivePreviewMessage)
  window.removeEventListener('resize', clampStoredPaneWidth)
  document.removeEventListener('pointerdown', closeMenusOnOutsideClick)
})

watch([markdown, css, title], () => {
  if (!deck.value) return
  if (markdown.value === savedMarkdown && css.value === savedCss && title.value === savedTitle) {
    window.clearTimeout(saveTimer)
    if (recoverableDraft.value) return
    if (saveState.value === 'saving') storeRecoveryDraft()
    else {
      clearLocalDraft(window.localStorage, deckId)
      saveState.value = 'saved'
    }
    return
  }
  recoverableDraft.value = null
  storeRecoveryDraft()
  saveState.value = 'dirty'
  window.clearTimeout(saveTimer)
  saveTimer = window.setTimeout(save, 700)
})

watch(
  [assistantOpen, assistantPending, () => assistantTurns.value.length],
  scrollAssistantToLatest,
  { flush: 'post' },
)

async function scrollAssistantToLatest() {
  if (!assistantOpen.value) return
  await nextTick()
  const conversation = assistantConversation.value
  if (conversation) conversation.scrollTop = conversation.scrollHeight
}

function storeRecoveryDraft() {
  if (!deck.value) return
  writeLocalDraft(window.localStorage, {
    deckId,
    title: title.value,
    markdown: markdown.value,
    css: css.value,
    baseVersion: deck.value.version,
  })
}

function restoreAssistantMemory() {
  try {
    const stored = JSON.parse(window.sessionStorage.getItem(ASSISTANT_MEMORY_KEY) || '[]')
    if (Array.isArray(stored)) {
      assistantTurns.value = stored
        .filter(turn => turn && (turn.role === 'user' || turn.role === 'assistant') && typeof turn.text === 'string')
        .slice(-12)
        .map(turn => ({ role: turn.role, text: turn.text.slice(0, 1500) }))
    }
  }
  catch {
    assistantTurns.value = []
  }
}

function persistAssistantMemory() {
  try {
    window.sessionStorage.setItem(ASSISTANT_MEMORY_KEY, JSON.stringify(assistantTurns.value.slice(-12)))
  }
  catch {
    // Chat memory is a convenience and must never interrupt deck editing.
  }
}

function rememberAssistantTurn(turn: AssistantTurn) {
  assistantTurns.value = [...assistantTurns.value, turn].slice(-12)
  persistAssistantMemory()
}

function clearAssistantMemory() {
  assistantTurns.value = []
  assistantProposal.value = null
  assistantError.value = ''
  window.sessionStorage.removeItem(ASSISTANT_MEMORY_KEY)
}

async function askSlideAssistant() {
  const instruction = assistantInput.value.trim()
  if (!instruction || !deck.value || deck.value.is_live || assistantPending.value || assistantProposal.value) return
  assistantError.value = ''
  if (saveState.value === 'dirty' && !await save(false)) return
  if (saveState.value === 'saving' || saveState.value === 'error') {
    assistantError.value = 'Wait for the current deck save to finish before asking Gemini.'
    return
  }
  const editorContext = editor.value?.getContext() || { selectedText: '', currentLine: 1 }
  const slideLineOffset = sourceKind.value === 'slide' && currentSlideContent.value
    ? markdown.value.slice(0, currentSlideContent.value.start).split(/\r?\n/).length - 1
    : 0
  const context = sourceKind.value === 'css'
    ? { selectedText: '', currentLine: 1 }
    : { selectedText: editorContext.selectedText, currentLine: editorContext.currentLine + slideLineOffset }
  const history = assistantTurns.value.slice(-10)
  const baseMarkdown = markdown.value
  const baseVersion = deck.value.version
  rememberAssistantTurn({ role: 'user', text: instruction })
  assistantInput.value = ''
  assistantPending.value = true
  try {
    const response = await post<AssistantResponse>(`/api/decks/${deckId}/assistant/propose`, {
      instruction,
      expected_version: baseVersion,
      history,
      selected_text: context.selectedText,
      current_line: context.currentLine,
      current_slide_number: currentSlideContent.value?.slideNumber || currentSlideNumber.value,
      editor_scope: sourceKind.value,
    })
    if (response.action === 'propose_patch' && response.proposed_markdown) {
      assistantProposal.value = { ...response, baseMarkdown }
    }
    else {
      rememberAssistantTurn({ role: 'assistant', text: response.message })
    }
  }
  catch (reason) {
    assistantError.value = reason instanceof Error ? reason.message : 'Gemini could not prepare a slide edit.'
  }
  finally {
    assistantPending.value = false
  }
}

function submitAssistantOnEnter(event: KeyboardEvent) {
  if (event.key !== 'Enter' || event.shiftKey || event.isComposing) return
  event.preventDefault()
  void askSlideAssistant()
}

async function applyAssistantProposal() {
  const proposal = assistantProposal.value
  if (!proposal || !deck.value || !proposal.proposed_markdown) return
  if (deck.value.version !== proposal.base_version || markdown.value !== proposal.baseMarkdown) {
    assistantError.value = 'The deck changed after this proposal was created. Discard it and ask Gemini again.'
    return
  }
  const focusedSlideNumber = findSlideNumberByKey(proposal.proposed_markdown, proposal.focus_slide_key)
    || currentSlideNumber.value
  markdown.value = proposal.proposed_markdown
  currentSlideNumber.value = focusedSlideNumber
  sourceKind.value = 'slide'
  await nextTick()
  if (!editor.value?.syncValue(editorSource.value)) {
    assistantError.value = 'The editor could not show the updated slide.'
    return
  }
  rememberAssistantTurn({
    role: 'assistant',
    text: `Applied: ${proposal.summary || proposal.message}${proposal.focus_slide_key ? ` Focus slide: ${proposal.focus_slide_key}.` : ''}`,
  })
  assistantProposal.value = null
  assistantError.value = ''
}

function discardAssistantProposal() {
  if (assistantProposal.value) {
    rememberAssistantTurn({ role: 'assistant', text: `Discarded proposal: ${assistantProposal.value.summary || assistantProposal.value.message}` })
  }
  assistantProposal.value = null
  assistantError.value = ''
}

function warnBeforeUnload(event: BeforeUnloadEvent) {
  if (!['dirty', 'saving', 'error'].includes(saveState.value)) return
  event.preventDefault()
  event.returnValue = ''
}

async function load() {
  sourceKind.value = 'slide'
  currentSlideNumber.value = 1
  try {
    const [value, catalog] = await Promise.all([
      api<DeckDetail>(`/api/decks/${deckId}`),
      api<SlidevCatalog>('/_gateway/slidev/catalog'),
    ])
    deck.value = value
    slidevCatalog.value = catalog
    markdown.value = savedMarkdown = value.markdown
    css.value = savedCss = value.css || ''
    title.value = savedTitle = value.title
    const localDraft = readLocalDraft(window.localStorage, deckId)
    if (localDraft && (localDraft.markdown !== value.markdown || localDraft.css !== value.css || localDraft.title !== value.title)) {
      recoverableDraft.value = localDraft
    }
    else if (localDraft) {
      clearLocalDraft(window.localStorage, deckId)
    }
    previewState.value = 'idle'
    void syncPreview()
  }
  catch (reason) {
    if (reason instanceof ApiError && reason.status === 401) return router.replace('/login')
    error.value = reason instanceof Error ? reason.message : 'Could not load this deck'
  }
}

function changeSourceSelection(event: Event) {
  const value = (event.target as HTMLSelectElement).value
  if (value.startsWith('theme:')) {
    const theme = value.slice('theme:'.length)
    if (!deck.value?.is_live && theme && theme !== selectedTheme.value) markdown.value = setConfiguredTheme(markdown.value, theme)
    nextTick(() => { (event.target as HTMLSelectElement).value = sourceKind.value })
    return
  }
  if (value === 'slide' || value === 'markdown' || value === 'css' || value === 'organizer') sourceKind.value = value
}

function applyOrganizerMarkdown(nextSource: string, focusSlideKey: string, message: string, remember = true) {
  if (nextSource === markdown.value) return
  if (remember) {
    organizerUndoStack.value = [...organizerUndoStack.value.slice(-19), markdown.value]
    organizerRedoStack.value = []
  }
  if (!editor.value?.replaceAll(nextSource)) markdown.value = nextSource
  currentSlideNumber.value = findSlideNumberByKey(nextSource, focusSlideKey) || 1
  noticeMessage.value = message
  window.clearTimeout(noticeTimer)
  noticeTimer = window.setTimeout(() => { noticeMessage.value = '' }, 4000)
}

function moveOrganizedSlide(slideKey: string, delta: number) {
  if (deck.value?.is_live) return
  const keys = organizedSlides.value.filter(slide => !slide.fixed).map(slide => slide.key)
  const from = keys.indexOf(slideKey)
  const to = from + delta
  if (from < 0 || to < 0 || to >= keys.length) return
  ;[keys[from], keys[to]] = [keys[to], keys[from]]
  applyOrganizerMarkdown(reorderOrganizedSlides(markdown.value, keys), slideKey, `Moved slide to position ${to + 2}.`)
}

function beginSlideDrag(event: DragEvent, slideKey: string) {
  if (deck.value?.is_live) return
  draggedSlideKey.value = slideKey
  event.dataTransfer?.setData('text/plain', slideKey)
  if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move'
}

function markSlideDropTarget(event: DragEvent, slideKey: string) {
  if (!draggedSlideKey.value || slideKey === draggedSlideKey.value) return
  event.preventDefault()
  dragTargetSlideKey.value = slideKey
  if (event.dataTransfer) event.dataTransfer.dropEffect = 'move'
}

function dropOrganizedSlide(event: DragEvent, targetSlideKey: string) {
  event.preventDefault()
  const draggedKey = draggedSlideKey.value || event.dataTransfer?.getData('text/plain') || ''
  const keys = organizedSlides.value.filter(slide => !slide.fixed).map(slide => slide.key)
  const from = keys.indexOf(draggedKey)
  const target = keys.indexOf(targetSlideKey)
  draggedSlideKey.value = ''
  dragTargetSlideKey.value = ''
  if (from < 0 || target < 0 || from === target) return
  keys.splice(from, 1)
  keys.splice(target, 0, draggedKey)
  applyOrganizerMarkdown(reorderOrganizedSlides(markdown.value, keys), draggedKey, `Moved slide to position ${keys.indexOf(draggedKey) + 2}.`)
}

function endSlideDrag() {
  draggedSlideKey.value = ''
  dragTargetSlideKey.value = ''
}

function editOrganizedSlide(slideKey: string) {
  const slideNumber = findSlideNumberByKey(markdown.value, slideKey) || 1
  currentSlideNumber.value = slideNumber
  sourceKind.value = 'slide'
  navigatePreviewToSlide(slideKey, slideNumber)
}

function duplicateSlideFromOrganizer(slideKey: string) {
  if (deck.value?.is_live) return
  const duplicated = duplicateOrganizedSlide(markdown.value, slideKey)
  if (!duplicated.slideKey) return
  applyOrganizerMarkdown(duplicated.source, duplicated.slideKey, 'Duplicated slide with fresh slide and interaction IDs.')
}

function deleteSlideFromOrganizer(slideKey: string) {
  if (deck.value?.is_live) return
  const slide = organizedSlides.value.find(item => item.key === slideKey)
  if (!slide || slide.fixed) return
  const interactionWarning = slide.interactionCount
    ? ` It contains ${slide.interactionCount} ID-bearing element${slide.interactionCount === 1 ? '' : 's'}; saved response contracts will remain protected by the server.`
    : ''
  if (!window.confirm(`Delete slide ${slide.number}, “${slide.title}”?${interactionWarning}`)) return
  const remaining = organizedSlides.value.filter(item => item.key !== slideKey)
  const fallback = remaining[Math.max(0, Math.min(slide.number - 2, remaining.length - 1))]?.key || remaining[0]?.key || ''
  applyOrganizerMarkdown(deleteOrganizedSlide(markdown.value, slideKey), fallback, `Deleted “${slide.title}”.`)
}

function undoOrganizerChange() {
  const previous = organizerUndoStack.value.at(-1)
  if (!previous) return
  organizerUndoStack.value = organizerUndoStack.value.slice(0, -1)
  organizerRedoStack.value = [...organizerRedoStack.value.slice(-19), markdown.value]
  applyOrganizerMarkdown(previous, currentSlideKey.value, 'Undid slide organizer change.', false)
}

function redoOrganizerChange() {
  const next = organizerRedoStack.value.at(-1)
  if (!next) return
  organizerRedoStack.value = organizerRedoStack.value.slice(0, -1)
  organizerUndoStack.value = [...organizerUndoStack.value.slice(-19), markdown.value]
  applyOrganizerMarkdown(next, currentSlideKey.value, 'Redid slide organizer change.', false)
}

async function save(updateLivePreview = true) {
  if (!deck.value || deck.value.is_live || saveState.value === 'saving') return false
  const sourceAtSave = markdown.value
  const cssAtSave = css.value
  const titleAtSave = title.value
  saveState.value = 'saving'
  error.value = ''
  try {
    const updated = await put<DraftSaveResponse>(`/api/decks/${deckId}`, {
      markdown: sourceAtSave,
      css: cssAtSave,
      title: titleAtSave,
      expected_version: deck.value.version,
    })
    deck.value.version = updated.version
    deck.value.verified_version = updated.verified_version
    deck.value.is_verified = updated.is_verified
    deck.value.can_restore_last_verified = !updated.is_verified && updated.verified_version !== null
    deck.value.markdown = sourceAtSave
    deck.value.css = cssAtSave
    deck.value.title = titleAtSave
    savedMarkdown = sourceAtSave
    savedCss = cssAtSave
    savedTitle = titleAtSave
    saveState.value = markdown.value === sourceAtSave && css.value === cssAtSave && title.value === titleAtSave ? 'saved' : 'dirty'
    if (saveState.value === 'saved') {
      clearLocalDraft(window.localStorage, deckId)
      if (updateLivePreview) void syncPreview()
    }
    if (saveState.value === 'dirty') {
      window.clearTimeout(saveTimer)
      saveTimer = window.setTimeout(save, 400)
    }
    return true
  }
  catch (reason) {
    saveState.value = 'error'
    error.value = reason instanceof ApiError && reason.status === 409
      ? `${reason.message}. Reload this page; your local draft will remain available for review.`
      : reason instanceof Error ? reason.message : 'Could not save this deck'
    return false
  }
}

async function updatePreview() {
  if (!deck.value || deck.value.is_live) return false
  if (saveState.value === 'dirty' && !await save(false)) return false
  if (saveState.value === 'saving' || saveState.value === 'error') return false
  return syncPreview()
}

async function previewApi<T>(path: string, init: RequestInit = {}): Promise<T> {
  const controller = new AbortController()
  const timeout = window.setTimeout(() => controller.abort(), PREVIEW_REQUEST_TIMEOUT_MS)
  try {
    return await api<T>(path, { ...init, signal: controller.signal })
  }
  catch (reason) {
    if (controller.signal.aborted)
      throw new Error('Preview validation timed out. Your draft is saved; retry the preview without reloading the editor.')
    throw reason
  }
  finally {
    window.clearTimeout(timeout)
  }
}

function repairHeadmatterTitle() {
  if (!repairableHeadmatter.value) return
  if (!editor.value?.replaceAll(repairableHeadmatter.value)) {
    error.value = 'The editor could not apply the YAML title repair.'
    return
  }
  previewError.value = ''
  noticeMessage.value = 'Quoted the deck title. Saving and refreshing the preview…'
}

async function syncPreview() {
  if (!deck.value || deck.value.is_live) return false
  if (previewBusy.value) {
    previewQueued = true
    return false
  }
  const targetVersion = deck.value.version
  if (previewMatchesVersion.value) return true
  previewState.value = 'checking'
  previewError.value = ''
  try {
    const access = await previewApi<SlidevAccess>(`/api/decks/${deckId}/slidev-access?mode=preview`)
    if (access.version !== targetVersion) throw new Error('A newer saved version is available; updating again now.')
    const accessToken = new URL(access.url, window.location.origin).searchParams.get('access')
    if (!accessToken) throw new Error('The preview access token was missing')
    let preflight = await previewApi<SlidevPreflight>('/_gateway/slidev/preflight', {
      method: 'POST',
      body: JSON.stringify({ deck_id: deckId, access: accessToken, allow_restart: false }),
    })
    if (preflight.superseded) {
      previewQueued = true
      return false
    }
    if (preflight.restart_required) {
      if (!deck.value || deck.value.version !== targetVersion) {
        previewQueued = true
        return false
      }
      previewState.value = 'restarting'
      preflight = await previewApi<SlidevPreflight>('/_gateway/slidev/preflight', {
        method: 'POST',
        body: JSON.stringify({ deck_id: deckId, access: accessToken, allow_restart: true }),
      })
      if (preflight.superseded) {
        previewQueued = true
        return false
      }
    }
    if (!deck.value || deck.value.version !== targetVersion || preflight.version !== targetVersion) {
      previewQueued = true
      return false
    }
    const cleanPreviewUrl = new URL(access.url, window.location.origin)
    cleanPreviewUrl.searchParams.delete('access')
    cleanPreviewUrl.searchParams.set('version', String(targetVersion))
    const nextPreviewUrl = `${cleanPreviewUrl.pathname}${cleanPreviewUrl.search}`
    const refreshed = await previewApi<DeckDetail>(`/api/decks/${deckId}`)
    if (deck.value.version === targetVersion && refreshed.version === targetVersion) {
      deck.value = refreshed
      if (!previewUrl.value) previewUrl.value = nextPreviewUrl
      else sendPreviewSource(refreshed)
    }
    else {
      previewQueued = true
      return false
    }
    previewVersion.value = preflight.version
    previewState.value = 'ready'
    return true
  }
  catch (reason) {
    previewState.value = 'error'
    previewError.value = reason instanceof Error ? reason.message : 'Slidev could not update the live preview.'
    return false
  }
  finally {
    const shouldContinue = previewQueued || (!!deck.value && deck.value.version !== targetVersion)
    previewQueued = false
    // Superseded-version paths intentionally return before marking the preview
    // ready. Clear their busy state before starting the queued validation;
    // otherwise the next call sees itself as busy and can remain stuck forever.
    if (shouldContinue && previewBusy.value) previewState.value = 'idle'
    if (shouldContinue && saveState.value === 'saved') queueMicrotask(() => void syncPreview())
  }
}

function sendPreviewSource(value: DeckDetail | null = deck.value) {
  if (!value || !previewFrame.value?.contentWindow) return
  previewFrame.value.contentWindow.postMessage({
    type: 'interdeck:reload-source',
    version: value.version,
    deck: JSON.parse(JSON.stringify(value)),
  }, window.location.origin)
}

function postPreviewNavigation() {
  if (!pendingPreviewNavigation || !previewFrame.value?.contentWindow) return
  previewFrame.value.contentWindow.postMessage({
    type: 'interdeck:command',
    action: 'go',
    slideKey: pendingPreviewNavigation.slideKey,
    slideNumber: pendingPreviewNavigation.slideNumber,
    clickStep: 0,
    commandId: pendingPreviewNavigation.commandId,
  }, window.location.origin)
}

function navigatePreviewToSlide(slideKey: string, slideNumber: number) {
  previewNavigationSequence += 1
  pendingPreviewNavigation = {
    commandId: `editor-organizer-${previewNavigationSequence}`,
    slideKey,
    slideNumber,
  }
  postPreviewNavigation()
}

function receivePreviewMessage(event: MessageEvent) {
  if (event.origin !== window.location.origin || event.source !== previewFrame.value?.contentWindow) return
  if (event.data?.type === 'interdeck:ready') {
    postPreviewNavigation()
    return
  }
  if (event.data?.type !== 'interdeck:navigation') return
  const requested = Number(event.data.slideNumber)
  if (!Number.isFinite(requested)) return
  if (pendingPreviewNavigation && event.data.commandId === pendingPreviewNavigation.commandId) {
    currentSlideNumber.value = findSlideNumberByKey(markdown.value, pendingPreviewNavigation.slideKey)
      || pendingPreviewNavigation.slideNumber
    pendingPreviewNavigation = null
    return
  }
  currentSlideNumber.value = Math.max(1, Math.min(Math.trunc(requested), deck.value?.slides.length || 1))
}

async function launch(mode: 'live' | 'rehearsal') {
  const consoleRoute = router.resolve({
    path: `/decks/${deckId}/present`,
    query: { ...(mode === 'rehearsal' ? { mode } : {}), view: 'console' },
  })
  const presenterConsole = window.open(
    consoleRoute.href,
    `interdeck-presenter-${deckId}`,
    'popup,width=480,height=900,resizable=yes,scrollbars=yes',
  )
  if (!presenterConsole) {
    error.value = 'Your browser blocked the private presenter console. Allow pop-ups for Interdeck and try again.'
    return
  }
  presenterConsole.focus()

  if (!previewMatchesDeck.value && !await updatePreview()) {
    presenterConsole.close()
    return
  }
  if (saveState.value === 'error' || !previewMatchesDeck.value) {
    presenterConsole.close()
    error.value = 'Presentation blocked: fix the preview validation error before presenting this deck.'
    return
  }
  await router.push({ path: `/decks/${deckId}/present`, query: mode === 'rehearsal' ? { mode } : {} })
}

async function prepareToPresent() {
  error.value = ''
  if (!previewMatchesDeck.value && !await updatePreview()) return
  if (saveState.value === 'saving' || saveState.value === 'error' || !previewMatchesDeck.value) return
  preflightOpen.value = true
}

function downloadResults() {
  if (!deck.value) return
  const link = document.createElement('a')
  link.href = `/api/decks/${deckId}/results.csv?epoch=${deck.value.result_epoch}`
  link.download = `interdeck-results-epoch-${deck.value.result_epoch}.csv`
  document.body.appendChild(link)
  link.click()
  link.remove()
}

async function confirmPresent() {
  if (!presenterReady.value) return
  preflightOpen.value = false
  await launch('live')
}

async function endPresentationFromEditor() {
  if (!deck.value?.is_live || endPresentationPending.value) return
  if (!window.confirm(`End the live presentation “${deck.value.title}”?`)) return
  endPresentationPending.value = true
  error.value = ''
  try {
    await post(`/api/decks/${deckId}/stop`)
    deck.value.is_live = false
    noticeMessage.value = 'Presentation ended. This deck can be edited again.'
    window.clearTimeout(noticeTimer)
    noticeTimer = window.setTimeout(() => { noticeMessage.value = '' }, 5000)
  }
  catch (reason) {
    error.value = reason instanceof Error ? reason.message : 'Could not end the presentation'
  }
  finally {
    endPresentationPending.value = false
  }
}

async function exportDeck() {
  if (!canExport.value || exportPending.value) return
  error.value = ''
  // Open synchronously while the settings click still carries a browser user
  // gesture. Waiting for the access-token request first allows Chrome to treat
  // the exporter as an unsolicited popup.
  const exportWindow = window.open('', '_blank')
  if (!exportWindow) {
    error.value = `Your browser blocked the PDF exporter. Allow pop-ups for ${window.location.host} and try again.`
    return
  }
  exportWindow.opener = null
  exportWindow.document.write(`<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Preparing PDF export…</title><style>
:root{font-family:Inter,ui-sans-serif,system-ui,sans-serif;color:#172033;background:#f5f2eb}
body{min-height:100vh;display:grid;place-items:center;margin:0}.card{padding:32px 36px;border:1px solid #ded8cc;border-radius:18px;background:#fff;box-shadow:0 18px 60px #26324a14;text-align:center}
.spinner{width:28px;height:28px;margin:0 auto 16px;border:3px solid #ddd7fa;border-top-color:#6b58d9;border-radius:50%;animation:spin .8s linear infinite}@keyframes spin{to{transform:rotate(360deg)}}
h1{margin:0 0 8px;font-size:22px}p{margin:0;color:#626978;font-size:15px}
</style></head><body><main class="card"><div class="spinner"></div><h1>Preparing PDF exporter</h1><p>Loading the verified deck…</p></main></body></html>`)
  exportWindow.document.close()
  exportPending.value = true
  try {
    const access = await api<SlidevAccess>(`/api/decks/${deckId}/slidev-access?mode=export`)
    exportWindow.location.replace(buildSlidevExportUrl(access.url, window.location.href))
  }
  catch (reason) {
    exportWindow.close()
    error.value = reason instanceof Error ? reason.message : 'Could not prepare the Slidev export'
  }
  finally {
    exportPending.value = false
  }
}

async function copyJoinLink() {
  await navigator.clipboard.writeText(joinUrl.value)
}

async function rotateJoinCode() {
  if (!deck.value || deck.value.is_live) return
  if (!window.confirm('Rotate this room code? Existing links and QR codes using the old code will stop working immediately.')) return
  error.value = ''
  try {
    const result = await post<{ join_code: string }>(`/api/decks/${deckId}/join-code/rotate`)
    deck.value.join_code = result.join_code
    await navigator.clipboard.writeText(joinUrl.value)
  }
  catch (reason) {
    error.value = reason instanceof Error ? reason.message : 'Could not rotate this room code'
  }
}

async function resetInteractionData() {
  if (!deck.value || deck.value.is_live || resetPending.value) return
  const confirmed = window.confirm(
    'Reset all interaction data for this deck? This starts a fresh result epoch for votes, responses, word clouds, free text, and Q&A. Existing data remains archived in the database.',
  )
  if (!confirmed) return
  resetPending.value = true
  error.value = ''
  try {
    const result = await post<{ epoch: number }>(`/api/decks/${deckId}/reset`)
    deck.value.result_epoch = result.epoch
    noticeMessage.value = `Interaction data reset. Result epoch ${result.epoch} is now empty.`
    window.clearTimeout(noticeTimer)
    noticeTimer = window.setTimeout(() => { noticeMessage.value = '' }, 5000)
  }
  catch (reason) {
    error.value = reason instanceof Error ? reason.message : 'Could not reset interaction data'
  }
  finally {
    resetPending.value = false
  }
}

async function updateQaDisplayMode(displayMode: 'verbatim' | 'ai_grouped') {
  if (!deck.value || qaSettingPending.value || deck.value.qa_display_mode === displayMode) return
  qaSettingPending.value = true
  error.value = ''
  try {
    const result = await patch<{ qa_display_mode: 'verbatim' | 'ai_grouped' }>(`/api/decks/${deckId}/qa-settings`, {
      display_mode: displayMode,
    })
    deck.value.qa_display_mode = result.qa_display_mode
    noticeMessage.value = displayMode === 'verbatim'
      ? 'Q&A slide now shows approved questions verbatim. Gemini theme generation is paused.'
      : 'Q&A slide will group and rephrase approved questions with Gemini.'
    window.clearTimeout(noticeTimer)
    noticeTimer = window.setTimeout(() => { noticeMessage.value = '' }, 5000)
  }
  catch (reason) {
    error.value = reason instanceof Error ? reason.message : 'Could not update the Q&A display mode'
  }
  finally {
    qaSettingPending.value = false
  }
}

async function restoreLastWorking() {
  if (!deck.value || deck.value.is_live) return
  if (!window.confirm('Restore the last version that rendered successfully? Your current Markdown will be replaced.')) return
  error.value = ''
  try {
    const restored = await post<DeckDetail>(`/api/decks/${deckId}/restore-last-verified`, {
      expected_version: deck.value.version,
    })
    deck.value = restored
    markdown.value = savedMarkdown = restored.markdown
    css.value = savedCss = restored.css || ''
    title.value = savedTitle = restored.title
    saveState.value = 'saved'
    clearLocalDraft(window.localStorage, deckId)
    await updatePreview()
  }
  catch (reason) {
    error.value = reason instanceof Error ? reason.message : 'Could not restore the last working deck'
  }
}

async function openAssets() {
  assetsOpen.value = true
  assetsLoading.value = true
  assetError.value = ''
  try {
    assets.value = await api<DeckAsset[]>(`/api/decks/${deckId}/assets`)
  }
  catch (reason) {
    assetError.value = reason instanceof Error ? reason.message : 'Could not load deck assets'
  }
  finally {
    assetsLoading.value = false
  }
}

async function uploadAsset(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  assetUploading.value = true
  assetError.value = ''
  try {
    const form = new FormData()
    form.append('file', file)
    const asset = await api<DeckAsset>(`/api/decks/${deckId}/assets`, { method: 'POST', body: form })
    assets.value = [asset, ...assets.value.filter(existing => existing.id !== asset.id)]
  }
  catch (reason) {
    assetError.value = reason instanceof Error ? reason.message : 'Could not upload this asset'
  }
  finally {
    assetUploading.value = false
    input.value = ''
  }
}

function insertAsset(asset: DeckAsset) {
  if (asset.media_type.startsWith('image/')) {
    const alt = asset.original_filename.replace(/\.[^.]+$/, '').replace(/[\[\]]/g, '')
    insertMarkdown(`\n![${alt}](${asset.content_url})\n`)
  }
  else {
    const family = asset.original_filename.replace(/\.[^.]+$/, '').replace(/['"\\]/g, '') || 'Deck font'
    const format = asset.media_type === 'font/woff2' ? 'woff2' : 'woff'
    insertMarkdown(`\n<style>\n@font-face { font-family: '${family}'; src: url('${asset.content_url}') format('${format}'); font-display: swap; }\n</style>\n`)
  }
  assetsOpen.value = false
}

async function deleteAsset(asset: DeckAsset) {
  if (!window.confirm(`Delete ${asset.original_filename}? Assets used by the current or last working deck cannot be deleted.`)) return
  assetError.value = ''
  try {
    await api(`/api/decks/${deckId}/assets/${asset.id}`, { method: 'DELETE' })
    assets.value = assets.value.filter(existing => existing.id !== asset.id)
  }
  catch (reason) {
    assetError.value = reason instanceof Error ? reason.message : 'Could not delete this asset'
  }
}

async function insertMarkdown(source: string) {
  if (sourceKind.value === 'css') sourceKind.value = currentSlideContent.value ? 'slide' : 'markdown'
  await nextTick()
  if (!editor.value?.insertText(source)) editorSource.value += source
}

function insertAudienceQr() {
  insertMarkdown('\n::audience-qr{size="180"}\n')
}

function recoverDraft() {
  if (!recoverableDraft.value || deck.value?.is_live) return
  const draft = recoverableDraft.value
  recoverableDraft.value = null
  title.value = draft.title
  markdown.value = draft.markdown
  css.value = draft.css
}

function discardDraft() {
  clearLocalDraft(window.localStorage, deckId)
  recoverableDraft.value = null
}

function menuItems(menu: HTMLElement | null) {
  return menu ? Array.from(menu.querySelectorAll<HTMLElement>('button[role="menuitem"]:not(:disabled), a[role="menuitem"]')) : []
}

function toggleInteractionMenu() {
  const opening = !interactionMenuOpen.value
  settingsOpen.value = false
  addMenuLevel.value = 'root'
  interactionMenuOpen.value = opening
  if (opening) nextTick(() => menuItems(interactionMenu.value)[0]?.focus())
}

function toggleSettingsMenu() {
  const opening = !settingsOpen.value
  interactionMenuOpen.value = false
  settingsOpen.value = opening
  if (opening) nextTick(() => menuItems(settingsMenu.value)[0]?.focus())
}

function closeMenusOnEscape(event: KeyboardEvent) {
  if (event.key !== 'Escape') return
  if (interactionMenuOpen.value) {
    interactionMenuOpen.value = false
    interactionTrigger.value?.focus()
    event.preventDefault()
  }
  else if (settingsOpen.value) {
    settingsOpen.value = false
    settingsTrigger.value?.focus()
    event.preventDefault()
  }
}

function closeMenusOnOutsideClick(event: PointerEvent) {
  const target = event.target as Node
  if (interactionMenuOpen.value && !interactionMenu.value?.parentElement?.contains(target)) interactionMenuOpen.value = false
  if (settingsOpen.value && !settingsMenu.value?.parentElement?.contains(target)) settingsOpen.value = false
}

function navigateMenu(event: KeyboardEvent, menu: HTMLElement | null) {
  const items = menuItems(menu)
  if (!items.length) return
  const current = Math.max(0, items.indexOf(document.activeElement as HTMLElement))
  let next: number | undefined
  if (event.key === 'ArrowDown') next = (current + 1) % items.length
  else if (event.key === 'ArrowUp') next = (current - 1 + items.length) % items.length
  else if (event.key === 'Home') next = 0
  else if (event.key === 'End') next = items.length - 1
  if (next === undefined) return
  event.preventDefault()
  items[next]?.focus()
}

function navigateInteractionMenu(event: KeyboardEvent) {
  navigateMenu(event, interactionMenu.value)
  if (event.ctrlKey || event.metaKey || event.altKey || event.key.length !== 1 || !/[a-z0-9]/i.test(event.key)) return
  interactionTypeahead += event.key.toLowerCase()
  window.clearTimeout(interactionTypeaheadTimer)
  interactionTypeaheadTimer = window.setTimeout(() => { interactionTypeahead = '' }, 600)
  const match = menuItems(interactionMenu.value).find(item => item.dataset.label?.startsWith(interactionTypeahead))
  if (match) {
    event.preventDefault()
    match.focus()
  }
}

function showInteractionChoices() {
  addMenuLevel.value = 'interactions'
  nextTick(() => menuItems(interactionMenu.value)[0]?.focus())
}

function showAddChoices() {
  addMenuLevel.value = 'root'
  nextTick(() => menuItems(interactionMenu.value)[0]?.focus())
}

function chooseAssetFromAddMenu() {
  interactionMenuOpen.value = false
  void openAssets()
}

function insertQrFromAddMenu() {
  interactionMenuOpen.value = false
  void insertAudienceQr()
}

async function runSettingsAction(action: () => void | Promise<void>) {
  settingsOpen.value = false
  await action()
}

function insertSnippet(kind: SnippetKind) {
  const id = `${kind}-${Date.now().toString(36)}`
  const snippets = {
    chart: `\n:::echarts{height="320"}\n{\n  "tooltip": { "trigger": "axis" },\n  "legend": { "bottom": 0 },\n  "grid": { "left": 48, "right": 20, "top": 20, "bottom": 48, "containLabel": true },\n  "dataset": {\n    "source": [\n      ["Quarter", "Revenue", "Costs"],\n      ["Q1", 12, 8],\n      ["Q2", 18, 11],\n      ["Q3", 27, 16],\n      ["Q4", 34, 20]\n    ]\n  },\n  "xAxis": { "type": "category" },\n  "yAxis": { "type": "value" },\n  "series": [\n    { "type": "bar" },\n    { "type": "bar" }\n  ]\n}\n:::\n`,
    poll: `\n:::interact{type="poll" id="${id}" results="after-vote"}\n- [option-a] Option A\n- [option-b] Option B\n:::\n`,
    'multi-poll': `\n:::interact{type="poll" id="${id}" multiple="true" max="2" results="after-vote"}\n- [option-a] Option A\n- [option-b] Option B\n- [option-c] Option C\n:::\n`,
    quiz: `\n:::interact{type="quiz" id="${id}" correct="option-a" results="manual" timer="30"}\n- [option-a] Correct answer\n- [option-b] Another answer\n- [option-c] Another answer\n:::\n`,
    reaction: `\n:::interact{type="reaction" id="${id}" results="always"}\n- [love] ❤️ Love it\n- [thinking] 🤔 Thinking\n- [question] ❓ Questions\n- [concern] ⚠️ Concern\n:::\n`,
    'word-cloud': `\n:::interact{type="word-cloud" id="${id}" entries="multiple" height="320" spacing="0.16"}\n:::\n`,
    'free-text': `\n:::interact{type="free-text" id="${id}" results="presenter"}\n:::\n`,
    rating: `\n:::interact{type="rating" id="${id}" min="1" max="5"}\n:::\n`,
    number: `\n:::interact{type="number" id="${id}" min="0" max="100" step="1" unit="%" results="after-vote"}\n:::\n`,
    allocation: `\n:::interact{type="allocation" id="${id}" total="100" results="after-vote"}\n- [quality] Quality\n- [speed] Speed\n- [learning] Learning\n:::\n`,
    matrix: `\n:::interact{type="matrix" id="${id}" x-min="0" x-max="10" x-label="Effort" y-min="0" y-max="10" y-label="Impact" results="after-vote"}\n:::\n`,
    'image-choice': `\n:::interact{type="image-choice" id="${id}" results="after-vote"}\n- [option-a] Option A {image="https://placehold.co/640x480?text=Option+A"}\n- [option-b] Option B {image="https://placehold.co/640x480?text=Option+B"}\n:::\n`,
    ranking: `\n:::interact{type="ranking" id="${id}" results="after-vote"}\n- [quality] Quality\n- [speed] Speed\n- [learning] Learning\n:::\n`,
    hotspot: `\n:::interact{type="image-hotspot" id="${id}" image="https://placehold.co/1200x675?text=Click+target" alt="Diagram to place a pin on" results="after-vote"}\n:::\n`,
    survey: `\n:::interact{type="survey" id="${id}" results="presenter"}\n- [confidence] How confident are you? {type="rating" min="1" max="5"}\n- [priority] Which priority matters most? {type="choice" options="quality:Quality|speed:Speed|learning:Learning"}\n- [comment] What should we improve? {type="text" max="500" required="false"}\n:::\n`,
    ranked: `\n:::interact{type="ranked-list" id="${id}" display="slide" reveal="click" results="on-close"}\n- [idea-a] First idea\n- [idea-b] Second idea\n- [idea-c] Third idea\n:::\n`,
    'live-qa': `\n---\nlayout: default\n---\n\n# Questions from the room\n\n::live-qa{show="open"}\n`,
    'live-summary': `\n---\nlayout: default\n---\n\n# What the room told us\n\n::live-summary\n`,
    'audience-count': `\n::audience-count{label="people joined"}\n`,
    'element-updown': `\n- An idea to evaluate {interact="updown" id="${id}"}\n`,
    'element-reaction': `\n- An idea to react to {interact="reaction" id="${id}" options="like:👍|celebrate:🎉|question:🤔"}\n`,
    'element-rating': `\n- An idea to rate {interact="rating" id="${id}" min="1" max="5"}\n`,
  } satisfies Record<SnippetKind, string>
  interactionMenuOpen.value = false
  void insertMarkdown(snippets[kind])
}
</script>

<template>
  <main class="editor-page">
    <header class="editor-topbar">
      <RouterLink to="/" class="wordmark"><span class="wordmark-icon">I</span> interdeck</RouterLink>
      <div class="deck-title-wrap">
        <input v-model="title" :disabled="deck?.is_live" maxlength="120" aria-label="Deck title">
        <span :class="['save-status', saveState]">{{ statusLabel }}</span>
      </div>
      <div class="toolbar-actions">
        <button class="button button-ghost" @click="copyJoinLink">Join · {{ deck?.join_code }}</button>
        <button class="button button-primary" :disabled="!deck || saveState === 'saving' || saveState === 'error' || previewBusy" @click="prepareToPresent">Present <span>▶</span></button>
        <div class="deck-settings">
          <button ref="settingsTrigger" class="settings-trigger" aria-label="Deck settings" aria-haspopup="menu" :aria-expanded="settingsOpen" @click="toggleSettingsMenu"><span aria-hidden="true">⚙</span></button>
          <div v-if="settingsOpen" ref="settingsMenu" class="settings-menu" role="menu" aria-label="Deck settings" @keydown="navigateMenu($event, settingsMenu)">
            <button role="menuitem" :disabled="!deck || deck.is_live" @click="runSettingsAction(rotateJoinCode)"><strong>Rotate room</strong><span>Replace the audience code and copy the new link.</span></button>
            <button role="menuitem" :disabled="!deck || qaSettingPending" @click="runSettingsAction(() => updateQaDisplayMode('verbatim'))"><strong>{{ deck?.qa_display_mode === 'verbatim' ? '✓ ' : '' }}Approved questions verbatim</strong><span>Default. Show exact moderated questions and avoid Gemini theme generation.</span></button>
            <button role="menuitem" :disabled="!deck || qaSettingPending" @click="runSettingsAction(() => updateQaDisplayMode('ai_grouped'))"><strong>{{ deck?.qa_display_mode === 'ai_grouped' ? '✓ ' : '' }}AI-grouped questions</strong><span>Show the latest presenter-requested Gemini brief on the Q&A slide.</span></button>
            <button role="menuitem" :disabled="!canExport || exportPending" @click="runSettingsAction(exportDeck)"><strong>{{ exportPending ? 'Preparing PDF export…' : 'PDF export' }}</strong><span>Open Slidev's browser exporter for the current verified deck.</span></button>
            <button role="menuitem" :disabled="!deck" @click="runSettingsAction(downloadResults)"><strong>Results CSV</strong><span>Download responses from result epoch {{ deck?.result_epoch || '—' }}.</span></button>
            <button role="menuitem" :disabled="!deck || deck.is_live || resetPending" @click="runSettingsAction(resetInteractionData)"><strong>{{ resetPending ? 'Resetting interaction data…' : 'Reset interaction data' }}</strong><span>Start a fresh empty epoch while retaining prior data in the archive.</span></button>
            <button role="menuitem" :disabled="!deck || saveState === 'saving' || saveState === 'error' || previewBusy" @click="runSettingsAction(() => launch('rehearsal'))"><strong>Test presentation</strong><span>Use isolated test results without changing live tallies.</span></button>
            <a role="menuitem" href="/help" target="_blank" rel="noopener noreferrer" @click="settingsOpen = false"><strong>Help Center ↗</strong><span>Open the complete Interdeck guide in a new tab.</span></a>
          </div>
        </div>
      </div>
    </header>

    <div v-if="error" class="editor-error">{{ error }}</div>
    <div v-if="noticeMessage" class="editor-notice">{{ noticeMessage }}</div>
    <div v-if="deck?.is_live" class="live-edit-lock">
      <span class="live-pill"><i /> LIVE</span>
      <span>End the presentation before editing this deck.</span>
      <button type="button" :disabled="endPresentationPending" @click="endPresentationFromEditor">{{ endPresentationPending ? 'Ending…' : 'End presentation' }}</button>
    </div>
    <aside v-if="recoverableDraft" class="draft-recovery" role="status">
      <div>
        <strong>{{ recoverableDraftIsStale ? 'Outdated local draft found' : 'Unsaved local draft found' }}</strong>
        <span v-if="recoverableDraftIsStale">Saved from version {{ recoverableDraft.baseVersion }}, while the server is now version {{ deck?.version }}. Discard it unless it contains work you still need.</span>
        <span v-else>From {{ recoverableDraftTime }} · based on version {{ recoverableDraft.baseVersion }}</span>
      </div>
      <button :disabled="deck?.is_live" @click="recoverDraft">{{ recoverableDraftIsStale ? 'Recover anyway' : 'Recover' }}</button>
      <button class="draft-discard" @click="discardDraft">{{ recoverableDraftIsStale ? 'Discard outdated draft' : 'Discard' }}</button>
    </aside>

    <section ref="workspace" :class="['editor-workspace', { 'source-collapsed': sourceCollapsed }]" :style="sourcePaneStyle">
      <div class="source-panel">
        <div class="panel-heading">
          <div class="source-picker">
            <label for="editor-source-kind">Edit</label>
            <select id="editor-source-kind" :value="sourceKind" :title="sourceScopeDescription" aria-describedby="editor-source-description" @change="changeSourceSelection">
              <optgroup label="Edit">
                <option value="slide">Current slide</option>
                <option value="markdown">All slides</option>
                <option value="organizer">Slide organizer</option>
                <option value="css">Deck styles</option>
              </optgroup>
              <optgroup :label="`Theme · ${selectedTheme || 'loading'}`" :disabled="deck?.is_live || !slidevCatalog">
                <option v-if="slidevCatalog && !slidevCatalog.themes.some(theme => theme.id === selectedTheme)" :value="`theme:${selectedTheme}`" disabled>Unsupported · {{ selectedTheme }}</option>
                <option v-for="theme in slidevCatalog?.themes" :key="theme.id" :value="`theme:${theme.id}`" :title="theme.description">{{ theme.id === selectedTheme ? '✓ ' : '' }}{{ theme.label }}</option>
              </optgroup>
            </select>
            <span id="editor-source-description" class="source-scope-description">{{ sourceScopeDescription }}</span>
          </div>
          <span v-if="sourceKind === 'markdown' && sourceDiagnostics.length" class="source-issue-count" role="status">{{ sourceDiagnostics.length }} source issue{{ sourceDiagnostics.length === 1 ? '' : 's' }}</span>
          <div class="source-actions">
            <div v-if="sourceKind !== 'css' && sourceKind !== 'organizer'" class="interaction-insert">
              <button ref="interactionTrigger" class="source-add-button" aria-label="Add slide content" aria-haspopup="menu" :aria-expanded="interactionMenuOpen" :disabled="deck?.is_live" @click="toggleInteractionMenu"><span aria-hidden="true">+</span><b>Add</b></button>
              <div v-if="interactionMenuOpen" ref="interactionMenu" class="interaction-menu" role="menu" aria-label="Add slide content" @keydown="navigateInteractionMenu">
                <template v-if="addMenuLevel === 'root'">
                  <header><strong>Add to slide</strong><span>Insert at the current cursor</span></header>
                  <button role="menuitem" data-label="interaction" @click="showInteractionChoices"><strong>Interaction or chart</strong><span>Polls, Q&amp;A, reactions, audience count, ECharts, and more.</span></button>
                  <button role="menuitem" data-label="asset" @click="chooseAssetFromAddMenu"><strong>Asset</strong><span>Upload or insert a private image, SVG, or font.</span></button>
                  <button role="menuitem" data-label="qr code" @click="insertQrFromAddMenu"><strong>Audience QR code</strong><span>Insert this deck's live room QR component.</span></button>
                </template>
                <template v-else>
                  <header class="interaction-menu-subheader"><button role="menuitem" data-label="back" aria-label="Back to Add menu" @click="showAddChoices">←</button><span><strong>Interactions &amp; charts</strong><small>Choose a component to insert</small></span></header>
                  <button v-for="item in interactionTypes" :key="item.kind" role="menuitem" :data-label="item.label.toLowerCase()" @click="insertSnippet(item.kind)"><strong>{{ item.label }}</strong><span>{{ item.description }}</span></button>
                </template>
              </div>
            </div>
          </div>
        </div>
        <MarkdownEditor v-show="sourceKind !== 'organizer'" ref="editor" v-model="editorSource" :language="editorLanguage" :aria-label="editorAriaLabel" :disabled="deck?.is_live" :diagnostics="editorDiagnostics" />
        <section v-if="sourceKind === 'organizer'" class="slide-organizer" aria-labelledby="slide-organizer-title">
          <header>
            <div><strong id="slide-organizer-title">Slide organizer</strong><span>Drag slides into order, or use the arrow buttons.</span></div>
            <div class="organizer-history"><button type="button" :disabled="!organizerUndoStack.length || deck?.is_live" @click="undoOrganizerChange">Undo</button><button type="button" :disabled="!organizerRedoStack.length || deck?.is_live" @click="redoOrganizerChange">Redo</button></div>
          </header>
          <ol>
            <li
              v-for="slide in organizedSlides"
              :key="slide.key"
              :class="[{ fixed: slide.fixed, current: slide.key === currentSlideKey, dragging: slide.key === draggedSlideKey, target: slide.key === dragTargetSlideKey }]"
              :draggable="!slide.fixed && !deck?.is_live"
              @dragstart="beginSlideDrag($event, slide.key)"
              @dragover="markSlideDropTarget($event, slide.key)"
              @drop="dropOrganizedSlide($event, slide.key)"
              @dragend="endSlideDrag"
            >
              <span class="organizer-grip" :aria-label="slide.fixed ? 'Title slide is fixed' : 'Drag to reorder slide'" aria-hidden="true">{{ slide.fixed ? '◆' : '⋮⋮' }}</span>
              <button type="button" class="organizer-slide-main" @click="editOrganizedSlide(slide.key)">
                <b>{{ slide.number }}</b>
                <span><strong>{{ slide.title }}</strong><small>{{ slide.fixed ? 'Title slide · deck headmatter stays first' : slide.key }}<template v-if="slide.interactionCount"> · {{ slide.interactionCount }} ID-bearing element{{ slide.interactionCount === 1 ? '' : 's' }}</template></small></span>
              </button>
              <div class="organizer-slide-actions">
                <button type="button" title="Move slide up" aria-label="Move slide up" :disabled="slide.fixed || slide.number === 2 || deck?.is_live" @click="moveOrganizedSlide(slide.key, -1)">↑</button>
                <button type="button" title="Move slide down" aria-label="Move slide down" :disabled="slide.fixed || slide.number === organizedSlides.length || deck?.is_live" @click="moveOrganizedSlide(slide.key, 1)">↓</button>
                <button type="button" title="Duplicate slide" aria-label="Duplicate slide" :disabled="slide.fixed || deck?.is_live" @click="duplicateSlideFromOrganizer(slide.key)">⧉</button>
                <button type="button" class="organizer-delete" title="Delete slide" aria-label="Delete slide" :disabled="slide.fixed || deck?.is_live" @click="deleteSlideFromOrganizer(slide.key)">×</button>
              </div>
            </li>
          </ol>
          <footer>Click a slide to edit it. Slide 1 remains fixed because its frontmatter configures the whole deck.</footer>
        </section>
        <section v-if="sourceKind !== 'organizer'" :class="['slide-assistant', { collapsed: !assistantOpen }]">
          <header>
            <button class="assistant-toggle" type="button" :aria-expanded="assistantOpen" @click="assistantOpen = !assistantOpen"><i>✦</i><span>Gemini Slide Assistant</span><small>{{ assistantOpen ? 'Hide' : 'Show' }}</small></button>
            <button v-if="assistantOpen && assistantTurns.length" class="assistant-clear" type="button" title="Forget this tab's conversation" @click="clearAssistantMemory">Clear</button>
          </header>
          <template v-if="assistantOpen">
            <p class="microcopy">Requests send your instruction, conversation, and relevant deck content to Google Gemini. <a href="/privacy">Privacy details</a></p>
            <div ref="assistantConversation" class="assistant-conversation">
              <div v-if="!assistantTurns.length" class="assistant-welcome"><strong>Ask for a deck change</strong><span>Gemini knows the slide in the preview, the complete deck, Slidev, and every Interdeck interaction.</span></div>
              <div v-for="(turn, index) in assistantTurns" :key="index" :class="['assistant-turn', turn.role]">{{ turn.text }}</div>
              <div v-if="assistantPending" class="assistant-thinking"><i /> Reading the complete deck and preparing an edit…</div>
            </div>
            <article v-if="assistantProposal" class="assistant-proposal">
              <div><span>PROPOSED CHANGE</span><strong>{{ assistantProposal.summary || assistantProposal.message }}</strong></div>
              <details><summary>Review {{ assistantProposal.operations.length }} Markdown change{{ assistantProposal.operations.length === 1 ? '' : 's' }}</summary><section v-for="(operation, index) in assistantProposal.operations" :key="index"><small>{{ operation.kind }}</small><pre v-if="operation.old_text">{{ operation.old_text }}</pre><b>→</b><pre>{{ operation.new_text }}</pre></section></details>
              <footer><button type="button" class="button button-ghost" @click="discardAssistantProposal">Discard</button><button type="button" class="button button-primary" @click="applyAssistantProposal">Apply to deck</button></footer>
            </article>
            <p v-if="assistantError" class="assistant-error">{{ assistantError }}</p>
            <form class="assistant-compose" @submit.prevent="askSlideAssistant">
              <textarea v-model="assistantInput" maxlength="4000" :disabled="assistantPending || !!assistantProposal || deck?.is_live" placeholder="Create a slide, improve the selected content, add a poll…" aria-label="Ask the slide agent. Enter sends; Shift+Enter adds a new line." @keydown="submitAssistantOnEnter" />
              <button type="submit" :disabled="!assistantInput.trim() || assistantPending || !!assistantProposal || deck?.is_live">{{ assistantPending ? 'Working…' : 'Ask Agent' }}</button>
            </form>
          </template>
        </section>
        <div v-if="deck?.warnings.length" class="warning-strip">{{ deck.warnings[0] }}</div>
      </div>
      <div class="pane-divider" @pointerdown="beginPaneResize">
        <button
          type="button"
          :aria-label="sourceCollapsed ? 'Show Markdown editor' : 'Collapse Markdown editor'"
          :title="sourceCollapsed ? 'Show Markdown editor' : 'Collapse editor · drag divider to resize'"
          @pointerdown.stop
          @click="toggleSourcePane"
          @keydown.left.prevent="resizeSourcePane(-32)"
          @keydown.right.prevent="resizeSourcePane(32)"
        >{{ sourceCollapsed ? '›' : '‹' }}</button>
      </div>
      <div class="preview-panel">
        <div class="panel-heading">
          <span>LIVE PREVIEW</span>
          <div class="preview-tools">
            <button v-if="sourceCollapsed" class="show-source-button" @click="toggleSourcePane">Show editor</button>
            <button :disabled="previewBusy || saveState === 'saving' || saveState === 'error' || deck?.is_live" @click="updatePreview">Refresh preview</button>
          </div>
        </div>
        <div class="preview-canvas">
          <iframe v-if="previewUrl" ref="previewFrame" :src="previewUrl" title="Interdeck deck preview" allow="fullscreen; display-capture" @load="sendPreviewSource()" />
          <div v-if="previewBusy" :class="['preview-loading', { overlay: previewUrl, restarting: previewState === 'restarting' }]"><i /> Validating and refreshing preview…</div>
          <div v-else-if="previewState === 'idle' && !previewUrl" class="preview-diagnostic">
            <strong>Starting live preview</strong>
            <span>Interdeck is loading this deck into the shared, always-on player.</span>
            <button @click="updatePreview">Start now</button>
          </div>
          <div v-else-if="previewError && !previewUrl" class="preview-diagnostic">
            <strong>Slide preview could not render</strong>
            <span>{{ previewError }}</span>
            <button v-if="repairableHeadmatter" @click="repairHeadmatterTitle">Repair YAML title</button>
            <small v-else>Edit the highlighted source. The preview will update automatically after it saves.</small>
            <button v-if="deck?.can_restore_last_verified" @click="restoreLastWorking">Restore last working deck</button>
          </div>
          <div v-if="previewError && previewUrl" class="preview-build-error"><strong>Latest change not shown.</strong> {{ previewError }} <button v-if="deck?.can_restore_last_verified" @click="restoreLastWorking">Restore last working deck</button></div>
        </div>
        <div class="preview-footer">
          <span :class="['preview-health', previewState]"><i />{{ previewStatusLabel }}</span>
          <span>{{ deck?.slides.length || 0 }} slides</span>
          <span>{{ deck?.interactions.length || 0 }} {{ deck?.interactions.length === 1 ? 'interaction' : 'interactions' }}</span>
          <button class="audience-preview-link" @click="audiencePreviewOpen = true">Audience preview ↗</button>
        </div>
      </div>
    </section>

    <div v-if="assetsOpen" class="history-backdrop" @click.self="assetsOpen = false">
      <section class="asset-panel" role="dialog" aria-modal="true" aria-label="Deck assets">
        <header>
          <div><span class="eyebrow">PRIVATE DECK LIBRARY</span><h2>Images & fonts</h2></div>
          <button class="icon-button" aria-label="Close deck assets" @click="assetsOpen = false">×</button>
        </header>
        <label :class="['asset-upload', { busy: assetUploading }]">
          <input type="file" accept="image/png,image/jpeg,image/gif,image/webp,image/svg+xml,.svg,font/woff,font/woff2,.woff,.woff2" :disabled="assetUploading" @change="uploadAsset">
          <strong>{{ assetUploading ? 'Uploading…' : 'Upload an asset' }}</strong>
          <span>PNG, JPEG, GIF, WebP up to 10 MB · static SVG up to 2 MB · WOFF/WOFF2 up to 5 MB</span>
        </label>
        <div v-if="assetError" class="notice notice-error">{{ assetError }}</div>
        <div v-if="assetsLoading" class="history-loading">Loading assets…</div>
        <div v-else-if="!assets.length" class="asset-empty"><strong>No assets yet</strong><span>Upload once, then insert the stable URL anywhere in your Slidev Markdown.</span></div>
        <div v-else class="asset-grid">
          <article v-for="asset in assets" :key="asset.id">
            <div class="asset-preview">
              <img v-if="asset.media_type.startsWith('image/')" :src="asset.content_url" :alt="asset.original_filename">
              <span v-else>Aa</span>
            </div>
            <div class="asset-meta"><strong :title="asset.original_filename">{{ asset.original_filename }}</strong><span>{{ (asset.byte_size / 1024).toFixed(0) }} KB · {{ asset.media_type }}</span></div>
            <div class="asset-actions"><button class="button" @click="insertAsset(asset)">Insert</button><button title="Delete unused asset" @click="deleteAsset(asset)">×</button></div>
          </article>
        </div>
        <footer>Assets are private to this deck and retained while the current or last working Markdown uses them.</footer>
      </section>
    </div>

    <div v-if="preflightOpen" class="history-backdrop" @click.self="preflightOpen = false">
      <section class="preflight-panel" role="dialog" aria-modal="true" aria-labelledby="preflight-title">
        <header>
          <div><span class="eyebrow">BEFORE YOU GO LIVE</span><h2 id="preflight-title">Presenter preflight</h2></div>
          <button class="icon-button" aria-label="Close presenter preflight" @click="preflightOpen = false">×</button>
        </header>
        <div class="preflight-body">
          <div class="preflight-room">
            <QrcodeVue v-if="joinUrl" :value="joinUrl" :size="180" level="M" render-as="svg" />
            <span>ROOM</span>
            <strong>{{ deck?.join_code }}</strong>
            <a :href="joinUrl" target="_blank" rel="noopener">Open audience view ↗</a>
            <small>Scan this from the projected screen before starting.</small>
          </div>
          <ol class="preflight-checks">
            <li v-for="check in presenterChecks" :key="check.id" :class="check.status">
              <i aria-hidden="true">{{ check.status === 'pass' ? '✓' : check.status === 'warn' ? '!' : '×' }}</i>
              <div><strong>{{ check.label }}</strong><span>{{ check.detail }}</span></div>
            </li>
          </ol>
        </div>
        <footer>
          <span>Present resumes epoch {{ deck?.result_epoch }} and opens a private presenter console. Share only the presentation window; keep moderation and controls in the console.</span>
          <div><button class="button button-ghost" @click="preflightOpen = false">Back to editor</button><button class="button button-primary" :disabled="!presenterReady" @click="confirmPresent">Start presenting ▶</button></div>
        </footer>
      </section>
    </div>

    <div v-if="audiencePreviewOpen" class="history-backdrop" @click.self="audiencePreviewOpen = false">
      <section class="audience-preview-panel" role="dialog" aria-modal="true" aria-labelledby="audience-preview-title">
        <header>
          <div><span class="eyebrow">PHONE PREVIEW</span><h2 id="audience-preview-title">What the audience sees</h2></div>
          <button class="icon-button" aria-label="Close audience preview" @click="audiencePreviewOpen = false">×</button>
        </header>
        <div class="phone-frame"><iframe :src="joinUrl" title="Audience phone preview" /></div>
        <footer><span>Room {{ deck?.join_code }} · this uses the real audience route.</span><a :href="joinUrl" target="_blank" rel="noopener">Open in a new tab ↗</a></footer>
      </section>
    </div>
  </main>
</template>
