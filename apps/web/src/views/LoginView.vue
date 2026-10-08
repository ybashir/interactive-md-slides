<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { api } from '../api'
const loginUrl = `/api/auth/google/start?return_to=${encodeURIComponent('/')}`
const workspace = ref('')
onMounted(async () => { try { workspace.value = (await api<{ workspace_domain: string }>('/api/config')).workspace_domain } catch {} })
</script>

<template>
  <main class="login-page">
    <section class="login-panel">
      <div class="brand-mark" aria-hidden="true"><span /><span /><span /></div>
      <p class="eyebrow">INTERDECK · LIVE PRESENTATIONS</p>
      <h1>Ideas deserve<br><em>a live room.</em></h1>
      <p class="login-copy">Build expressive Slidev decks in Markdown, then turn every slide into a conversation.</p>
      <a class="button button-primary button-large" :href="loginUrl">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M21.35 11.1h-9.18v2.98h5.28c-.25 1.53-1.8 4.48-5.28 4.48A6.58 6.58 0 0 1 5.6 12a6.58 6.58 0 0 1 6.57-6.56c1.98 0 3.3.84 4.05 1.57l2.77-2.67A9.57 9.57 0 0 0 12.17 1.7 10.3 10.3 0 1 0 22.5 12c0-.69-.08-1.2-.15-1.7Z" /></svg>
        Continue with Google
      </a>
      <p class="microcopy">{{ workspace ? `Sign in with your ${workspace} Workspace account.` : 'Use a Google account permitted by this server’s administrator.' }}</p>
      <p class="microcopy"><a href="/privacy">Privacy and your data</a></p>
    </section>
    <aside class="login-art" aria-label="Live presentation illustration">
      <div class="art-card art-card-main">
        <span class="live-pill"><i /> LIVE · 684</span>
        <h2>What should we<br>build next?</h2>
        <div class="mock-result"><span style="--width: 78%">Developer experience</span><b>48%</b></div>
        <div class="mock-result"><span style="--width: 52%">Delivery speed</span><b>31%</b></div>
        <div class="mock-result"><span style="--width: 34%">Reliability</span><b>21%</b></div>
      </div>
      <div class="art-card art-card-float"><strong>▲ 126</strong><span>Ship smaller changes</span></div>
      <div class="art-orbit" />
    </aside>
  </main>
</template>
