use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::{markdown::parse_deck, state::AppState};

const PROVIDER: &str = "gemini";
const MODERATION_PROMPT_VERSION: &str = "question-moderation-v3-presentation-context";
const INSIGHTS_PROMPT_VERSION: &str = "deck-insights-v4-presentation-neutral";
const STALLED_JOB_LEASE_SECONDS: i32 = 90;
const STALLED_JOB_SWEEP_SECONDS: u64 = 30;

#[derive(Debug)]
struct Job {
    id: Uuid,
    deck_id: Uuid,
    kind: String,
    payload: Value,
    attempts: i32,
}

#[derive(Debug, Deserialize)]
struct QuestionClassification {
    suggested_action: String,
    confidence: f64,
    safety_flagged: bool,
    safety_categories: Vec<String>,
    spam: bool,
    contains_pii: bool,
    off_topic: bool,
    topic: String,
    duplicate_question_id: Option<String>,
    reason: String,
}

pub fn spawn_worker(state: Arc<AppState>) {
    if state.config.gemini_moderator_key.is_none() {
        tracing::info!("AI moderation is disabled; manual moderation remains available");
        return;
    }
    let concurrency = state.config.ai_worker_concurrency;
    tokio::spawn(async move {
        match recover_stalled_jobs(&state).await {
            Ok(recovered) => record_recovered_jobs(&state, recovered),
            Err(error) => tracing::warn!(error = %error, "could not recover stalled AI jobs"),
        }
        if let Err(error) = recover_unprocessed_questions(&state).await {
            tracing::warn!(error = %error, "could not enqueue existing questions for AI review");
        }
        match recover_orphaned_queued_questions(&state).await {
            Ok(recovered) => record_orphaned_questions(&state, recovered),
            Err(error) => {
                tracing::warn!(error = %error, "could not repair orphaned AI review questions")
            }
        }
        tracing::info!(concurrency, "AI moderation workers started");
        tokio::spawn(stalled_job_recovery_loop(Arc::clone(&state)));
        for _ in 0..concurrency {
            tokio::spawn(worker_loop(Arc::clone(&state)));
        }
    });
}

async fn worker_loop(state: Arc<AppState>) {
    loop {
        match claim_job(&state).await {
            Ok(Some(job)) => {
                let result = match job.kind.as_str() {
                    "question_moderation" => process_question(&state, &job).await,
                    "deck_insights" => process_insights(&state, &job).await,
                    _ => Err(anyhow!("unsupported AI job kind")),
                };
                if let Err(error) = finish_job(&state, &job, result).await {
                    tracing::warn!(job_id = %job.id, error = %error, "could not finish AI job");
                }
            }
            Ok(None) => tokio::time::sleep(Duration::from_millis(750)).await,
            Err(error) => {
                tracing::warn!(error = %error, "could not claim AI job");
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    }
}

async fn stalled_job_recovery_loop(state: Arc<AppState>) {
    loop {
        tokio::time::sleep(Duration::from_secs(STALLED_JOB_SWEEP_SECONDS)).await;
        match recover_stalled_jobs(&state).await {
            Ok(recovered) => record_recovered_jobs(&state, recovered),
            Err(error) => tracing::warn!(error = %error, "could not sweep stalled AI jobs"),
        }
        match recover_orphaned_queued_questions(&state).await {
            Ok(recovered) => record_orphaned_questions(&state, recovered),
            Err(error) => {
                tracing::warn!(error = %error, "could not sweep orphaned AI review questions")
            }
        }
    }
}

fn record_orphaned_questions(state: &AppState, recovered: u64) {
    if recovered == 0 {
        return;
    }
    state.record_ai_jobs_recovered(recovered);
    tracing::warn!(
        recovered,
        "recreated missing AI moderation jobs for queued questions"
    );
}

fn record_recovered_jobs(state: &AppState, recovered: u64) {
    if recovered == 0 {
        return;
    }
    state.record_ai_jobs_recovered(recovered);
    tracing::warn!(recovered, "requeued AI jobs whose worker lease expired");
}

async fn recover_stalled_jobs(state: &AppState) -> Result<u64> {
    let recovered = sqlx::query(
        r#"
        UPDATE background_jobs
        SET status = 'queued', locked_at = NULL, available_at = now(), updated_at = now()
        WHERE status = 'running'
          AND locked_at < now() - make_interval(secs => $1)
        "#,
    )
    .bind(STALLED_JOB_LEASE_SECONDS)
    .execute(&state.db)
    .await?
    .rows_affected();
    Ok(recovered)
}

async fn recover_orphaned_queued_questions(state: &AppState) -> Result<u64> {
    let mut tx = state.db.begin().await?;
    let rows = sqlx::query(
        r#"
        SELECT q.id, q.deck_id
        FROM questions q
        JOIN decks d ON d.id = q.deck_id AND d.result_epoch = q.result_epoch
        WHERE q.analysis_status = 'queued'
          AND q.moderation_status <> 'rejected'
          AND q.lifecycle_status <> 'archived'
          AND NOT EXISTS (
              SELECT 1 FROM background_jobs job
              WHERE job.kind = 'question_moderation'
                AND job.status IN ('queued', 'running')
                AND job.payload->>'question_id' = q.id::text
          )
        ORDER BY q.created_at
        FOR UPDATE OF q SKIP LOCKED
        LIMIT 2000
        "#,
    )
    .fetch_all(&mut *tx)
    .await?;
    let recovered = rows.len() as u64;
    for row in rows {
        let question_id: Uuid = row.try_get("id")?;
        let deck_id: Uuid = row.try_get("deck_id")?;
        sqlx::query(
            "INSERT INTO background_jobs (id, deck_id, kind, payload) VALUES ($1, $2, 'question_moderation', $3)",
        )
        .bind(Uuid::now_v7())
        .bind(deck_id)
        .bind(json!({ "question_id": question_id }))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(recovered)
}

async fn recover_unprocessed_questions(state: &AppState) -> Result<()> {
    let rows = sqlx::query(
        r#"
        SELECT q.id, q.deck_id
        FROM questions q
        JOIN decks d ON d.id = q.deck_id AND d.result_epoch = q.result_epoch
        WHERE q.analysis_status IN ('not_requested', 'failed')
          AND q.moderation_status <> 'rejected'
          AND q.lifecycle_status <> 'archived'
        ORDER BY q.created_at
        LIMIT 2000
        "#,
    )
    .fetch_all(&state.db)
    .await?;
    let mut tx = state.db.begin().await?;
    let mut recovered = 0_u64;
    for row in rows {
        let question_id: Uuid = row.try_get("id")?;
        let deck_id: Uuid = row.try_get("deck_id")?;
        let claimed = sqlx::query(
            "UPDATE questions SET analysis_status = 'queued', updated_at = now() WHERE id = $1 AND analysis_status IN ('not_requested', 'failed')",
        )
        .bind(question_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if claimed == 0 {
            continue;
        }
        sqlx::query(
            "INSERT INTO background_jobs (id, deck_id, kind, payload) VALUES ($1, $2, 'question_moderation', $3)",
        )
        .bind(Uuid::now_v7())
        .bind(deck_id)
        .bind(json!({ "question_id": question_id }))
        .execute(&mut *tx)
        .await?;
        recovered += 1;
    }

    tx.commit().await?;
    if recovered > 0 {
        tracing::info!(
            questions = recovered,
            "existing Q&A queued for Gemini recovery"
        );
    }
    Ok(())
}

async fn claim_job(state: &AppState) -> Result<Option<Job>> {
    let row = sqlx::query(
        r#"
        UPDATE background_jobs
        SET status = 'running', attempts = attempts + 1, locked_at = now(), updated_at = now()
        WHERE id = (
            SELECT id FROM background_jobs
            WHERE status = 'queued' AND available_at <= now()
            ORDER BY
                CASE WHEN kind = 'question_moderation' THEN 0 ELSE 1 END,
                available_at,
                created_at
            FOR UPDATE SKIP LOCKED
            LIMIT 1
        )
        RETURNING id, deck_id, kind, payload, attempts
        "#,
    )
    .fetch_optional(&state.db)
    .await?;
    row.map(|row| {
        Ok(Job {
            id: row.try_get("id")?,
            deck_id: row.try_get("deck_id")?,
            kind: row.try_get("kind")?,
            payload: row.try_get("payload")?,
            attempts: row.try_get("attempts")?,
        })
    })
    .transpose()
}

async fn finish_job(state: &Arc<AppState>, job: &Job, result: Result<()>) -> Result<()> {
    match result {
        Ok(()) => {
            sqlx::query(
                "UPDATE background_jobs SET status = 'succeeded', locked_at = NULL, last_error = NULL, updated_at = now() WHERE id = $1",
            )
            .bind(job.id)
            .execute(&state.db)
            .await?;
            state.record_ai_job_succeeded();
            if job.kind == "deck_insights"
                && let Some(sequence) = bump_sequence(state, job.deck_id).await?
            {
                state.notify(job.deck_id, sequence);
            }
            tracing::info!(job_id = %job.id, deck_id = %job.deck_id, kind = %job.kind, attempt = job.attempts, "AI job succeeded");
        }
        Err(error) => {
            state.record_ai_job_attempt_failure();
            let terminal = job.attempts >= 5;
            let safe_error = truncate(&error.to_string(), 300);
            sqlx::query(
                r#"
                UPDATE background_jobs SET
                    status = CASE WHEN $2 THEN 'failed' ELSE 'queued' END,
                    available_at = CASE WHEN $2 THEN available_at
                        ELSE now() + make_interval(secs => LEAST(300, POWER(2, attempts)::integer)) END,
                    locked_at = NULL,
                    last_error = $3,
                    updated_at = now()
                WHERE id = $1
                "#,
            )
            .bind(job.id)
            .bind(terminal)
            .bind(&safe_error)
            .execute(&state.db)
            .await?;
            if terminal && job.kind == "question_moderation" {
                if let Some(question_id) = payload_uuid(&job.payload, "question_id") {
                    sqlx::query(
                        "UPDATE questions SET analysis_status = 'failed', moderation_reason = 'AI review could not be completed; manual review is available', updated_at = now() WHERE id = $1 AND analysis_status = 'queued'",
                    )
                    .bind(question_id)
                    .execute(&state.db)
                    .await?;
                    if let Some(sequence) = bump_sequence(state, job.deck_id).await? {
                        state.notify(job.deck_id, sequence);
                    }
                }
            }
            if terminal
                && job.kind == "deck_insights"
                && let Some(sequence) = bump_sequence(state, job.deck_id).await?
            {
                state.notify(job.deck_id, sequence);
            }
            tracing::warn!(job_id = %job.id, attempt = job.attempts, terminal, error = %safe_error, "AI job failed");
        }
    }
    Ok(())
}

async fn process_question(state: &Arc<AppState>, job: &Job) -> Result<()> {
    let question_id = payload_uuid(&job.payload, "question_id")
        .ok_or_else(|| anyhow!("question moderation job is missing question_id"))?;
    let row = sqlx::query(
        r#"
        SELECT q.body, q.result_epoch, q.slide_key, q.slide_number,
               d.title AS deck_title, d.markdown
        FROM questions q
        JOIN decks d ON d.id = q.deck_id
        WHERE q.id = $1 AND q.deck_id = $2
        "#,
    )
    .bind(question_id)
    .bind(job.deck_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| anyhow!("question no longer exists"))?;
    let body: String = row.try_get("body")?;
    let epoch: i32 = row.try_get("result_epoch")?;
    let slide_key: Option<String> = row.try_get("slide_key")?;
    let slide_number: Option<i32> = row.try_get("slide_number")?;
    let deck_title: String = row.try_get("deck_title")?;
    let markdown: String = row.try_get("markdown")?;
    let parsed = parse_deck(&markdown).ok();
    let slide_topics = parsed
        .as_ref()
        .map(|deck| {
            deck.slides
                .iter()
                .filter(|slide| !slide.title.trim().is_empty())
                .take(100)
                .map(|slide| {
                    json!({
                        "number": slide.number,
                        "key": slide.key,
                        "title": slide.title,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let current_slide = parsed.as_ref().and_then(|deck| {
        deck.slides.iter().find(|slide| {
            slide_key.as_deref() == Some(slide.key.as_str()) || slide_number == Some(slide.number)
        })
    });
    let moderation_context = json!({
        "question": body,
        "scope": if slide_number.is_some() || slide_key.is_some() { "slide" } else { "deck" },
        "presentation": {
            "title": deck_title,
            "slide_topics": slide_topics,
            "current_slide": current_slide.map(|slide| json!({
                "number": slide.number,
                "key": slide.key,
                "title": slide.title,
            })),
        },
    });
    let candidate_rows = sqlx::query(
        r#"
        SELECT id, body FROM questions
        WHERE deck_id = $1 AND result_epoch = $2 AND id <> $3
          AND moderation_status <> 'rejected'
        ORDER BY created_at DESC LIMIT 30
        "#,
    )
    .bind(job.deck_id)
    .bind(epoch)
    .bind(question_id)
    .fetch_all(&state.db)
    .await?;
    let candidates: Vec<Value> = candidate_rows
        .iter()
        .map(|row| json!({ "id": row.get::<Uuid, _>("id"), "body": row.get::<String, _>("body") }))
        .collect();
    let candidate_ids: HashSet<Uuid> = candidate_rows
        .iter()
        .map(|row| row.get::<Uuid, _>("id"))
        .collect();

    let classification = classify_question(state, &moderation_context, &candidates).await?;
    let duplicate_of = classification
        .duplicate_question_id
        .as_deref()
        .and_then(|value| Uuid::parse_str(value).ok())
        .filter(|id| candidate_ids.contains(id));
    let labels = json!({
        "safety_flagged": classification.safety_flagged,
        "safety_categories": classification.safety_categories,
        "spam": classification.spam,
        "contains_pii": classification.contains_pii,
        "off_topic": classification.off_topic,
    });
    let action = normalized_action(&classification.suggested_action);
    let confidence = classification.confidence.clamp(0.0, 1.0);
    let reason = truncate(classification.reason.trim(), 300);
    let topic = nonempty_truncated(&classification.topic, 80);
    let model = &state.config.gemini_moderator_model;
    let enforce = state.config.ai_moderation_mode == "enforce";
    let decided_status = enforcement_decision(
        enforce,
        classification.safety_flagged,
        classification.spam,
        classification.contains_pii,
        classification.off_topic,
        action,
    );

    let mut tx = state.db.begin().await?;
    sqlx::query(
        r#"
        INSERT INTO question_analysis_versions
            (id, question_id, provider, model, prompt_version, suggested_action,
             confidence, labels, reason, topic, duplicate_of)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
    )
    .bind(Uuid::now_v7())
    .bind(question_id)
    .bind(PROVIDER)
    .bind(model)
    .bind(MODERATION_PROMPT_VERSION)
    .bind(action)
    .bind(confidence)
    .bind(&labels)
    .bind(&reason)
    .bind(&topic)
    .bind(duplicate_of)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        UPDATE questions SET
            analysis_status = 'complete', moderation_labels = $2, moderation_reason = $3,
            topic = $4, duplicate_of = $5, analyzed_at = now(),
            moderation_status = CASE
                WHEN $6::text IS NOT NULL AND moderation_status = 'pending' THEN $6
                ELSE moderation_status
            END,
            moderation_source = CASE
                WHEN $6::text IS NOT NULL AND moderation_status = 'pending' THEN 'ai'
                ELSE moderation_source
            END,
            updated_at = now()
        WHERE id = $1
        "#,
    )
    .bind(question_id)
    .bind(&labels)
    .bind(&reason)
    .bind(&topic)
    .bind(duplicate_of)
    .bind(decided_status)
    .execute(&mut *tx)
    .await?;
    let sequence: Option<i64> = sqlx::query_scalar(
        "UPDATE live_deck_states SET sequence = sequence + 1, updated_at = now() WHERE deck_id = $1 RETURNING sequence",
    )
    .bind(job.deck_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    if let Some(sequence) = sequence {
        state.notify(job.deck_id, sequence);
    }
    Ok(())
}

async fn process_insights(state: &Arc<AppState>, job: &Job) -> Result<()> {
    let epoch = job
        .payload
        .get("result_epoch")
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| anyhow!("deck insights job is missing result_epoch"))?;
    let is_current_epoch: bool =
        sqlx::query_scalar("SELECT result_epoch = $2 FROM decks WHERE id = $1")
            .bind(job.deck_id)
            .bind(epoch)
            .fetch_optional(&state.db)
            .await?
            .unwrap_or(false);
    if !is_current_epoch {
        return Ok(());
    }
    let rows = sqlx::query(
        r#"
        SELECT q.id, q.body, p.display_name, q.slide_number
        FROM questions q
        JOIN participant_sessions p ON p.id = q.participant_id
        WHERE q.deck_id = $1 AND q.result_epoch = $2
          AND q.moderation_status = 'approved' AND q.lifecycle_status <> 'archived'
          AND q.analysis_status = 'complete'
        ORDER BY q.created_at DESC LIMIT 100
        "#,
    )
    .bind(job.deck_id)
    .bind(epoch)
    .fetch_all(&state.db)
    .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let questions: Vec<Value> = rows
        .iter()
        .map(|row| {
            let slide_number = row.get::<Option<i32>, _>("slide_number");
            json!({
                "id": row.get::<Uuid, _>("id"),
                "body": row.get::<String, _>("body"),
                "scope": if slide_number.is_some() { "slide" } else { "deck" },
                "slide_number": slide_number,
            })
        })
        .collect();
    let allowed_ids: HashSet<String> = rows
        .iter()
        .map(|row| row.get::<Uuid, _>("id").to_string())
        .collect();
    let mut insight = request_insights(state, &questions).await?;
    sanitize_theme_ids(&mut insight, &allowed_ids);
    let question_metadata: HashMap<String, Value> = rows
        .iter()
        .map(|row| {
            let id = row.get::<Uuid, _>("id").to_string();
            let slide_number = row.get::<Option<i32>, _>("slide_number");
            let asker = row
                .get::<Option<String>, _>("display_name")
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| "Anonymous".to_owned());
            let metadata = json!({
                "id": id,
                "body": row.get::<String, _>("body"),
                "asker": asker,
                "scope": if slide_number.is_some() { "slide" } else { "deck" },
                "slide_number": slide_number,
            });
            (id, metadata)
        })
        .collect();
    enrich_theme_questions(&mut insight, &question_metadata);
    let summary = insight
        .get("summary")
        .and_then(Value::as_str)
        .map(|value| truncate(value, 1000))
        .unwrap_or_default();
    let themes = insight.get("themes").cloned().unwrap_or_else(|| json!([]));
    let answers = insight
        .get("suggested_answers")
        .cloned()
        .unwrap_or_else(|| json!([]));
    sqlx::query(
        r#"
        INSERT INTO deck_insight_versions
            (id, deck_id, result_epoch, provider, model, prompt_version,
             question_count, summary, themes, suggested_answers)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
    )
    .bind(Uuid::now_v7())
    .bind(job.deck_id)
    .bind(epoch)
    .bind(PROVIDER)
    .bind(&state.config.gemini_moderator_model)
    .bind(INSIGHTS_PROMPT_VERSION)
    .bind(rows.len() as i32)
    .bind(summary)
    .bind(themes)
    .bind(answers)
    .execute(&state.db)
    .await?;
    Ok(())
}

async fn classify_question(
    state: &AppState,
    moderation_context: &Value,
    candidates: &[Value],
) -> Result<QuestionClassification> {
    let input = json!({
        "submission": moderation_context,
        "recent_questions": candidates,
    });
    let response = gemini_request(
        state,
        "Classify an audience question for the presentation described in the supplied context. Judge relevance from the actual presentation title and slide topics, never from an assumed event type, organization, or subject. A slide-scoped question should normally relate to its current slide or the wider presentation; a deck-scoped question may relate to any supplied topic. Be conservative about rejection: approve plausible, good-faith questions and use review for genuinely ambiguous relevance. Detect unsafe content, harassment, spam, personal/contact information, and semantic duplicates. Ordinary disagreement, criticism, difficult questions, and negative sentiment are not unsafe. Return only the requested schema. Never repeat personal information in the reason.",
        &input.to_string(),
        question_schema(),
    )
    .await?;
    serde_json::from_str(&response).context("invalid structured question analysis")
}

async fn request_insights(state: &AppState, questions: &[Value]) -> Result<Value> {
    gemini_request(
        state,
        "Theme approved audience questions for the presenter. For every theme, write one concise, neutral, answerable representative question that faithfully covers the shared gist of its source questions. Preserve meaningful disagreement instead of smoothing it away, split themes when one representative question would be misleading, distinguish deck-wide from slide-specific context when relevant, and never invent facts. Base every theme on supplied question IDs. Make suggested answers brief talking-point prompts rather than factual claims. Return only the requested schema. Participant names are intentionally not provided and must not be invented.",
        &json!({ "questions": questions }).to_string(),
        insights_schema(),
    )
    .await
    .and_then(|text| serde_json::from_str(&text).context("invalid structured deck insights"))
}

async fn gemini_request(
    state: &AppState,
    instructions: &str,
    input: &str,
    schema: Value,
) -> Result<String> {
    let endpoint = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
        state.config.gemini_moderator_model
    );
    let response = state
        .http
        .post(endpoint)
        .header("x-goog-api-key", api_key(state)?)
        .timeout(Duration::from_secs(30))
        .json(&json!({
            "system_instruction": { "parts": [{ "text": instructions }] },
            "contents": [{ "role": "user", "parts": [{ "text": input }] }],
            "generationConfig": structured_generation_config(schema)
        }))
        .send()
        .await
        .context("structured analysis request failed")?;
    ensure_success(response.status(), "structured analysis")?;
    let value: Value = response
        .json()
        .await
        .context("invalid structured analysis response")?;
    extract_output_text(&value).ok_or_else(|| anyhow!("structured analysis returned no text"))
}

fn structured_generation_config(schema: Value) -> Value {
    json!({
        "responseMimeType": "application/json",
        "responseJsonSchema": schema,
        "thinkingConfig": { "thinkingLevel": "low" }
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

fn question_schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false,
        "properties": {
            "suggested_action": { "type": "string", "enum": ["approve", "review", "reject"] },
            "confidence": { "type": "number", "minimum": 0, "maximum": 1 },
            "safety_flagged": { "type": "boolean" },
            "safety_categories": { "type": "array", "items": { "type": "string" } },
            "spam": { "type": "boolean" },
            "contains_pii": { "type": "boolean" },
            "off_topic": { "type": "boolean" },
            "topic": { "type": "string" },
            "duplicate_question_id": { "type": ["string", "null"] },
            "reason": { "type": "string" }
        },
        "required": ["suggested_action", "confidence", "safety_flagged", "safety_categories", "spam", "contains_pii", "off_topic", "topic", "duplicate_question_id", "reason"]
    })
}

fn insights_schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false,
        "properties": {
            "summary": { "type": "string" },
            "themes": { "type": "array", "maxItems": 8, "items": {
                "type": "object", "additionalProperties": false,
                "properties": {
                    "label": { "type": "string" },
                    "summary": { "type": "string" },
                    "representative_question": { "type": "string" },
                    "question_ids": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["label", "summary", "representative_question", "question_ids"]
            }},
            "suggested_answers": { "type": "array", "maxItems": 8, "items": { "type": "string" } }
        },
        "required": ["summary", "themes", "suggested_answers"]
    })
}

pub(crate) async fn enqueue_insights(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    deck_id: Uuid,
    result_epoch: i32,
) -> Result<bool> {
    let queued = sqlx::query(
        r#"
        INSERT INTO background_jobs (id, deck_id, kind, payload)
        SELECT $1, $2, 'deck_insights', $3
        WHERE NOT EXISTS (
            SELECT 1 FROM background_jobs
            WHERE deck_id = $2 AND kind = 'deck_insights'
              AND status IN ('queued', 'running')
        )
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(Uuid::now_v7())
    .bind(deck_id)
    .bind(json!({ "result_epoch": result_epoch }))
    .execute(&mut **tx)
    .await?
    .rows_affected()
        > 0;
    Ok(queued)
}

async fn bump_sequence(state: &AppState, deck_id: Uuid) -> Result<Option<i64>> {
    sqlx::query_scalar(
        "UPDATE live_deck_states SET sequence = sequence + 1, updated_at = now() WHERE deck_id = $1 RETURNING sequence",
    )
    .bind(deck_id)
    .fetch_optional(&state.db)
    .await
    .map_err(Into::into)
}

fn api_key(state: &AppState) -> Result<&str> {
    state
        .config
        .gemini_moderator_key
        .as_deref()
        .ok_or_else(|| anyhow!("AI provider is not configured"))
}

fn ensure_success(status: StatusCode, operation: &str) -> Result<()> {
    if status.is_success() {
        Ok(())
    } else {
        bail!("{operation} provider returned HTTP {}", status.as_u16())
    }
}

fn payload_uuid(payload: &Value, key: &str) -> Option<Uuid> {
    payload.get(key)?.as_str()?.parse().ok()
}

fn normalized_action(action: &str) -> &str {
    match action {
        "approve" | "reject" => action,
        _ => "review",
    }
}

fn enforcement_decision(
    enforce: bool,
    safety_flagged: bool,
    spam: bool,
    contains_pii: bool,
    off_topic: bool,
    suggested_action: &str,
) -> Option<&'static str> {
    if !enforce {
        return None;
    }
    Some(
        if safety_flagged || spam || contains_pii || suggested_action == "reject" {
            "rejected"
        } else if suggested_action == "approve" && !off_topic {
            "approved"
        } else {
            "pending"
        },
    )
}

fn nonempty_truncated(value: &str, max_chars: usize) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| truncate(value, max_chars))
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn sanitize_theme_ids(insight: &mut Value, allowed_ids: &HashSet<String>) {
    let Some(themes) = insight.get_mut("themes").and_then(Value::as_array_mut) else {
        return;
    };
    for theme in themes {
        let Some(ids) = theme.get_mut("question_ids").and_then(Value::as_array_mut) else {
            continue;
        };
        ids.retain(|id| id.as_str().is_some_and(|id| allowed_ids.contains(id)));
    }
}

fn enrich_theme_questions(insight: &mut Value, metadata: &HashMap<String, Value>) {
    let Some(themes) = insight.get_mut("themes").and_then(Value::as_array_mut) else {
        return;
    };
    for theme in themes {
        let questions = theme
            .get("question_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|id| metadata.get(id).cloned())
            .collect::<Vec<_>>();
        if let Some(theme) = theme.as_object_mut() {
            theme.insert("questions".to_owned(), Value::Array(questions));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_structured_output_text() {
        let response = json!({ "candidates": [{ "content": { "parts": [
            { "text": "{\"suggested_action\":\"review\"}" }
        ]}}] });
        assert_eq!(
            extract_output_text(&response).as_deref(),
            Some("{\"suggested_action\":\"review\"}")
        );
    }

    #[test]
    fn removes_hallucinated_question_ids_from_themes() {
        let valid = Uuid::now_v7().to_string();
        let allowed = HashSet::from([valid.clone()]);
        let mut insight = json!({ "themes": [{
            "label": "Delivery", "summary": "Questions about delivery",
            "question_ids": [valid, Uuid::now_v7().to_string()]
        }] });
        sanitize_theme_ids(&mut insight, &allowed);
        assert_eq!(
            insight["themes"][0]["question_ids"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn joins_trusted_asker_and_scope_metadata_after_theming() {
        let id = Uuid::now_v7().to_string();
        let mut insight = json!({ "themes": [{
            "label": "Delivery", "summary": "Questions about delivery", "question_ids": [id]
        }] });
        let metadata = HashMap::from([(
            id.clone(),
            json!({
                "id": id,
                "body": "When will this ship?",
                "asker": "Ayesha",
                "scope": "slide",
                "slide_number": 4
            }),
        )]);
        enrich_theme_questions(&mut insight, &metadata);
        assert_eq!(insight["themes"][0]["questions"][0]["asker"], "Ayesha");
        assert_eq!(insight["themes"][0]["questions"][0]["slide_number"], 4);
    }

    #[test]
    fn classification_schema_is_closed_and_requires_all_fields() {
        let schema = question_schema();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"].as_array().unwrap().len(), 10);
    }

    #[test]
    fn uses_full_json_schema_for_gemini_structured_output() {
        let config = structured_generation_config(question_schema());
        assert!(config.get("responseJsonSchema").is_some());
        assert!(config.get("responseSchema").is_none());
    }

    #[test]
    fn assist_mode_never_makes_a_moderation_decision() {
        assert_eq!(
            enforcement_decision(false, true, true, true, true, "reject"),
            None
        );
    }

    #[test]
    fn enforcement_is_conservative_for_risky_and_uncertain_questions() {
        assert_eq!(
            enforcement_decision(true, false, true, false, false, "approve"),
            Some("rejected")
        );
        assert_eq!(
            enforcement_decision(true, false, false, false, true, "review"),
            Some("pending")
        );
        assert_eq!(
            enforcement_decision(true, false, false, false, false, "approve"),
            Some("approved")
        );
    }
}
