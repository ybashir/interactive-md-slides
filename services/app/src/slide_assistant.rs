use std::{
    collections::HashSet,
    sync::{Arc, LazyLock},
    time::Duration,
};

use anyhow::{Context, anyhow};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::{
    auth::require_user,
    error::{AppError, AppResult},
    markdown::{
        normalize_marked_slide_boundaries, parse_deck, repair_duplicate_slide_markers,
        repair_redundant_slide_markers, repair_unquoted_headmatter_title, validate_deck_headmatter,
    },
    state::AppState,
};

const SKILL_PACK: &str = include_str!("../prompts/slidev_assistant.md");
const PROMPT_VERSION: &str = "slidev-author-v9-focused-frontmatter";
const REFUSAL: &str = "I can only help create or update this slide deck.";
const GEMINI_MAX_ATTEMPTS: usize = 2;
static PRIVATE_ASSET_REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"/api/decks/[^\s\"'()<>]+/assets/[^\s\"'()<>]+/content"#).unwrap()
});
static CONVENTIONAL_ASSET_REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(?:(?:src|href|image)\s*=\s*[\"']|\]\(|url\(\s*[\"']?)/?assets/[^\s\"'()<>]+"#,
    )
    .unwrap()
});
static ASSISTANT_SLIDE_MARKER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^<!--\s*interdeck-slide:\s*([a-zA-Z0-9][a-zA-Z0-9_-]*)\s*-->[ \t]*(?:\r?\n|$)")
        .unwrap()
});
static SLIDE_DELIMITER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^---[ \t]*$").unwrap());

#[derive(Debug, Deserialize)]
pub struct SlideAssistantRequest {
    instruction: String,
    expected_version: i64,
    #[serde(default)]
    history: Vec<ConversationTurn>,
    selected_text: Option<String>,
    current_line: Option<i32>,
    current_slide_number: Option<i32>,
    editor_scope: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ConversationTurn {
    role: String,
    text: String,
}

#[derive(Debug, Serialize)]
struct SlideAssistantAsset {
    original_filename: String,
    media_type: String,
    content_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EditOperation {
    kind: String,
    old_text: String,
    new_text: String,
}

#[derive(Debug, Deserialize)]
struct GeminiProposal {
    action: String,
    message: String,
    summary: String,
    focus_slide_key: Option<String>,
    operations: Vec<EditOperation>,
}

#[derive(Debug, Serialize)]
pub struct SlideAssistantResponse {
    proposal_id: Uuid,
    action: String,
    message: String,
    summary: String,
    focus_slide_key: Option<String>,
    operations: Vec<EditOperation>,
    proposed_markdown: Option<String>,
    base_version: i64,
    source_hash: String,
}

pub async fn propose(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<SlideAssistantRequest>,
) -> AppResult<Json<SlideAssistantResponse>> {
    let user = require_user(&state, &headers).await?;
    let instruction = request.instruction.trim();
    if instruction.chars().count() < 2 || instruction.chars().count() > 4_000 {
        return Err(AppError::BadRequest(
            "Slide request must be between 2 and 4,000 characters".to_owned(),
        ));
    }
    if request.history.len() > 12 {
        return Err(AppError::BadRequest(
            "Slide assistant history is limited to 12 recent messages".to_owned(),
        ));
    }
    if request.history.iter().any(|turn| {
        !matches!(turn.role.as_str(), "user" | "assistant") || turn.text.chars().count() > 1_500
    }) {
        return Err(AppError::BadRequest(
            "Slide assistant history is invalid or too large".to_owned(),
        ));
    }
    if request
        .selected_text
        .as_deref()
        .is_some_and(|text| text.len() > 8_000)
    {
        return Err(AppError::BadRequest(
            "The selected Markdown is too large for one assistant request".to_owned(),
        ));
    }
    if request
        .editor_scope
        .as_deref()
        .is_some_and(|scope| !matches!(scope, "slide" | "markdown" | "css"))
    {
        return Err(AppError::BadRequest(
            "The slide assistant editor scope is invalid".to_owned(),
        ));
    }
    state
        .check_rate_limit("slide-assistant", user.id, 30, Duration::from_secs(60 * 60))
        .map_err(AppError::RateLimited)?;
    let _request_guard = state
        .begin_assistant_request(user.id, deck_id)
        .ok_or_else(|| {
            AppError::Conflict(
                "A slide assistant request is already running for this deck".to_owned(),
            )
        })?;
    let api_key = state
        .config
        .gemini_assistant_key
        .as_deref()
        .ok_or_else(|| {
            AppError::BadRequest("The Gemini slide assistant is not configured".to_owned())
        })?;

    let row = sqlx::query(
        r#"
        SELECT d.markdown, d.version,
               EXISTS(SELECT 1 FROM presentation_runs r WHERE r.deck_id = d.id AND r.status = 'live') AS is_live
        FROM decks d
        WHERE d.id = $1 AND d.owner_id = $2
        "#,
    )
    .bind(deck_id)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let markdown: String = row.try_get("markdown")?;
    let version: i64 = row.try_get("version")?;
    let is_live: bool = row.try_get("is_live")?;
    if is_live {
        return Err(AppError::Conflict(
            "End the presentation before asking Gemini to edit this deck".to_owned(),
        ));
    }
    if version != request.expected_version {
        return Err(AppError::Conflict(
            "The deck changed before Gemini started. Save and ask again".to_owned(),
        ));
    }

    let parsed_deck = parse_deck(&markdown).map_err(AppError::BadRequest)?;
    let current_slide_number = request.current_slide_number.unwrap_or(1);
    let current_slide = parsed_deck
        .slides
        .iter()
        .find(|slide| slide.number == current_slide_number)
        .ok_or_else(|| {
            AppError::BadRequest("The current preview slide no longer exists".to_owned())
        })?;
    let current_slide_markdown = extract_marked_slide(&markdown, &current_slide.key);
    let editor_scope = request.editor_scope.as_deref().unwrap_or("markdown");
    let focused_slide_context = editor_scope == "slide";
    let deck_headmatter = extract_deck_headmatter(&markdown);
    let deck_outline = parsed_deck
        .slides
        .iter()
        .map(|slide| {
            json!({
                "number": slide.number,
                "stable_key": slide.key,
                "title": slide.title,
            })
        })
        .collect::<Vec<_>>();
    let interaction_catalog = parsed_deck
        .interactions
        .iter()
        .map(|interaction| {
            json!({
                "id": interaction.id,
                "slide_key": interaction.slide_key,
                "kind": interaction.kind,
                "element": interaction.element,
            })
        })
        .collect::<Vec<_>>();

    let deck_assets = sqlx::query(
        r#"
        SELECT id, original_filename, media_type
        FROM deck_assets
        WHERE deck_id = $1
        ORDER BY original_filename, created_at
        "#,
    )
    .bind(deck_id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|row| {
        let asset_id: Uuid = row.try_get("id")?;
        Ok(SlideAssistantAsset {
            original_filename: row.try_get("original_filename")?,
            media_type: row.try_get("media_type")?,
            content_url: format!("/api/decks/{deck_id}/assets/{asset_id}/content"),
        })
    })
    .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let allowed_asset_urls = deck_assets
        .iter()
        .map(|asset| asset.content_url.clone())
        .collect::<HashSet<_>>();

    let source_hash = format!("{:x}", Sha256::digest(markdown.as_bytes()));
    let input = json!({
        "current_user_request": instruction,
        "recent_conversation": request.history,
        "context_mode": if focused_slide_context { "focused_slide" } else { "full_deck" },
        "editor_context": {
            "selected_text": request.selected_text.unwrap_or_default(),
            "current_line": request.current_line,
            "editor_scope": editor_scope,
            "current_slide": {
                "number": current_slide.number,
                "stable_key": current_slide.key,
                "title": current_slide.title,
                "canonical_markdown": current_slide_markdown,
            },
        },
        "deck_headmatter": deck_headmatter,
        "deck_outline": deck_outline,
        "interaction_catalog": interaction_catalog,
        "deck_asset_catalog": deck_assets,
        "canonical_deck_markdown": if focused_slide_context {
            Value::Null
        } else {
            Value::String(markdown.clone())
        },
    });
    let started = std::time::Instant::now();
    let mut proposal = request_gemini(&state, api_key, &input, focused_slide_context).await?;
    normalize_proposal(&mut proposal)?;
    let (
        proposed_markdown,
        repaired_slide_ids,
        redundant_slide_markers,
        normalized_slide_boundaries,
        repaired_headmatter,
    ) = if proposal.action == "propose_patch" {
        let proposed = apply_operations(&markdown, &proposal.operations)?;
        let (proposed, repaired_headmatter) = repair_unquoted_headmatter_title(&proposed);
        let (proposed, normalized_slide_boundaries) = normalize_marked_slide_boundaries(&proposed);
        let (proposed, redundant_slide_markers) = repair_redundant_slide_markers(&proposed);
        let (repaired, repairs) = repair_duplicate_slide_markers(&proposed);
        (
            Some(repaired),
            repairs,
            redundant_slide_markers,
            normalized_slide_boundaries,
            repaired_headmatter,
        )
    } else {
        (None, Vec::new(), 0, 0, false)
    };
    if !repaired_slide_ids.is_empty()
        || redundant_slide_markers > 0
        || normalized_slide_boundaries > 0
        || repaired_headmatter
    {
        let count = repaired_slide_ids.len();
        let mut notes = Vec::new();
        if count > 0 {
            notes.push(format!(
                "Assigned unique stable IDs to {count} duplicate slide marker{}.",
                if count == 1 { "" } else { "s" }
            ));
        }
        if repaired_headmatter {
            notes.push("Quoted the deck title so its YAML remains valid.".to_owned());
        }
        if redundant_slide_markers > 0 {
            notes.push(format!(
                "Removed {redundant_slide_markers} redundant slide marker{}.",
                if redundant_slide_markers == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }
        if normalized_slide_boundaries > 0 {
            notes.push(format!(
                "Normalized {normalized_slide_boundaries} slide {} so Slidev keeps every slide separate.",
                if normalized_slide_boundaries == 1 {
                    "boundary"
                } else {
                    "boundaries"
                }
            ));
        }
        let note = notes.join(" ");
        proposal.summary = truncate(
            &format!(
                "{}{}{}",
                proposal.summary,
                if proposal.summary.is_empty() { "" } else { " " },
                note
            ),
            500,
        );
        tracing::info!(
            deck_id = %deck_id,
            repair_count = count,
            redundant_slide_markers,
            normalized_slide_boundaries,
            repaired_headmatter,
            "Repaired generated Slidev metadata in a Gemini proposal"
        );
    }
    if let Some(source) = &proposed_markdown {
        validate_proposed_asset_references(source, &allowed_asset_urls)?;
        let expected_slide_count = validate_proposed_markdown(source)?;
        validate_with_slidev_parser(&state, deck_id, source, expected_slide_count).await?;
    }
    let proposal_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO audit_events (id, deck_id, actor_user_id, action, data) VALUES ($1, $2, $3, 'slide_assistant.proposed', $4)",
    )
    .bind(proposal_id)
    .bind(deck_id)
    .bind(user.id)
    .bind(json!({
        "provider": "gemini",
        "model": state.config.gemini_slide_model,
        "prompt_version": PROMPT_VERSION,
        "base_version": version,
        "source_hash": source_hash,
        "result_action": proposal.action,
        "operation_count": proposal.operations.len(),
        "available_asset_count": allowed_asset_urls.len(),
        "current_slide_number": current_slide.number,
        "current_slide_key": current_slide.key,
        "context_mode": if focused_slide_context { "focused_slide" } else { "full_deck" },
        "repaired_duplicate_slide_ids": repaired_slide_ids.len(),
        "removed_redundant_slide_markers": redundant_slide_markers,
        "normalized_slide_boundaries": normalized_slide_boundaries,
        "repaired_headmatter": repaired_headmatter,
        "latency_ms": started.elapsed().as_millis().min(i64::MAX as u128) as i64,
    }))
    .execute(&state.db)
    .await?;

    Ok(Json(SlideAssistantResponse {
        proposal_id,
        action: proposal.action,
        message: proposal.message,
        summary: proposal.summary,
        focus_slide_key: proposal.focus_slide_key,
        operations: proposal.operations,
        proposed_markdown,
        base_version: version,
        source_hash,
    }))
}

async fn request_gemini(
    state: &AppState,
    api_key: &str,
    input: &Value,
    focused_slide_context: bool,
) -> AppResult<GeminiProposal> {
    let endpoint = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
        state.config.gemini_slide_model
    );
    let timeout = if focused_slide_context {
        Duration::from_secs(45)
    } else {
        Duration::from_secs(60)
    };
    let request_body = json!({
        "system_instruction": { "parts": [{ "text": SKILL_PACK }] },
        "contents": [{ "role": "user", "parts": [{ "text": input.to_string() }] }],
        "generationConfig": proposal_generation_config(if focused_slide_context { "low" } else { "medium" })
    });
    let mut attempt = 0_usize;
    let response = loop {
        attempt += 1;
        match state
            .http
            .post(&endpoint)
            .header("x-goog-api-key", api_key)
            .timeout(timeout)
            .json(&request_body)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => break response,
            Ok(response) => {
                let status = response.status();
                let details = response.text().await.unwrap_or_default();
                let transient =
                    status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS;
                if transient && attempt < GEMINI_MAX_ATTEMPTS {
                    tracing::warn!(
                        attempt,
                        provider_status = status.as_u16(),
                        provider_error = %provider_error_message(&details),
                        "Retrying a transient Gemini slide assistant failure"
                    );
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    continue;
                }
                tracing::warn!(
                    attempt,
                    provider_status = status.as_u16(),
                    provider_error = %provider_error_message(&details),
                    "Gemini rejected a slide assistant request"
                );
                return Err(AppError::Upstream(
                    "Gemini could not process this slide request. Your deck was not modified. Try again or ask for a smaller change."
                        .to_owned(),
                ));
            }
            Err(error) => {
                let transient = error.is_timeout() || error.is_connect();
                if transient && attempt < GEMINI_MAX_ATTEMPTS {
                    tracing::warn!(
                        attempt,
                        ?error,
                        "Retrying a transient Gemini connection failure"
                    );
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    continue;
                }
                tracing::warn!(attempt, ?error, "Gemini slide assistant request failed");
                let message = if error.is_timeout() {
                    "The Agent timed out before making any changes. Your deck was not modified. Try again."
                } else {
                    "The Agent could not reach Gemini. Your deck was not modified. Try again."
                };
                return Err(AppError::Upstream(message.to_owned()));
            }
        }
    };
    let value: Value = response
        .json()
        .await
        .context("Gemini returned an invalid response")?;
    let text =
        extract_output_text(&value).ok_or_else(|| anyhow!("Gemini returned no slide proposal"))?;
    serde_json::from_str(&text)
        .context("Gemini returned an invalid structured slide proposal")
        .map_err(AppError::Internal)
}

fn extract_marked_slide(markdown: &str, slide_key: &str) -> String {
    let markers = ASSISTANT_SLIDE_MARKER
        .captures_iter(markdown)
        .collect::<Vec<_>>();
    let Some(index) = markers
        .iter()
        .position(|captures| captures.get(1).is_some_and(|key| key.as_str() == slide_key))
    else {
        return String::new();
    };
    let Some(current) = markers[index].get(0) else {
        return String::new();
    };
    let start = if index == 0 {
        current.start()
    } else {
        let range_start = markers[index - 1]
            .get(0)
            .map(|previous| previous.end())
            .unwrap_or(0);
        let candidate = boundary_before_marker(markdown, range_start, current.start());
        let delimiter_count = SLIDE_DELIMITER
            .find_iter(&markdown[candidate..current.start()])
            .count();
        if delimiter_count >= 2 {
            candidate
        } else {
            current.start()
        }
    };
    let end = markers
        .get(index + 1)
        .and_then(|captures| captures.get(0))
        .map(|next| boundary_before_marker(markdown, current.end(), next.start()))
        .unwrap_or(markdown.len());
    truncate(&markdown[start..end], 20_000)
}

fn extract_deck_headmatter(markdown: &str) -> String {
    let normalized = markdown.replace("\r\n", "\n");
    if !normalized.starts_with("---\n") {
        return String::new();
    }
    let Some(closing) = normalized[4..].find("\n---") else {
        return String::new();
    };
    let end = 4 + closing + 4;
    normalized[..end].to_owned()
}

fn boundary_before_marker(markdown: &str, range_start: usize, marker_start: usize) -> usize {
    let region = &markdown[range_start..marker_start];
    let delimiters = SLIDE_DELIMITER.find_iter(region).collect::<Vec<_>>();
    let Some(closing) = delimiters.last() else {
        return marker_start;
    };
    if !region[closing.end()..].trim().is_empty() {
        return marker_start;
    }
    if let Some(opening) = delimiters
        .len()
        .checked_sub(2)
        .and_then(|index| delimiters.get(index))
    {
        if resembles_frontmatter(&region[opening.end()..closing.start()]) {
            return range_start + opening.start();
        }
    }
    range_start + closing.start()
}

fn resembles_frontmatter(source: &str) -> bool {
    let lines = source
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim().starts_with('#'))
        .collect::<Vec<_>>();
    !lines.is_empty()
        && lines.iter().all(|line| {
            line.chars().next().is_some_and(char::is_whitespace)
                || line.split_once(':').is_some_and(|(key, _)| {
                    !key.is_empty()
                        && key.chars().all(|character| {
                            character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
                        })
                })
                || line.trim_start().starts_with("- ")
                || matches!(line.trim(), "}" | "}," | "]" | "],")
        })
}

fn normalize_proposal(proposal: &mut GeminiProposal) -> AppResult<()> {
    proposal.message = truncate(proposal.message.trim(), 1_000);
    proposal.summary = truncate(proposal.summary.trim(), 500);
    proposal.focus_slide_key = proposal
        .focus_slide_key
        .take()
        .map(|value| truncate(value.trim(), 64))
        .filter(|value| !value.is_empty());
    match proposal.action.as_str() {
        "refuse_out_of_scope" => {
            proposal.message = REFUSAL.to_owned();
            proposal.summary.clear();
            proposal.operations.clear();
        }
        "clarify_deck_request" => {
            proposal.operations.clear();
            if proposal.message.is_empty() {
                proposal.message = "What should change in the current deck?".to_owned();
            }
        }
        "propose_patch" if proposal.operations.is_empty() => {
            return Err(AppError::Internal(anyhow!(
                "Gemini proposed a deck edit without any operations"
            )));
        }
        "propose_patch" => {}
        _ => {
            return Err(AppError::Internal(anyhow!(
                "Gemini returned an unsupported assistant action"
            )));
        }
    }
    if proposal.operations.len() > 12 {
        return Err(AppError::BadRequest(
            "Gemini proposed too many changes at once; ask for a smaller edit".to_owned(),
        ));
    }
    Ok(())
}

fn apply_operations(source: &str, operations: &[EditOperation]) -> AppResult<String> {
    let mut result = source.to_owned();
    let mut output_chars = 0_usize;
    for operation in operations {
        output_chars = output_chars.saturating_add(operation.new_text.chars().count());
        if output_chars > 250_000 {
            return Err(AppError::BadRequest(
                "Gemini proposed more Markdown than can be safely reviewed at once".to_owned(),
            ));
        }
        match operation.kind.as_str() {
            "replace" => {
                if operation.old_text.is_empty() {
                    return Err(AppError::Internal(anyhow!(
                        "Gemini returned an empty replacement target"
                    )));
                }
                let matches = result.match_indices(&operation.old_text).take(2).count();
                if matches != 1 {
                    return Err(AppError::Conflict(
                        "Gemini's proposed edit no longer has one exact Markdown target. Ask it to try again".to_owned(),
                    ));
                }
                result = result.replacen(&operation.old_text, &operation.new_text, 1);
            }
            "append" => {
                if !result.ends_with('\n') {
                    result.push('\n');
                }
                result.push_str(operation.new_text.trim_start_matches('\n'));
                if !result.ends_with('\n') {
                    result.push('\n');
                }
            }
            _ => {
                return Err(AppError::Internal(anyhow!(
                    "Gemini returned an unsupported edit operation"
                )));
            }
        }
    }
    if result == source {
        return Err(AppError::BadRequest(
            "Gemini's proposal would not change the deck".to_owned(),
        ));
    }
    if result.len() > 2_000_000 {
        return Err(AppError::BadRequest(
            "Gemini's proposal would make the deck too large".to_owned(),
        ));
    }
    Ok(result)
}

fn validate_proposed_markdown(source: &str) -> AppResult<usize> {
    let lower = source.to_ascii_lowercase();
    for prohibited in ["<script", "javascript:", "vite.config", "package.json"] {
        if lower.contains(prohibited) {
            return Err(AppError::BadRequest(format!(
                "Gemini proposed prohibited deck content containing `{prohibited}`"
            )));
        }
    }
    validate_deck_headmatter(source).map_err(|error| {
        AppError::BadRequest(format!("Gemini proposed invalid Slidev Markdown: {error}"))
    })?;
    let parsed = parse_deck(source).map_err(|error| {
        AppError::BadRequest(format!(
            "Gemini proposed invalid Interdeck Markdown: {error}"
        ))
    })?;
    Ok(parsed.slides.len())
}

fn validate_proposed_asset_references(
    source: &str,
    allowed_asset_urls: &HashSet<String>,
) -> AppResult<()> {
    if CONVENTIONAL_ASSET_REFERENCE.is_match(source) {
        return Err(AppError::BadRequest(
            "Gemini proposed a filename-based `/assets/...` URL, but Interdeck assets are private. Ask it to use the matching asset from this deck's asset library."
                .to_owned(),
        ));
    }
    for reference in PRIVATE_ASSET_REFERENCE.find_iter(source) {
        if !allowed_asset_urls.contains(reference.as_str()) {
            return Err(AppError::BadRequest(
                "Gemini proposed an asset URL that is not in this deck's asset library. Ask it to use an uploaded asset by its exact filename."
                    .to_owned(),
            ));
        }
    }
    Ok(())
}

async fn validate_with_slidev_parser(
    state: &AppState,
    deck_id: Uuid,
    source: &str,
    expected_slide_count: usize,
) -> AppResult<()> {
    let endpoint = format!(
        "{}/_gateway/slidev/validate-source",
        state.config.app_base_url
    );
    let response = state
        .http
        .post(endpoint)
        .header(
            "x-interdeck-internal-token",
            &state.config.internal_service_token,
        )
        .timeout(Duration::from_secs(15))
        .json(&json!({ "deck_id": deck_id, "markdown": source }))
        .send()
        .await
        .map_err(|_| {
            AppError::Upstream(
                "The Slidev proposal validator is temporarily unavailable. Try again.".to_owned(),
            )
        })?;
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or_default();
    if status.is_success() {
        let rendered_slide_count = body
            .get("slide_count")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| {
                AppError::Upstream(
                    "The Slidev proposal validator returned an incomplete result. Try again."
                        .to_owned(),
                )
            })?;
        ensure_matching_slide_counts(expected_slide_count, rendered_slide_count)?;
        return Ok(());
    }
    let message = body
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("Slidev could not parse the proposed deck Markdown.");
    if status.is_client_error() {
        return Err(AppError::BadRequest(format!(
            "Gemini proposed invalid Slidev Markdown: {}",
            truncate(message, 500)
        )));
    }
    Err(AppError::Upstream(
        "The Slidev proposal validator could not check this change. Try again.".to_owned(),
    ))
}

fn ensure_matching_slide_counts(expected: usize, rendered: usize) -> AppResult<()> {
    if expected != rendered {
        return Err(AppError::BadRequest(format!(
            "Gemini proposed slide boundaries that would merge slides: Interdeck found {expected}, but Slidev rendered {rendered}. Ask Gemini to add the slide again."
        )));
    }
    Ok(())
}

fn proposal_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "action": { "type": "string", "enum": ["propose_patch", "clarify_deck_request", "refuse_out_of_scope"] },
            "message": { "type": "string" },
            "summary": { "type": "string" },
            "focus_slide_key": { "type": ["string", "null"] },
            "operations": {
                "type": "array",
                "maxItems": 12,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "kind": { "type": "string", "enum": ["replace", "append"] },
                        "old_text": { "type": "string" },
                        "new_text": { "type": "string" }
                    },
                    "required": ["kind", "old_text", "new_text"]
                }
            }
        },
        "required": ["action", "message", "summary", "focus_slide_key", "operations"]
    })
}

fn proposal_generation_config(thinking_level: &str) -> Value {
    json!({
        "responseMimeType": "application/json",
        // responseSchema uses Google's older protobuf Schema dialect. The
        // assistant relies on JSON Schema features such as union types and
        // closed objects, so use the full JSON Schema field instead.
        "responseJsonSchema": proposal_schema(),
        "thinkingConfig": { "thinkingLevel": thinking_level }
    })
}

fn extract_output_text(response: &Value) -> Option<String> {
    response
        .get("candidates")?
        .as_array()?
        .iter()
        .flat_map(|candidate| {
            candidate
                .get("content")
                .and_then(|content| content.get("parts"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .find_map(|part| part.get("text").and_then(Value::as_str).map(str::to_owned))
}

fn provider_error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| value.pointer("/error/message")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| "Gemini returned an undocumented error".to_owned())
        .chars()
        .filter(|character| !character.is_control() || *character == '\n')
        .take(800)
        .collect()
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_unique_replacements_and_append_operations() {
        let result = apply_operations(
            "# One\n",
            &[
                EditOperation {
                    kind: "replace".to_owned(),
                    old_text: "# One".to_owned(),
                    new_text: "# First".to_owned(),
                },
                EditOperation {
                    kind: "append".to_owned(),
                    old_text: String::new(),
                    new_text: "\n---\n# Second".to_owned(),
                },
            ],
        )
        .unwrap();
        assert_eq!(result, "# First\n---\n# Second\n");
    }

    #[test]
    fn refuses_ambiguous_replacement_targets() {
        let error = apply_operations(
            "same\nsame\n",
            &[EditOperation {
                kind: "replace".to_owned(),
                old_text: "same".to_owned(),
                new_text: "new".to_owned(),
            }],
        )
        .unwrap_err();
        assert!(error.to_string().contains("one exact Markdown target"));
    }

    #[test]
    fn refuses_a_proposal_when_slidev_merges_marked_slides() {
        assert!(ensure_matching_slide_counts(4, 4).is_ok());
        let error = ensure_matching_slide_counts(4, 3).unwrap_err();
        assert!(error.to_string().contains("would merge slides"));
        assert!(error.to_string().contains("Interdeck found 4"));
        assert!(error.to_string().contains("Slidev rendered 3"));
    }

    #[test]
    fn uses_full_json_schema_for_gemini_structured_output() {
        let config = proposal_generation_config("low");
        assert!(config.get("responseJsonSchema").is_some());
        assert!(config.get("responseSchema").is_none());
        assert_eq!(
            config.pointer("/thinkingConfig/thinkingLevel"),
            Some(&json!("low"))
        );
    }

    #[test]
    fn extracts_only_global_deck_headmatter() {
        let markdown = "---\ntheme: default\ntitle: Demo\n---\n\n<!-- interdeck-slide: one -->\n# One\n\n---\nlayout: center\n---\n\n<!-- interdeck-slide: two -->\n# Two\n";
        assert_eq!(
            extract_deck_headmatter(markdown),
            "---\ntheme: default\ntitle: Demo\n---"
        );
    }

    #[test]
    fn gives_gemini_only_the_current_marked_slide() {
        let markdown = "---\ntheme: default\n---\n\n<!-- interdeck-slide: first -->\n# First\n\n---\nlayout: center\n---\n\n<!-- interdeck-slide: second_slide -->\n# Second\n\n---\n\n<!-- interdeck-slide: third -->\n# Third\n";
        assert_eq!(
            extract_marked_slide(markdown, "first"),
            "<!-- interdeck-slide: first -->\n# First\n\n"
        );
        assert_eq!(
            extract_marked_slide(markdown, "second_slide"),
            "---\nlayout: center\n---\n\n<!-- interdeck-slide: second_slide -->\n# Second\n\n"
        );
    }

    #[test]
    fn accepts_only_private_asset_urls_from_the_supplied_deck_catalog() {
        let allowed = HashSet::from([
            "/api/decks/00000000-0000-4000-8000-000000000001/assets/00000000-0000-4000-8000-000000000002/content".to_owned(),
        ]);
        let source = r#"<img src="/api/decks/00000000-0000-4000-8000-000000000001/assets/00000000-0000-4000-8000-000000000002/content" alt="Logo">"#;
        assert!(validate_proposed_asset_references(source, &allowed).is_ok());

        let wrong_deck = r#"<img src="/api/decks/00000000-0000-4000-8000-000000000099/assets/00000000-0000-4000-8000-000000000002/content" alt="Logo">"#;
        assert!(
            validate_proposed_asset_references(wrong_deck, &allowed)
                .unwrap_err()
                .to_string()
                .contains("not in this deck")
        );
    }

    #[test]
    fn rejects_invented_slidev_asset_paths_without_blocking_remote_urls() {
        let allowed = HashSet::new();
        for source in [
            r#"<img src="/assets/example-logo.png">"#,
            "![Logo](/assets/example-logo.png)",
            "background-image: url('/assets/example-logo.png')",
            r#":::interact{type="image-hotspot" id="map" image="/assets/map.png" alt="Map"}"#,
        ] {
            assert!(
                validate_proposed_asset_references(source, &allowed)
                    .unwrap_err()
                    .to_string()
                    .contains("filename-based")
            );
        }
        assert!(
            validate_proposed_asset_references(
                r#"<img src="https://cdn.example.com/assets/logo.png">"#,
                &allowed,
            )
            .is_ok()
        );
    }
}
