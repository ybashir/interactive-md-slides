<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { api } from '../api'
const config = ref<{ ai_moderation_enabled?: boolean; ai_assistant_enabled?: boolean; audience_retention_days?: number }>({})
onMounted(async () => { try { config.value = await api('/api/config') } catch {} })
</script>

<template>
  <main class="privacy-page">
    <a href="/">← Interdeck</a>
    <h1>Privacy and your data</h1>
    <p>This Interdeck instance is operated by the person or organization that invited you. Contact them about access, deletion, or their additional privacy terms.</p>
    <h2>Creators</h2>
    <p>Google sign-in provides your verified email, name, and profile image. The server stores those details, your decks, uploaded assets, and a session cookie that expires after seven days.</p>
    <h2>Audience participation</h2>
    <p>Your name is optional. Responses and questions are stored against a random participant identifier, even when you join anonymously. The presenter can see results and download responses. Approved questions and results that the presenter reveals may be shown to the room. Avoid sharing personal or confidential information in free-text answers.</p>
    <p>The audience cookie expires after one day. No advertising or tracking cookies are used by Interdeck.</p>
    <h2>AI features</h2>
    <p v-if="config.ai_moderation_enabled">AI question moderation is enabled here. Submitted questions and related deck topics may be sent to Google Gemini for labels, duplicate suggestions, or summaries. Your submitted name is not included in that request, but the text you write may contain personal information.</p>
    <p v-else-if="config.ai_moderation_enabled === false">AI question moderation is disabled here. Interdeck does not send audience questions to an AI provider.</p>
    <p v-else>AI settings could not be loaded. Ask the administrator whether AI processing is enabled before submitting sensitive text.</p>
    <p v-if="config.ai_assistant_enabled">The creator's optional slide assistant sends the instruction, recent conversation, relevant deck Markdown, and asset names/URLs to Google Gemini when the creator requests a proposal. The creator chooses whether to apply it.</p>
    <h2>Retention and deletion</h2>
    <p v-if="config.audience_retention_days">On inactive decks, audience records are removed after {{ config.audience_retention_days }} days without participant activity. AI summaries older than that period are also removed. Active presentations are protected from cleanup.</p>
    <p v-else-if="config.audience_retention_days === 0">Automatic audience-data deletion is disabled here. Records remain until the deck owner deletes the deck or the administrator removes them.</p>
    <p v-else>Retention settings could not be loaded. Ask the administrator for this instance's retention policy.</p>
    <p>Deleting a deck removes its stored responses, questions, and metadata. Uploaded files are queued for deletion with retries if storage is unavailable. Backups and provider-side retention are controlled separately by the administrator and Google.</p>
  </main>
</template>

<style scoped>
.privacy-page { max-width: 760px; margin: 0 auto; padding: 40px 24px; line-height: 1.7; }
.privacy-page h1, .privacy-page h2 { margin-top: 24px; }
</style>
