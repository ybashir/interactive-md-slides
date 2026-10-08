export interface User {
  id: string
  email: string
  display_name: string
  picture_url?: string
}

export interface DeckSummary {
  id: string
  title: string
  join_code: string
  version: number
  updated_at: string
  is_live: boolean
}

export interface ParsedSlide {
  key: string
  number: number
  title: string
}

export interface InteractionOption {
  id: string
  label: string
  image_url?: string
}

export interface InteractionDefinition {
  id: string
  slide_key: string
  slide_number: number
  kind: string
  title: string
  options: InteractionOption[]
  config: Record<string, any>
  element: boolean
}

export interface DeckDetail extends DeckSummary {
  markdown: string
  css: string
  verified_version: number | null
  is_verified: boolean
  can_restore_last_verified: boolean
  result_epoch: number
  qa_display_mode: 'verbatim' | 'ai_grouped'
  slides: ParsedSlide[]
  interactions: InteractionDefinition[]
  warnings: string[]
}

export interface DeckAsset {
  id: string
  deck_id: string
  original_filename: string
  media_type: string
  byte_size: number
  sha256: string
  content_url: string
  created_at: string
}

export interface AudienceInteraction extends InteractionDefinition {
  phase?: 'revealing' | 'voting' | 'ranked'
  revealed_count?: number
  accepting_responses: boolean
  presenter_closed: boolean
  results_revealed: boolean
  timer_running: boolean
  timer_remaining_seconds?: number
  result: Record<string, any>
  response?: Record<string, any>
}

export interface Question {
  id: string
  body: string
  display_name?: string
  votes: number
  moderation_status: 'pending' | 'approved' | 'rejected'
  lifecycle_status: 'open' | 'answered' | 'archived'
  is_pinned: boolean
  slide_key?: string
  slide_number?: number
  analysis_status?: 'not_requested' | 'queued' | 'complete' | 'failed'
  moderation_source?: 'human' | 'ai'
  moderation_labels?: {
    safety_flagged?: boolean
    spam?: boolean
    contains_pii?: boolean
    off_topic?: boolean
  }
  moderation_reason?: string
  topic?: string
  duplicate_of?: string
  is_novel?: boolean
  repeat_count?: number
  priority_score?: number
  mine: boolean
  created_at: string
}

export interface DeckInsight {
  summary: string
  themes: Array<{
    label: string
    summary: string
    representative_question?: string
    question_ids: string[]
    questions?: Array<{
      id: string
      body: string
      asker: string
      scope: 'deck' | 'slide'
      slide_number?: number
    }>
  }>
  suggested_answers: string[]
  question_count: number
  generated_at: string
}

export interface LiveState {
  live: boolean
  run_id?: string
  run_mode?: 'live' | 'rehearsal'
  input_frozen: boolean
  result_epoch: number
  deck_id: string
  deck_title: string
  join_code: string
  slide_key?: string
  slide_number?: number
  click_step: number
  sequence: number
  interactions: AudienceInteraction[]
  questions: Question[]
  ai_configured: boolean
  ai_moderation_mode?: 'assist' | 'enforce'
  qa_display_mode: 'verbatim' | 'ai_grouped'
  ai_insights?: DeckInsight
  ai_insights_status?: 'queued' | 'running' | 'succeeded' | 'failed'
  participant_count: number
  audience_session_active: boolean
}

export interface LiveSummaryInteraction {
  id: string
  slide_key: string
  slide_number: number
  kind: string
  title: string
  response_count: number
  headline: string
  highlights: string[]
}

export interface LiveSummaryQuestion {
  id: string
  body: string
  display_name?: string
  slide_number?: number
  votes: number
  is_pinned: boolean
  lifecycle_status: 'open' | 'answered'
}

export interface LiveSummary {
  deck_id: string
  result_epoch: number
  generated_at: string
  participant_count: number
  response_count: number
  interactions: LiveSummaryInteraction[]
  questions: {
    submitted: number
    approved: number
    pending: number
    rejected: number
    answered: number
    total_votes: number
    top: LiveSummaryQuestion[]
  }
}
