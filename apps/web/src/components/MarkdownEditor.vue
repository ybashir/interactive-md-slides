<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import * as monaco from 'monaco-editor/editor/editor.api'
import 'monaco-editor/languages/definitions/markdown/register'
import 'monaco-editor/language/css/monaco.contribution'
import EditorWorker from 'monaco-editor/editor/editor.worker?worker'
import type { SourceDiagnostic } from '../source-diagnostics.mjs'

const props = defineProps<{
  modelValue: string
  disabled?: boolean
  diagnostics?: SourceDiagnostic[]
  language?: 'markdown' | 'css'
  ariaLabel?: string
}>()
const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

const container = ref<HTMLElement | null>(null)
let instance: monaco.editor.IStandaloneCodeEditor | null = null
let model: monaco.editor.ITextModel | null = null
let changeSubscription: monaco.IDisposable | null = null
let applyingExternalValue = false

onMounted(() => {
  if (!container.value) return
  ;(globalThis as typeof globalThis & { MonacoEnvironment?: { getWorker: () => Worker } }).MonacoEnvironment = {
    getWorker: () => new EditorWorker(),
  }
  monaco.editor.defineTheme('interdeck-dark', {
    base: 'vs-dark',
    inherit: true,
    rules: [
      { token: 'keyword', foreground: 'A99CFF' },
      { token: 'string', foreground: 'D6B271' },
    ],
    colors: {
      'editor.background': '#1A1F2B',
      'editor.foreground': '#D8DCE7',
      'editorCursor.foreground': '#A99CFF',
      'editorLineNumber.foreground': '#596171',
      'editorLineNumber.activeForeground': '#AAB0BE',
      'editor.selectionBackground': '#6252CC66',
      'editor.inactiveSelectionBackground': '#6252CC35',
    },
  })
  model = monaco.editor.createModel(props.modelValue, props.language || 'markdown')
  instance = monaco.editor.create(container.value, {
    model,
    theme: 'interdeck-dark',
    ariaLabel: props.ariaLabel || 'Slidev Markdown editor',
    automaticLayout: true,
    readOnly: props.disabled,
    wordWrap: 'on',
    wrappingIndent: 'same',
    minimap: { enabled: false },
    overviewRulerBorder: false,
    overviewRulerLanes: 0,
    hideCursorInOverviewRuler: true,
    scrollBeyondLastLine: false,
    renderLineHighlight: 'line',
    lineNumbersMinChars: 3,
    fontFamily: "'DM Mono', ui-monospace, monospace",
    fontSize: 14,
    lineHeight: 23,
    padding: { top: 20, bottom: 20 },
    tabSize: 2,
    insertSpaces: true,
    bracketPairColorization: { enabled: true },
    guides: { bracketPairs: true, indentation: false },
    accessibilitySupport: 'auto',
  })
  changeSubscription = model.onDidChangeContent(() => {
    if (!applyingExternalValue) emit('update:modelValue', model?.getValue() || '')
  })
  applyDiagnostics(props.diagnostics)
})

watch(() => props.modelValue, (value) => {
  syncValue(value)
})

watch(() => props.disabled, disabled => instance?.updateOptions({ readOnly: disabled }))
watch(() => props.diagnostics, applyDiagnostics, { deep: true })
watch(() => props.language, language => {
  if (model) monaco.editor.setModelLanguage(model, language || 'markdown')
})

onBeforeUnmount(() => {
  changeSubscription?.dispose()
  instance?.dispose()
  model?.dispose()
})

function insertText(source: string) {
  if (!instance || !model || props.disabled) return false
  const selection = instance.getSelection()
  if (!selection) return false
  instance.pushUndoStop()
  instance.executeEdits('interdeck-snippet', [{ range: selection, text: source, forceMoveMarkers: true }])
  instance.pushUndoStop()
  instance.focus()
  return true
}

function replaceAll(source: string) {
  if (!instance || !model || props.disabled) return false
  instance.pushUndoStop()
  instance.executeEdits('interdeck-slide-assistant', [{ range: model.getFullModelRange(), text: source, forceMoveMarkers: true }])
  instance.pushUndoStop()
  instance.focus()
  return true
}

function syncValue(source: string) {
  if (!model) return false
  if (model.getValue() === source) return true
  applyingExternalValue = true
  try {
    model.setValue(source)
  }
  finally {
    applyingExternalValue = false
  }
  return true
}

function getContext() {
  if (!instance || !model) return { selectedText: '', currentLine: 1 }
  const selection = instance.getSelection()
  return {
    selectedText: selection ? model.getValueInRange(selection) : '',
    currentLine: instance.getPosition()?.lineNumber || 1,
  }
}

function applyDiagnostics(diagnostics: SourceDiagnostic[] = []) {
  if (!model) return
  monaco.editor.setModelMarkers(model, 'interdeck', diagnostics.map(diagnostic => ({
    message: diagnostic.message,
    severity: diagnostic.severity === 'warning' ? monaco.MarkerSeverity.Warning : monaco.MarkerSeverity.Error,
    startLineNumber: diagnostic.startLine,
    startColumn: diagnostic.startColumn,
    endLineNumber: diagnostic.endLine,
    endColumn: diagnostic.endColumn,
  })))
}

defineExpose({ insertText, replaceAll, syncValue, getContext })
</script>

<template>
  <div ref="container" class="markdown-editor" />
</template>
