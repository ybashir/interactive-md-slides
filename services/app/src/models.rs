use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub picture_url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeckSummary {
    pub id: Uuid,
    pub title: String,
    pub join_code: String,
    pub version: i64,
    pub updated_at: DateTime<Utc>,
    pub is_live: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeckDetail {
    pub id: Uuid,
    pub title: String,
    pub join_code: String,
    pub markdown: String,
    pub css: String,
    pub version: i64,
    pub verified_version: Option<i64>,
    pub is_verified: bool,
    pub can_restore_last_verified: bool,
    pub result_epoch: i32,
    pub qa_display_mode: String,
    pub slides: Vec<ParsedSlide>,
    pub interactions: Vec<InteractionDefinition>,
    pub warnings: Vec<String>,
    pub is_live: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeckAsset {
    pub id: Uuid,
    pub deck_id: Uuid,
    pub original_filename: String,
    pub media_type: String,
    pub byte_size: i64,
    pub sha256: String,
    pub content_url: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ParsedSlide {
    pub key: String,
    pub number: i32,
    pub title: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InteractionOption {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InteractionDefinition {
    pub id: String,
    pub slide_key: String,
    pub slide_number: i32,
    pub kind: String,
    pub title: String,
    pub options: Vec<InteractionOption>,
    pub config: Value,
    pub element: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct LiveState {
    pub live: bool,
    pub run_id: Option<Uuid>,
    pub run_mode: Option<String>,
    pub input_frozen: bool,
    pub result_epoch: i32,
    pub deck_id: Uuid,
    pub deck_title: String,
    pub join_code: String,
    pub slide_key: Option<String>,
    pub slide_number: Option<i32>,
    pub click_step: i32,
    pub sequence: i64,
    pub interactions: Vec<AudienceInteraction>,
    pub questions: Vec<QuestionView>,
    pub ai_configured: bool,
    pub ai_moderation_mode: Option<String>,
    pub qa_display_mode: String,
    pub ai_insights: Option<DeckInsightView>,
    pub ai_insights_status: Option<String>,
    pub participant_count: i64,
    pub audience_session_active: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct LiveSummary {
    pub deck_id: Uuid,
    pub result_epoch: i32,
    pub generated_at: DateTime<Utc>,
    pub participant_count: i64,
    pub response_count: i64,
    pub interactions: Vec<LiveSummaryInteraction>,
    pub questions: LiveSummaryQuestions,
}

#[derive(Clone, Debug, Serialize)]
pub struct LiveSummaryInteraction {
    pub id: String,
    pub slide_key: String,
    pub slide_number: i32,
    pub kind: String,
    pub title: String,
    pub response_count: i64,
    pub headline: String,
    pub highlights: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LiveSummaryQuestions {
    pub submitted: i64,
    pub approved: i64,
    pub pending: i64,
    pub rejected: i64,
    pub answered: i64,
    pub total_votes: i64,
    pub top: Vec<LiveSummaryQuestion>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LiveSummaryQuestion {
    pub id: Uuid,
    pub body: String,
    pub display_name: Option<String>,
    pub slide_number: Option<i32>,
    pub votes: i64,
    pub is_pinned: bool,
    pub lifecycle_status: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AudienceInteraction {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub options: Vec<InteractionOption>,
    pub config: Value,
    pub element: bool,
    pub phase: Option<String>,
    pub revealed_count: Option<i32>,
    pub accepting_responses: bool,
    pub presenter_closed: bool,
    pub results_revealed: bool,
    pub timer_running: bool,
    pub timer_remaining_seconds: Option<i32>,
    pub result: Value,
    pub response: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
pub struct QuestionView {
    pub id: Uuid,
    pub body: String,
    pub display_name: Option<String>,
    pub votes: i64,
    pub moderation_status: String,
    pub lifecycle_status: String,
    pub is_pinned: bool,
    pub slide_key: Option<String>,
    pub slide_number: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moderation_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moderation_labels: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moderation_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_novel: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority_score: Option<i64>,
    pub mine: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeckInsightView {
    pub summary: String,
    pub themes: Value,
    pub suggested_answers: Value,
    pub question_count: i32,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDeckRequest {
    pub title: String,
    pub markdown: Option<String>,
    pub sample: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDeckRequest {
    pub title: Option<String>,
    pub markdown: String,
    pub css: Option<String>,
    pub expected_version: i64,
}

#[derive(Debug, Serialize)]
pub struct DeckDraftSave {
    pub version: i64,
    pub verified_version: Option<i64>,
    pub is_verified: bool,
}

#[derive(Debug, Deserialize)]
pub struct RestoreLastVerifiedRequest {
    pub expected_version: i64,
}

#[derive(Debug, Deserialize)]
pub struct QaSettingsRequest {
    pub display_mode: String,
}

#[derive(Debug, Deserialize)]
pub struct RecordDeckBuildRequest {
    pub source_version: i64,
    pub status: String,
    pub slidev_version: Option<String>,
    pub theme: Option<String>,
    pub startup_ms: Option<i32>,
    pub diagnostic_code: Option<String>,
    pub diagnostic_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NavigateRequest {
    pub slide_number: i32,
    #[serde(default)]
    pub click_step: i32,
    #[serde(default)]
    pub visible_element_interaction_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct InteractionControlRequest {
    pub action: String,
}

#[derive(Debug, Deserialize)]
pub struct InputControlRequest {
    pub action: String,
}

#[derive(Debug, Deserialize)]
pub struct JoinRequest {
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseRequest {
    pub payload: Value,
    pub idempotency_key: String,
}

#[derive(Debug, Deserialize)]
pub struct QuestionRequest {
    pub body: String,
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ModerateQuestionRequest {
    pub moderation_status: Option<String>,
    pub lifecycle_status: Option<String>,
    pub is_pinned: Option<bool>,
}
