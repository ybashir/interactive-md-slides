use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    path::Path as FsPath,
    sync::{Arc, OnceLock},
    time::Duration as StdDuration,
};

use axum::{
    Json,
    body::Bytes,
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use rand::Rng;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgRow};
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::{
    auth::{cookie_value, hash_token, random_token, require_user},
    error::{AppError, AppResult},
    markdown::{ensure_slide_markers, parse_deck, validate_deck_headmatter},
    models::{
        AudienceInteraction, CreateDeckRequest, DeckAsset, DeckDetail, DeckDraftSave,
        DeckInsightView, DeckSummary, InputControlRequest, InteractionControlRequest,
        InteractionDefinition, InteractionOption, JoinRequest, LiveState, LiveSummary,
        LiveSummaryInteraction, LiveSummaryQuestion, LiveSummaryQuestions, ModerateQuestionRequest,
        NavigateRequest, QaSettingsRequest, QuestionRequest, QuestionView, RecordDeckBuildRequest,
        ResponseRequest, RestoreLastVerifiedRequest, UpdateDeckRequest,
    },
    state::AppState,
};

const AUDIENCE_COOKIE: &str = "interdeck_audience";
const SEA_CREATURES_SAMPLE: &str = include_str!("../../../samples/amazing-sea-creatures.md");
const SEA_CREATURES_SAMPLE_CSS: &str = include_str!("../../../samples/amazing-sea-creatures.css");
const MAX_INTERACTION_PAYLOAD_BYTES: usize = 8_192;
const MAX_WORD_CLOUD_ENTRY_CHARS: usize = 40;
const MAX_WORD_CLOUD_ENTRY_BYTES: usize = 160;
const MAX_WORD_CLOUD_ENTRIES_PER_PARTICIPANT: usize = 12;

pub async fn public_config(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({
        "workspace_domain": state.config.google_workspace_domain,
        "ai_moderation_enabled": state.config.gemini_moderator_key.is_some(),
        "ai_assistant_enabled": state.config.gemini_assistant_key.is_some(),
        "audience_retention_days": state.config.audience_retention_days,
    }))
}

#[derive(Debug, Deserialize)]
pub struct SlidevAccessQuery {
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InternalSourceQuery {
    pub version: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ResultExportQuery {
    pub epoch: Option<i32>,
}

#[derive(Debug, Serialize)]
struct SlidevTokenPayload {
    deck_id: Uuid,
    version: i64,
    mode: String,
    exp: i64,
}

#[derive(Debug, Serialize)]
pub struct SlidevSource {
    deck_id: Uuid,
    title: String,
    markdown: String,
    css: String,
    version: i64,
    join_url: String,
}

pub async fn health_live() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

pub async fn health_ready(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    sqlx::query("SELECT 1").execute(&state.db).await?;
    Ok(Json(json!({ "status": "ready" })))
}

pub async fn internal_metrics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    require_internal(&state, &headers)?;
    Ok(Json(state.operational_metrics()))
}

pub async fn export_results_csv(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    Query(query): Query<ResultExportQuery>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let user = require_user(&state, &headers).await?;
    let current_epoch: i32 =
        sqlx::query_scalar("SELECT result_epoch FROM decks WHERE id = $1 AND owner_id = $2")
            .bind(deck_id)
            .bind(user.id)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::NotFound)?;
    let epoch = query.epoch.unwrap_or(current_epoch);
    if epoch < 1 {
        return Err(AppError::BadRequest(
            "Result epoch must be a positive integer".to_owned(),
        ));
    }
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM result_epochs WHERE deck_id = $1 AND epoch = $2)",
    )
    .bind(deck_id)
    .bind(epoch)
    .fetch_one(&state.db)
    .await?;
    if !exists {
        return Err(AppError::NotFound);
    }

    let rows = sqlx::query(
        r#"
        SELECT cr.interaction_key, i.kind, i.title, i.slide_number,
               cr.participant_id, p.display_name, cr.payload, cr.updated_at
        FROM current_responses cr
        JOIN participant_sessions p ON p.id = cr.participant_id
        LEFT JOIN interaction_definitions i
          ON i.deck_id = cr.deck_id AND i.interaction_key = cr.interaction_key
        WHERE cr.deck_id = $1 AND cr.result_epoch = $2
        ORDER BY COALESCE(i.slide_number, 2147483647), cr.interaction_key,
                 cr.updated_at, cr.participant_id
        "#,
    )
    .bind(deck_id)
    .bind(epoch)
    .fetch_all(&state.db)
    .await?;

    let mut csv = String::from(
        "result_epoch,slide_number,interaction_id,interaction_kind,interaction_title,participant_id,participant_name,response_json,submitted_at\r\n",
    );
    for row in rows {
        let payload: Value = row.try_get("payload")?;
        let values = [
            epoch.to_string(),
            row.try_get::<Option<i32>, _>("slide_number")?
                .map(|value| value.to_string())
                .unwrap_or_default(),
            row.try_get("interaction_key")?,
            row.try_get::<Option<String>, _>("kind")?
                .unwrap_or_else(|| "archived".to_owned()),
            row.try_get::<Option<String>, _>("title")?
                .unwrap_or_default(),
            row.try_get::<Uuid, _>("participant_id")?.to_string(),
            row.try_get::<Option<String>, _>("display_name")?
                .unwrap_or_else(|| "Anonymous".to_owned()),
            payload.to_string(),
            row.try_get::<DateTime<Utc>, _>("updated_at")?.to_rfc3339(),
        ];
        csv.push_str(
            &values
                .iter()
                .map(|value| csv_cell(value))
                .collect::<Vec<_>>()
                .join(","),
        );
        csv.push_str("\r\n");
    }

    let mut response = csv.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    let disposition =
        format!("attachment; filename=\"interdeck-results-{deck_id}-epoch-{epoch}.csv\"");
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&disposition).map_err(|error| AppError::Internal(error.into()))?,
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    Ok(response)
}

pub async fn rotate_join_code(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let user = require_user(&state, &headers).await?;
    let mut tx = state.db.begin().await?;
    let live: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM presentation_runs
            WHERE deck_id = $1 AND status = 'live'
        )
        FROM decks
        WHERE id = $1 AND owner_id = $2
        FOR UPDATE
        "#,
    )
    .bind(deck_id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    if live {
        return Err(AppError::Conflict(
            "End the live presentation before rotating its audience code".to_owned(),
        ));
    }

    let mut rotated = None;
    for _ in 0..10 {
        let candidate = new_join_code();
        let result = sqlx::query(
            "UPDATE decks SET join_code = $1, updated_at = now() WHERE id = $2 AND owner_id = $3",
        )
        .bind(&candidate)
        .bind(deck_id)
        .bind(user.id)
        .execute(&mut *tx)
        .await;
        match result {
            Ok(_) => {
                rotated = Some(candidate);
                break;
            }
            Err(sqlx::Error::Database(error))
                if error.constraint() == Some("decks_join_code_key") => {}
            Err(error) => return Err(error.into()),
        }
    }
    let join_code = rotated
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("could not rotate the join code")))?;
    audit(
        &mut tx,
        Some(deck_id),
        Some(user.id),
        "deck.join_code_rotated",
        json!({ "invalidated_previous_code": true }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "join_code": join_code })))
}

pub async fn list_decks(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<Vec<DeckSummary>>> {
    let user = require_user(&state, &headers).await?;
    let rows = sqlx::query(
        r#"
        SELECT d.id, d.title, d.join_code, d.version, d.updated_at,
               EXISTS(SELECT 1 FROM presentation_runs r WHERE r.deck_id = d.id AND r.status = 'live') AS is_live
        FROM decks d
        WHERE d.owner_id = $1
        ORDER BY d.updated_at DESC
        "#,
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;

    let decks = rows
        .into_iter()
        .map(|row| {
            Ok(DeckSummary {
                id: row.try_get("id")?,
                title: row.try_get("title")?,
                join_code: row.try_get("join_code")?,
                version: row.try_get("version")?,
                updated_at: row.try_get("updated_at")?,
                is_live: row.try_get("is_live")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    Ok(Json(decks))
}

pub async fn create_deck(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateDeckRequest>,
) -> AppResult<(axum::http::StatusCode, Json<DeckDetail>)> {
    let user = require_user(&state, &headers).await?;
    let title = clean_title(&request.title)?;
    let deck_id = Uuid::now_v7();
    if request.markdown.is_some() && request.sample.is_some() {
        return Err(AppError::BadRequest(
            "Choose either imported Markdown or a sample, not both".to_owned(),
        ));
    }
    let (source, css) = match request.sample.as_deref() {
        Some("amazing-sea-creatures") => (
            SEA_CREATURES_SAMPLE.to_owned(),
            SEA_CREATURES_SAMPLE_CSS.to_owned(),
        ),
        Some(_) => return Err(AppError::BadRequest("Unknown sample deck".to_owned())),
        None => match request.markdown {
            Some(markdown) => (markdown, String::new()),
            None => (starter_markdown(&title), starter_css().to_owned()),
        },
    };
    if source.len() > 2_000_000 {
        return Err(AppError::BadRequest(
            "Deck Markdown is too large".to_owned(),
        ));
    }
    let markdown = ensure_slide_markers(&source);
    let parsed = parse_deck(&markdown).map_err(AppError::BadRequest)?;

    let mut tx = state.db.begin().await?;
    let join_code =
        insert_deck_with_unique_code(&mut tx, deck_id, user.id, &title, &markdown, &css).await?;
    sqlx::query(
        "UPDATE decks SET last_verified_markdown = $1, last_verified_css = $2 WHERE id = $3",
    )
    .bind(&markdown)
    .bind(&css)
    .bind(deck_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO result_epochs (deck_id, epoch, created_by, reason) VALUES ($1, 1, $2, 'initial')",
    )
    .bind(deck_id)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sync_interactions(&mut tx, deck_id, 1, &parsed.interactions).await?;
    tx.commit().await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(DeckDetail {
            id: deck_id,
            title,
            join_code,
            markdown,
            css,
            version: 1,
            verified_version: None,
            is_verified: false,
            can_restore_last_verified: false,
            result_epoch: 1,
            qa_display_mode: "verbatim".to_owned(),
            slides: parsed.slides,
            interactions: parsed.interactions,
            warnings: parsed.warnings,
            is_live: false,
        }),
    ))
}

pub async fn delete_deck(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<StatusCode> {
    let user = require_user(&state, &headers).await?;
    let mut tx = state.db.begin().await?;
    let row = sqlx::query(
        r#"
        SELECT d.title,
               EXISTS(
                   SELECT 1 FROM presentation_runs r
                   WHERE r.deck_id = d.id AND r.status = 'live'
               ) AS is_live
        FROM decks d
        WHERE d.id = $1 AND d.owner_id = $2
        FOR UPDATE OF d
        "#,
    )
    .bind(deck_id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    if row.try_get::<bool, _>("is_live")? {
        return Err(AppError::Conflict(
            "End the live presentation before deleting this deck".to_owned(),
        ));
    }

    let storage_keys = sqlx::query_scalar::<_, String>(
        "SELECT storage_key FROM deck_assets WHERE deck_id = $1 ORDER BY id",
    )
    .bind(deck_id)
    .fetch_all(&mut *tx)
    .await?;
    for storage_key in &storage_keys {
        sqlx::query("INSERT INTO asset_gc (storage_key) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(storage_key)
            .execute(&mut *tx)
            .await?;
    }

    sqlx::query("DELETE FROM decks WHERE id = $1 AND owner_id = $2")
        .bind(deck_id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!(
        deck_id = %deck_id,
        owner_id = %user.id,
        deleted_assets = storage_keys.len(),
        "deck deleted"
    );
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_deck(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Json<DeckDetail>> {
    let user = require_user(&state, &headers).await?;
    Ok(Json(fetch_deck_detail(&state.db, deck_id, user.id).await?))
}

pub async fn update_deck(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<UpdateDeckRequest>,
) -> AppResult<Json<DeckDraftSave>> {
    let user = require_user(&state, &headers).await?;
    if request.markdown.len() > 2_000_000 {
        return Err(AppError::BadRequest(
            "Deck Markdown is too large".to_owned(),
        ));
    }
    if request.css.as_ref().is_some_and(|css| css.len() > 500_000) {
        return Err(AppError::BadRequest("Deck CSS is too large".to_owned()));
    }
    let title = request.title.as_deref().map(clean_title).transpose()?;
    Ok(Json(
        persist_deck_draft(
            &state,
            deck_id,
            user.id,
            &request.markdown,
            request.css.as_deref(),
            title.as_deref(),
            request.expected_version,
        )
        .await?,
    ))
}

pub async fn restore_last_verified(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<RestoreLastVerifiedRequest>,
) -> AppResult<Json<DeckDetail>> {
    let user = require_user(&state, &headers).await?;
    let mut tx = state.db.begin().await?;
    let row = sqlx::query(
        r#"
        SELECT version, last_verified_markdown, last_verified_css,
               EXISTS(SELECT 1 FROM presentation_runs p WHERE p.deck_id = d.id AND p.status = 'live') AS is_live
        FROM decks d
        WHERE id = $1 AND owner_id = $2
        FOR UPDATE
        "#,
    )
    .bind(deck_id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let version: i64 = row.try_get("version")?;
    if version != request.expected_version {
        return Err(AppError::Conflict(
            "This deck changed in another tab; reload before restoring".to_owned(),
        ));
    }
    if row.try_get::<bool, _>("is_live")? {
        return Err(AppError::Conflict(
            "End the live presentation before restoring this deck".to_owned(),
        ));
    }
    let markdown: String = row
        .try_get::<Option<String>, _>("last_verified_markdown")?
        .ok_or_else(|| {
            AppError::Conflict("No verified preview is available to restore".to_owned())
        })?;
    let css: String = row
        .try_get::<Option<String>, _>("last_verified_css")?
        .unwrap_or_default();
    let parsed = parse_deck(&markdown).map_err(AppError::BadRequest)?;
    let next_version = version + 1;
    sqlx::query(
        r#"
        UPDATE decks
        SET markdown = $1, deck_css = $2, version = $3, verified_version = $3, updated_at = now()
        WHERE id = $4 AND owner_id = $5 AND version = $6
        "#,
    )
    .bind(&markdown)
    .bind(&css)
    .bind(next_version)
    .bind(deck_id)
    .bind(user.id)
    .bind(version)
    .execute(&mut *tx)
    .await?;
    sync_interactions(&mut tx, deck_id, next_version, &parsed.interactions).await?;
    audit(
        &mut tx,
        Some(deck_id),
        Some(user.id),
        "deck.last_verified_restored",
        json!({ "version": next_version, "previous_version": version }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(fetch_deck_detail(&state.db, deck_id, user.id).await?))
}

pub async fn list_deck_assets(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Json<Vec<DeckAsset>>> {
    let user = require_user(&state, &headers).await?;
    assert_owner(&state.db, deck_id, user.id).await?;
    let rows = sqlx::query(
        r#"
        SELECT id, deck_id, original_filename, media_type, byte_size, sha256, created_at
        FROM deck_assets
        WHERE deck_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(deck_id)
    .fetch_all(&state.db)
    .await?;
    let assets = rows
        .into_iter()
        .map(deck_asset)
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    Ok(Json(assets))
}

pub async fn upload_deck_asset(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> AppResult<(StatusCode, Json<DeckAsset>)> {
    let user = require_user(&state, &headers).await?;
    assert_owner(&state.db, deck_id, user.id).await?;
    let mut upload: Option<(String, Bytes)> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| AppError::BadRequest("The asset upload is malformed".to_owned()))?
    {
        if field.name() != Some("file") || upload.is_some() {
            continue;
        }
        let filename = field.file_name().map(str::to_owned).ok_or_else(|| {
            AppError::BadRequest("The uploaded asset needs a filename".to_owned())
        })?;
        let bytes = field
            .bytes()
            .await
            .map_err(|_| AppError::BadRequest("The asset upload is malformed".to_owned()))?;
        upload = Some((filename, bytes));
    }
    let (supplied_filename, bytes) = upload
        .ok_or_else(|| AppError::BadRequest("Choose one image or font to upload".to_owned()))?;
    let detected = detect_asset(&bytes)?;
    if bytes.len() > detected.max_bytes {
        return Err(AppError::BadRequest(format!(
            "{} files must be {} MB or smaller",
            detected.label,
            detected.max_bytes / 1_000_000
        )));
    }
    let original_filename = display_filename(&supplied_filename)?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let asset_id = Uuid::now_v7();
    let stored_filename = storage_filename(&original_filename, detected.extension);
    let storage_key = format!("decks/{deck_id}/assets/{asset_id}/{stored_filename}");
    // Register cleanup before writing the object. A crash or failed transaction
    // leaves a durable entry; success removes it atomically with the asset row.
    sqlx::query(
        "INSERT INTO asset_gc (storage_key, available_at) VALUES ($1, now() + interval '1 hour')",
    )
    .bind(&storage_key)
    .execute(&state.db)
    .await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("SELECT id FROM decks WHERE id = $1 AND owner_id = $2 FOR UPDATE")
        .bind(deck_id)
        .bind(user.id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
    sqlx::query("SELECT storage_key FROM asset_gc WHERE storage_key = $1 FOR UPDATE")
        .bind(&storage_key)
        .fetch_one(&mut *tx)
        .await?;
    // The deck lock serializes same-content uploads across replicas. Recheck
    // after acquiring it so a concurrent duplicate returns the committed asset
    // instead of writing another object and failing the uniqueness constraint.
    if let Some(existing) = sqlx::query(
        "SELECT id, deck_id, original_filename, media_type, byte_size, sha256, created_at FROM deck_assets WHERE deck_id = $1 AND sha256 = $2",
    )
    .bind(deck_id)
    .bind(&sha256)
    .fetch_optional(&mut *tx)
    .await?
    {
        sqlx::query("DELETE FROM asset_gc WHERE storage_key = $1")
            .bind(&storage_key)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok((StatusCode::OK, Json(deck_asset(existing)?)));
    }
    state
        .assets
        .put(&storage_key, detected.media_type, bytes.clone())
        .await
        .map_err(AppError::Internal)?;

    let row = sqlx::query(
        r#"
        INSERT INTO deck_assets (
            id, deck_id, original_filename, media_type, byte_size, sha256, storage_key, created_by
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING id, deck_id, original_filename, media_type, byte_size, sha256, created_at
        "#,
    )
    .bind(asset_id)
    .bind(deck_id)
    .bind(&original_filename)
    .bind(detected.media_type)
    .bind(bytes.len() as i64)
    .bind(&sha256)
    .bind(&storage_key)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    audit(
        &mut tx,
        Some(deck_id),
        Some(user.id),
        "deck.asset_uploaded",
        json!({
            "asset_id": asset_id,
            "media_type": detected.media_type,
            "byte_size": bytes.len(),
            "sha256": sha256,
        }),
    )
    .await?;
    sqlx::query("DELETE FROM asset_gc WHERE storage_key = $1")
        .bind(&storage_key)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(deck_asset(row)?)))
}

pub async fn deck_asset_content(
    State(state): State<Arc<AppState>>,
    Path((deck_id, asset_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let user = require_user(&state, &headers).await?;
    let row = sqlx::query(
        r#"
        SELECT a.storage_key, a.media_type
        FROM deck_assets a JOIN decks d ON d.id = a.deck_id
        WHERE a.id = $1 AND a.deck_id = $2 AND d.owner_id = $3
        "#,
    )
    .bind(asset_id)
    .bind(deck_id)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    stored_asset_response(&state, row).await
}

pub async fn audience_asset_content(
    State(state): State<Arc<AppState>>,
    Path((join_code, asset_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let deck_id = deck_id_from_code(&state.db, &join_code).await?;
    if participant_from_headers(&state.db, deck_id, &headers)
        .await?
        .is_none()
    {
        return Err(AppError::Unauthorized);
    }
    let row = sqlx::query(
        r#"
        SELECT a.storage_key, a.media_type
        FROM deck_assets a
        WHERE a.id = $1 AND a.deck_id = $2
          AND EXISTS (
            SELECT 1 FROM presentation_runs r
            WHERE r.deck_id = a.deck_id AND r.status = 'live'
          )
        "#,
    )
    .bind(asset_id)
    .bind(deck_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    stored_asset_response(&state, row).await
}

async fn stored_asset_response(state: &AppState, row: PgRow) -> AppResult<Response> {
    let storage_key: String = row.try_get("storage_key")?;
    let media_type: String = row.try_get("media_type")?;
    let stored = state
        .assets
        .get(&storage_key)
        .await
        .map_err(AppError::Internal)?;
    let mut response = stored.bytes.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&media_type).map_err(|error| AppError::Internal(error.into()))?,
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    response.headers_mut().insert(
        header::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        header::HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("same-origin"),
    );
    if media_type == "image/svg+xml" {
        response.headers_mut().insert(
            header::HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"),
        );
    }
    Ok(response)
}

pub async fn delete_deck_asset(
    State(state): State<Arc<AppState>>,
    Path((deck_id, asset_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<StatusCode> {
    let user = require_user(&state, &headers).await?;
    let mut tx = state.db.begin().await?;
    let row = sqlx::query(
        r#"
        SELECT a.storage_key
        FROM deck_assets a JOIN decks d ON d.id = a.deck_id
        WHERE a.id = $1 AND a.deck_id = $2 AND d.owner_id = $3
        FOR UPDATE OF d, a
        "#,
    )
    .bind(asset_id)
    .bind(deck_id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let content_url = format!("/api/decks/{deck_id}/assets/{asset_id}/content");
    let referenced: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM decks
            WHERE id = $1
              AND (
                strpos(markdown, $2) > 0
                OR strpos(COALESCE(last_verified_markdown, ''), $2) > 0
                OR strpos(deck_css, $2) > 0
                OR strpos(COALESCE(last_verified_css, ''), $2) > 0
              )
        )
        "#,
    )
    .bind(deck_id)
    .bind(&content_url)
    .fetch_one(&mut *tx)
    .await?;
    if referenced {
        return Err(AppError::Conflict(
            "This asset is used by the current or last working deck and must be retained"
                .to_owned(),
        ));
    }
    let storage_key: String = row.try_get("storage_key")?;
    sqlx::query("INSERT INTO asset_gc (storage_key) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(&storage_key)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM deck_assets WHERE id = $1 AND deck_id = $2")
        .bind(asset_id)
        .bind(deck_id)
        .execute(&mut *tx)
        .await?;
    audit(
        &mut tx,
        Some(deck_id),
        Some(user.id),
        "deck.asset_deleted",
        json!({ "asset_id": asset_id }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

fn deck_asset(row: PgRow) -> Result<DeckAsset, sqlx::Error> {
    let id: Uuid = row.try_get("id")?;
    let deck_id: Uuid = row.try_get("deck_id")?;
    Ok(DeckAsset {
        id,
        deck_id,
        original_filename: row.try_get("original_filename")?,
        media_type: row.try_get("media_type")?,
        byte_size: row.try_get("byte_size")?,
        sha256: row.try_get("sha256")?,
        content_url: format!("/api/decks/{deck_id}/assets/{id}/content"),
        created_at: row.try_get("created_at")?,
    })
}

struct DetectedAsset {
    media_type: &'static str,
    extension: &'static str,
    label: &'static str,
    max_bytes: usize,
}

fn detect_asset(bytes: &[u8]) -> AppResult<DetectedAsset> {
    const IMAGE_MAX: usize = 10_000_000;
    const SVG_MAX: usize = 2_000_000;
    const FONT_MAX: usize = 5_000_000;
    let detected = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        DetectedAsset {
            media_type: "image/png",
            extension: "png",
            label: "Image",
            max_bytes: IMAGE_MAX,
        }
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        DetectedAsset {
            media_type: "image/jpeg",
            extension: "jpg",
            label: "Image",
            max_bytes: IMAGE_MAX,
        }
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        DetectedAsset {
            media_type: "image/gif",
            extension: "gif",
            label: "Image",
            max_bytes: IMAGE_MAX,
        }
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        DetectedAsset {
            media_type: "image/webp",
            extension: "webp",
            label: "Image",
            max_bytes: IMAGE_MAX,
        }
    } else if looks_like_svg(bytes) {
        if bytes.len() > SVG_MAX {
            return Err(AppError::BadRequest(
                "SVG files must be 2 MB or smaller".to_owned(),
            ));
        }
        validate_static_svg(bytes)?;
        DetectedAsset {
            media_type: "image/svg+xml",
            extension: "svg",
            label: "SVG",
            max_bytes: SVG_MAX,
        }
    } else if bytes.starts_with(b"wOFF") {
        DetectedAsset {
            media_type: "font/woff",
            extension: "woff",
            label: "Font",
            max_bytes: FONT_MAX,
        }
    } else if bytes.starts_with(b"wOF2") {
        DetectedAsset {
            media_type: "font/woff2",
            extension: "woff2",
            label: "Font",
            max_bytes: FONT_MAX,
        }
    } else {
        return Err(AppError::BadRequest(
            "Unsupported asset. Use PNG, JPEG, GIF, WebP, static SVG, WOFF, or WOFF2 files"
                .to_owned(),
        ));
    };
    Ok(detected)
}

fn svg_document_body(source: &str) -> Option<&str> {
    let mut rest = source
        .strip_prefix('\u{feff}')
        .unwrap_or(source)
        .trim_start();
    if rest
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("<?xml"))
    {
        rest = rest.get(rest.find("?>")? + 2..)?.trim_start();
    }
    while rest.starts_with("<!--") {
        rest = rest.get(rest.find("-->")? + 3..)?.trim_start();
    }
    let prefix = rest.get(..4)?;
    if !prefix.eq_ignore_ascii_case("<svg") {
        return None;
    }
    match rest.as_bytes().get(4) {
        Some(b'>') => Some(rest),
        Some(character) if character.is_ascii_whitespace() => Some(rest),
        _ => None,
    }
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(svg_document_body)
        .is_some()
}

fn validate_static_svg(bytes: &[u8]) -> AppResult<()> {
    static DANGEROUS_TAG: OnceLock<Regex> = OnceLock::new();
    static EVENT_ATTRIBUTE: OnceLock<Regex> = OnceLock::new();
    static RESOURCE_ATTRIBUTE: OnceLock<Regex> = OnceLock::new();
    static HREF_ATTRIBUTE: OnceLock<Regex> = OnceLock::new();
    static LOCAL_FRAGMENT: OnceLock<Regex> = OnceLock::new();
    static URL_START: OnceLock<Regex> = OnceLock::new();
    static URL_REFERENCE: OnceLock<Regex> = OnceLock::new();

    let invalid = || {
        AppError::BadRequest(
            "SVG files must be static and self-contained; scripts, animation, embedded media, and external references are not allowed"
                .to_owned(),
        )
    };
    let source = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    let body = svg_document_body(source).ok_or_else(invalid)?;
    let lower = body.to_ascii_lowercase();

    if source.contains('\0')
        || source.contains('\\')
        || lower.contains("<!doctype")
        || lower.contains("<!entity")
        || lower.contains("<?")
        || ["@import", "expression(", "-moz-binding", "behavior:"]
            .iter()
            .any(|value| lower.contains(value))
    {
        return Err(invalid());
    }

    let dangerous_tag = DANGEROUS_TAG.get_or_init(|| {
        Regex::new(
            r"(?i)<\s*/?\s*(?:[a-z0-9_.-]+\s*:\s*)?(?:script|foreignobject|iframe|object|embed|audio|video|image|feimage|animate|animatemotion|animatetransform|set|discard)\b",
        )
        .expect("dangerous SVG tag regex should compile")
    });
    let event_attribute = EVENT_ATTRIBUTE.get_or_init(|| {
        Regex::new(r"(?i)\bon[a-z0-9_.-]*\s*=").expect("SVG event attribute regex should compile")
    });
    let resource_attribute = RESOURCE_ATTRIBUTE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:src|xml\s*:\s*base)\s*=")
            .expect("SVG resource attribute regex should compile")
    });
    if dangerous_tag.is_match(body)
        || event_attribute.is_match(body)
        || resource_attribute.is_match(body)
    {
        return Err(invalid());
    }

    let local_fragment = LOCAL_FRAGMENT.get_or_init(|| {
        Regex::new(r"\A#[A-Za-z_][A-Za-z0-9_.:-]*\z")
            .expect("SVG local fragment regex should compile")
    });
    let href_attribute = HREF_ATTRIBUTE.get_or_init(|| {
        Regex::new(r#"(?is)\b(?:[a-z0-9_.-]+\s*:\s*)?href\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#)
            .expect("SVG href attribute regex should compile")
    });
    for captures in href_attribute.captures_iter(body) {
        let value = captures
            .get(1)
            .or_else(|| captures.get(2))
            .or_else(|| captures.get(3))
            .map(|capture| capture.as_str().trim())
            .unwrap_or("");
        if !local_fragment.is_match(value) {
            return Err(invalid());
        }
    }

    let url_start = URL_START
        .get_or_init(|| Regex::new(r"(?i)url\s*\(").expect("SVG URL start regex should compile"));
    let url_reference = URL_REFERENCE.get_or_init(|| {
        Regex::new(r"(?is)url\s*\(\s*([^)]*?)\s*\)")
            .expect("SVG URL reference regex should compile")
    });
    let references = url_reference.captures_iter(body).collect::<Vec<_>>();
    if url_start.find_iter(body).count() != references.len() {
        return Err(invalid());
    }
    for captures in references {
        let value = captures
            .get(1)
            .map(|capture| capture.as_str().trim().trim_matches(['\'', '"']))
            .unwrap_or("");
        if !local_fragment.is_match(value) {
            return Err(invalid());
        }
    }
    Ok(())
}

fn display_filename(value: &str) -> AppResult<String> {
    let filename = FsPath::new(value)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .chars()
        .filter(|character| !character.is_control())
        .take(160)
        .collect::<String>();
    if filename.trim().is_empty() {
        return Err(AppError::BadRequest(
            "The uploaded asset needs a valid filename".to_owned(),
        ));
    }
    Ok(filename)
}

fn storage_filename(original: &str, extension: &str) -> String {
    let stem = FsPath::new(original)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("asset");
    let mut normalized = String::new();
    let mut separator = false;
    for character in stem.chars().flat_map(char::to_lowercase).take(70) {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
            normalized.push(character);
            separator = false;
        } else if !separator && !normalized.is_empty() {
            normalized.push('-');
            separator = true;
        }
    }
    let stem = normalized.trim_matches('-');
    format!(
        "{}.{}",
        if stem.is_empty() { "asset" } else { stem },
        extension
    )
}

fn csv_cell(value: &str) -> String {
    // Spreadsheet applications may interpret a downloaded cell beginning with
    // one of these characters as a formula. Prefix user-controlled values while
    // retaining their visible content and always apply RFC 4180 quoting.
    let protected = if value.trim_start().starts_with(['=', '+', '-', '@']) {
        format!("'{value}")
    } else {
        value.to_owned()
    };
    format!("\"{}\"", protected.replace('"', "\"\""))
}

async fn persist_deck_draft(
    state: &AppState,
    deck_id: Uuid,
    user_id: Uuid,
    markdown: &str,
    css: Option<&str>,
    title: Option<&str>,
    expected_version: i64,
) -> AppResult<DeckDraftSave> {
    let mut tx = state.db.begin().await?;
    let deck = sqlx::query(
        r#"
        SELECT d.title, d.markdown, d.deck_css, d.version, d.verified_version,
               EXISTS(SELECT 1 FROM presentation_runs p WHERE p.deck_id = d.id AND p.status = 'live') AS is_live
        FROM decks d
        WHERE d.id = $1 AND d.owner_id = $2
        FOR UPDATE OF d
        "#,
    )
    .bind(deck_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let current_version: i64 = deck.try_get("version")?;
    if current_version != expected_version {
        return Err(AppError::Conflict(
            "This draft changed in another tab; reload before saving".to_owned(),
        ));
    }
    if deck.try_get::<bool, _>("is_live")? {
        return Err(AppError::Conflict(
            "End the live presentation before editing this deck".to_owned(),
        ));
    }
    // Report an outdated editor tab before validating its source. This keeps a
    // stale local draft from surfacing a misleading Markdown error after a
    // newer, repaired version has already been saved on the server.
    validate_deck_headmatter(markdown).map_err(AppError::BadRequest)?;
    let current_title: String = deck.try_get("title")?;
    let next_title = title.unwrap_or(&current_title);
    let current_markdown: String = deck.try_get("markdown")?;
    let current_css: String = deck.try_get("deck_css")?;
    let next_css = css.unwrap_or(&current_css);
    let verified_version: Option<i64> = deck.try_get("verified_version")?;
    if current_markdown == markdown && current_css == next_css && current_title == next_title {
        tx.commit().await?;
        return Ok(DeckDraftSave {
            version: current_version,
            verified_version,
            is_verified: verified_version == Some(current_version),
        });
    }

    let version: i64 = sqlx::query_scalar(
        r#"
        UPDATE decks
        SET markdown = $1, deck_css = $2, title = $3, version = version + 1, updated_at = now()
        WHERE id = $4 AND owner_id = $5 AND version = $6
        RETURNING version
        "#,
    )
    .bind(markdown)
    .bind(next_css)
    .bind(next_title)
    .bind(deck_id)
    .bind(user_id)
    .bind(expected_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        AppError::Conflict("This draft changed in another tab; reload before saving".to_owned())
    })?;
    tx.commit().await?;
    Ok(DeckDraftSave {
        version,
        verified_version,
        is_verified: verified_version == Some(version),
    })
}

pub async fn slidev_access(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    Query(query): Query<SlidevAccessQuery>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let user = require_user(&state, &headers).await?;
    let mode = query.mode.as_deref().unwrap_or("preview");
    if !matches!(mode, "preview" | "present" | "export") {
        return Err(AppError::BadRequest(
            "Invalid Slidev access mode".to_owned(),
        ));
    }
    let version = if mode != "present" {
        sqlx::query_scalar::<_, i64>("SELECT version FROM decks WHERE id = $1 AND owner_id = $2")
            .bind(deck_id)
            .bind(user.id)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::NotFound)?
    } else {
        sqlx::query_scalar::<_, i64>(
            r#"
            SELECT r.source_revision
            FROM decks d JOIN presentation_runs r ON r.deck_id = d.id AND r.status = 'live'
            WHERE d.id = $1 AND d.owner_id = $2
            "#,
        )
        .bind(deck_id)
        .bind(user.id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| {
            AppError::Conflict("Start the presentation before requesting display access".to_owned())
        })?
    };
    // Preview and presentation use the always-on browser renderer. Explicit
    // export remains the one operation that launches the upstream Slidev
    // worker so its standard exporter stays fully compatible.
    let renderer_mode = "preview";
    let token = sign_slidev_token(&state, deck_id, version, renderer_mode)?;
    let url = if mode == "export" {
        format!("/slidev/{renderer_mode}/{deck_id}/?access={token}")
    } else {
        format!("/decks/{deck_id}/player?version={version}&access={token}")
    };
    Ok(Json(json!({
        "url": url,
        "version": version
    })))
}

pub async fn internal_deck_source(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    Query(query): Query<InternalSourceQuery>,
    headers: HeaderMap,
) -> AppResult<Json<SlidevSource>> {
    require_internal(&state, &headers)?;

    let row = if let Some(version) = query.version {
        sqlx::query(
            r#"
            SELECT title, join_code, markdown, deck_css, version
            FROM decks
            WHERE id = $1 AND version = $2
            "#,
        )
        .bind(deck_id)
        .bind(version)
        .fetch_optional(&state.db)
        .await?
    } else {
        sqlx::query("SELECT title, join_code, markdown, deck_css, version FROM decks WHERE id = $1")
            .bind(deck_id)
            .fetch_optional(&state.db)
            .await?
    }
    .ok_or(AppError::NotFound)?;

    let join_code: String = row.try_get("join_code")?;
    Ok(Json(SlidevSource {
        deck_id,
        title: row.try_get("title")?,
        markdown: row.try_get("markdown")?,
        css: row.try_get("deck_css")?,
        version: row.try_get("version")?,
        join_url: format!("{}/j/{join_code}", state.config.app_base_url),
    }))
}

pub async fn internal_record_deck_build(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<RecordDeckBuildRequest>,
) -> AppResult<Json<Value>> {
    require_internal(&state, &headers)?;
    if !matches!(request.status.as_str(), "succeeded" | "failed") {
        return Err(AppError::BadRequest("Invalid build status".to_owned()));
    }
    if request.startup_ms.is_some_and(|value| value < 0) {
        return Err(AppError::BadRequest("Invalid build duration".to_owned()));
    }
    let bounded = |value: Option<String>, max: usize| {
        value.map(|value| value.chars().take(max).collect::<String>())
    };
    let slidev_version = bounded(request.slidev_version, 80);
    let theme = bounded(request.theme, 120);
    let diagnostic_code = bounded(request.diagnostic_code, 120);
    let diagnostic_message = bounded(request.diagnostic_message, 1_000);
    let mut tx = state.db.begin().await?;
    let source = sqlx::query(
        "SELECT markdown, deck_css FROM decks WHERE id = $1 AND version = $2 FOR UPDATE",
    )
    .bind(deck_id)
    .bind(request.source_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let markdown: String = source.try_get("markdown")?;
    let css: String = source.try_get("deck_css")?;
    let parsed = if request.status == "succeeded" {
        Some(parse_deck(&markdown).map_err(AppError::BadRequest)?)
    } else {
        None
    };
    if let Some(parsed) = &parsed {
        ensure_response_contract_integrity(&mut tx, deck_id, &parsed.interactions).await?;
    }
    let source_sha256 = format!(
        "{:x}",
        Sha256::digest(format!("{markdown}\0{css}").as_bytes())
    );
    let row = sqlx::query(
        r#"
        INSERT INTO deck_builds (
            id, deck_id, source_revision, status, source_sha256, slidev_version, theme,
            startup_ms, diagnostic_code, diagnostic_message
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        ON CONFLICT (deck_id, source_revision) DO UPDATE
        SET status = EXCLUDED.status,
            source_sha256 = EXCLUDED.source_sha256,
            slidev_version = EXCLUDED.slidev_version,
            theme = EXCLUDED.theme,
            startup_ms = EXCLUDED.startup_ms,
            diagnostic_code = EXCLUDED.diagnostic_code,
            diagnostic_message = EXCLUDED.diagnostic_message,
            updated_at = now(),
            completed_at = now()
        WHERE deck_builds.status <> 'succeeded' OR EXCLUDED.status = 'succeeded'
        RETURNING id, status, source_sha256, completed_at
        "#,
    )
    .bind(Uuid::now_v7())
    .bind(deck_id)
    .bind(request.source_version)
    .bind(&request.status)
    .bind(&source_sha256)
    .bind(slidev_version)
    .bind(theme)
    .bind(request.startup_ms)
    .bind(diagnostic_code)
    .bind(diagnostic_message)
    .fetch_optional(&mut *tx)
    .await?;
    let row = match row {
        Some(row) => row,
        None => {
            sqlx::query(
                "SELECT id, status, source_sha256, completed_at FROM deck_builds WHERE deck_id = $1 AND source_revision = $2",
            )
            .bind(deck_id)
            .bind(request.source_version)
            .fetch_one(&mut *tx)
            .await?
        }
    };
    if let Some(parsed) = parsed {
        sqlx::query(
            r#"
            UPDATE decks
            SET verified_version = $1, last_verified_markdown = $2, last_verified_css = $3
            WHERE id = $4 AND version = $1
            "#,
        )
        .bind(request.source_version)
        .bind(&markdown)
        .bind(&css)
        .bind(deck_id)
        .execute(&mut *tx)
        .await?;
        sync_interactions(
            &mut tx,
            deck_id,
            request.source_version,
            &parsed.interactions,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({
        "id": row.try_get::<Uuid, _>("id")?,
        "status": row.try_get::<String, _>("status")?,
        "source_version": request.source_version,
        "source_sha256": row.try_get::<String, _>("source_sha256")?,
        "completed_at": row.try_get::<chrono::DateTime<Utc>, _>("completed_at")?,
    })))
}

pub async fn start_presentation(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    start_presentation_mode(state, deck_id, headers, "live").await
}

pub async fn start_rehearsal(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    start_presentation_mode(state, deck_id, headers, "rehearsal").await
}

async fn start_presentation_mode(
    state: Arc<AppState>,
    deck_id: Uuid,
    headers: HeaderMap,
    requested_mode: &str,
) -> AppResult<Json<Value>> {
    let user = require_user(&state, &headers).await?;
    let detail = fetch_deck_detail(&state.db, deck_id, user.id).await?;
    if !detail.is_verified {
        return Err(AppError::Conflict(
            "The latest saved version has not passed the Slidev preview check".to_owned(),
        ));
    }
    let first_slide = detail
        .slides
        .first()
        .ok_or_else(|| AppError::BadRequest("Deck has no slides".to_owned()))?;

    let mut tx = state.db.begin().await?;
    let locked = sqlx::query(
        "SELECT version, verified_version FROM decks WHERE id = $1 AND owner_id = $2 FOR UPDATE",
    )
    .bind(deck_id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let locked_version: i64 = locked.try_get("version")?;
    let locked_verified_version: Option<i64> = locked.try_get("verified_version")?;
    if locked_version != detail.version || locked_verified_version != Some(detail.version) {
        return Err(AppError::Conflict(
            "The deck changed while the presentation was starting; try again".to_owned(),
        ));
    }

    if let Some(existing) = sqlx::query(
        "SELECT id, source_revision, run_mode, result_epoch FROM presentation_runs WHERE deck_id = $1 AND status = 'live' FOR UPDATE",
    )
    .bind(deck_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        let run_id: Uuid = existing.try_get("id")?;
        let version: i64 = existing.try_get("source_revision")?;
        let run_mode: String = existing.try_get("run_mode")?;
        let result_epoch: i32 = existing.try_get("result_epoch")?;
        if run_mode == requested_mode && version == detail.version {
            tx.commit().await?;
            return Ok(Json(json!({
                "run_id": run_id,
                "resumed": true,
                "run_mode": run_mode,
                "result_epoch": result_epoch,
                "url": format!("/decks/{deck_id}/player?version={version}")
            })));
        }

        sqlx::query(
            "UPDATE presentation_runs SET status = 'ended', ended_at = now() WHERE id = $1",
        )
        .bind(run_id)
        .execute(&mut *tx)
        .await?;
        audit(
            &mut tx,
            Some(deck_id),
            Some(user.id),
            "presentation.replaced",
            json!({
                "previous_run_id": run_id,
                "previous_version": version,
                "previous_mode": run_mode,
                "new_version": detail.version,
                "new_mode": requested_mode,
            }),
        )
        .await?;
    }
    let result_epoch = if requested_mode == "rehearsal" {
        let epoch: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(epoch), 0)::integer + 1 FROM result_epochs WHERE deck_id = $1",
        )
        .bind(deck_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO result_epochs (deck_id, epoch, created_by, reason) VALUES ($1, $2, $3, 'rehearsal')",
        )
        .bind(deck_id)
        .bind(epoch)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
        epoch
    } else {
        detail.result_epoch
    };
    let run_id = Uuid::now_v7();
    sqlx::query(
        r#"
        INSERT INTO presentation_runs (
            id, deck_id, source_revision, result_epoch, status, started_by, run_mode
        )
        VALUES ($1, $2, $3, $4, 'live', $5, $6)
        "#,
    )
    .bind(run_id)
    .bind(deck_id)
    .bind(detail.version)
    .bind(result_epoch)
    .bind(user.id)
    .bind(requested_mode)
    .execute(&mut *tx)
    .await?;
    let sequence: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO live_deck_states (
            deck_id, run_id, slide_key, slide_number, click_step,
            visible_element_interaction_ids, sequence
        )
        VALUES ($1, $2, $3, $4, 0, '[]'::jsonb, 1)
        ON CONFLICT (deck_id) DO UPDATE
        SET run_id = EXCLUDED.run_id, slide_key = EXCLUDED.slide_key,
            slide_number = EXCLUDED.slide_number, click_step = 0,
            visible_element_interaction_ids = '[]'::jsonb,
            sequence = live_deck_states.sequence + 1, updated_at = now()
        RETURNING sequence
        "#,
    )
    .bind(deck_id)
    .bind(run_id)
    .bind(&first_slide.key)
    .bind(first_slide.number)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO interaction_live_states (run_id, interaction_key, phase)
        SELECT $1, interaction_key, 'voting'
        FROM interaction_definitions
        WHERE deck_id = $2 AND source_revision = $3
          AND is_archived = false
        ON CONFLICT (run_id, interaction_key) DO NOTHING
        "#,
    )
    .bind(run_id)
    .bind(deck_id)
    .bind(detail.version)
    .execute(&mut *tx)
    .await?;
    audit(
        &mut tx,
        Some(deck_id),
        Some(user.id),
        "presentation.started",
        json!({ "run_id": run_id, "run_mode": requested_mode, "result_epoch": result_epoch }),
    )
    .await?;
    tx.commit().await?;
    state.notify(deck_id, sequence);

    Ok(Json(json!({
        "run_id": run_id,
        "resumed": false,
        "run_mode": requested_mode,
        "result_epoch": result_epoch,
        "url": format!("/decks/{deck_id}/player?version={}", detail.version)
    })))
}

pub async fn stop_presentation(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let user = require_user(&state, &headers).await?;
    assert_owner(&state.db, deck_id, user.id).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE presentation_runs SET status = 'ended', ended_at = now() WHERE deck_id = $1 AND status = 'live'")
        .bind(deck_id)
        .execute(&mut *tx)
        .await?;
    let sequence: Option<i64> = sqlx::query_scalar(
        "UPDATE live_deck_states SET sequence = sequence + 1, updated_at = now() WHERE deck_id = $1 RETURNING sequence",
    )
    .bind(deck_id)
    .fetch_optional(&mut *tx)
    .await?;
    audit(
        &mut tx,
        Some(deck_id),
        Some(user.id),
        "presentation.ended",
        json!({}),
    )
    .await?;
    tx.commit().await?;
    state.notify(deck_id, sequence.unwrap_or(0));
    Ok(Json(json!({ "ok": true })))
}

pub async fn control_input(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<InputControlRequest>,
) -> AppResult<Json<Value>> {
    let user = require_user(&state, &headers).await?;
    assert_owner(&state.db, deck_id, user.id).await?;
    let frozen = match request.action.as_str() {
        "freeze" => true,
        "resume" => false,
        _ => {
            return Err(AppError::BadRequest(
                "Action must be freeze or resume".to_owned(),
            ));
        }
    };
    let mut tx = state.db.begin().await?;
    let run_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        UPDATE presentation_runs
        SET input_frozen = $1
        WHERE deck_id = $2 AND status = 'live'
        RETURNING id
        "#,
    )
    .bind(frozen)
    .bind(deck_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::Conflict("Presentation is not live".to_owned()))?;
    let sequence: i64 = sqlx::query_scalar(
        "UPDATE live_deck_states SET sequence = sequence + 1, updated_at = now() WHERE deck_id = $1 RETURNING sequence",
    )
    .bind(deck_id)
    .fetch_one(&mut *tx)
    .await?;
    audit(
        &mut tx,
        Some(deck_id),
        Some(user.id),
        "presentation.input_controlled",
        json!({ "run_id": run_id, "input_frozen": frozen }),
    )
    .await?;
    tx.commit().await?;
    state.notify(deck_id, sequence);
    Ok(Json(
        json!({ "input_frozen": frozen, "sequence": sequence }),
    ))
}

pub async fn navigate(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<NavigateRequest>,
) -> AppResult<Json<Value>> {
    let user = require_user(&state, &headers).await?;
    assert_owner(&state.db, deck_id, user.id).await?;
    let run = sqlx::query(
        r#"
        SELECT r.source_revision, d.markdown
        FROM presentation_runs r
        JOIN decks d ON d.id = r.deck_id AND d.version = r.source_revision
        WHERE r.deck_id = $1 AND r.status = 'live'
        "#,
    )
    .bind(deck_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Conflict("Presentation is not live".to_owned()))?;
    let markdown: String = run.try_get("markdown")?;
    let parsed = parse_deck(&markdown).map_err(AppError::BadRequest)?;
    let slide = parsed
        .slides
        .iter()
        .find(|slide| slide.number == request.slide_number)
        .ok_or_else(|| AppError::BadRequest("Slide number is out of range".to_owned()))?;
    let valid_element_ids = parsed
        .interactions
        .iter()
        .filter(|interaction| interaction.slide_key == slide.key && interaction.element)
        .map(|interaction| interaction.id.as_str())
        .collect::<HashSet<_>>();
    if request
        .visible_element_interaction_ids
        .iter()
        .any(|id| !valid_element_ids.contains(id.as_str()))
    {
        return Err(AppError::BadRequest(
            "Visible element interactions do not belong to the active slide".to_owned(),
        ));
    }
    let visible_element_interaction_ids = request
        .visible_element_interaction_ids
        .iter()
        .filter(|id| valid_element_ids.contains(id.as_str()))
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let sequence: i64 = sqlx::query_scalar(
        r#"
        UPDATE live_deck_states
        SET slide_key = $1, slide_number = $2, click_step = $3,
            visible_element_interaction_ids = $4,
            sequence = sequence + 1, updated_at = now()
        WHERE deck_id = $5
        RETURNING sequence
        "#,
    )
    .bind(&slide.key)
    .bind(slide.number)
    .bind(request.click_step.max(0))
    .bind(serde_json::to_value(visible_element_interaction_ids).unwrap_or_else(|_| json!([])))
    .bind(deck_id)
    .fetch_one(&state.db)
    .await?;
    state.notify(deck_id, sequence);
    Ok(Json(json!({ "sequence": sequence, "slide": slide })))
}

pub async fn control_interaction(
    State(state): State<Arc<AppState>>,
    Path((deck_id, interaction_id)): Path<(Uuid, String)>,
    headers: HeaderMap,
    Json(request): Json<InteractionControlRequest>,
) -> AppResult<Json<Value>> {
    let user = require_user(&state, &headers).await?;
    assert_owner(&state.db, deck_id, user.id).await?;
    let context = sqlx::query(
        r#"
        SELECT l.run_id, l.click_step, i.kind, i.options, i.config,
               COALESCE(s.phase, 'voting') AS phase,
               COALESCE(s.accepting_responses, true) AS accepting_responses,
               COALESCE(s.results_revealed, false) AS results_revealed,
               s.timer_ends_at, s.timer_remaining_seconds,
               clock_timestamp() AS database_now
        FROM live_deck_states l
        JOIN presentation_runs r ON r.id = l.run_id AND r.status = 'live'
        JOIN interaction_definitions i
          ON i.deck_id = l.deck_id AND i.interaction_key = $2
         AND i.slide_key = l.slide_key AND i.is_archived = false
        LEFT JOIN interaction_live_states s
          ON s.run_id = l.run_id AND s.interaction_key = i.interaction_key
        WHERE l.deck_id = $1
        "#,
    )
    .bind(deck_id)
    .bind(&interaction_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Conflict("This interaction is not on the current slide".to_owned()))?;
    let kind: String = context.try_get("kind")?;
    let run_id: Uuid = context.try_get("run_id")?;
    let current_phase: String = context.try_get("phase")?;
    let current_accepting: bool = context.try_get("accepting_responses")?;
    let current_results_revealed: bool = context.try_get("results_revealed")?;
    let current_timer_ends_at: Option<DateTime<Utc>> = context.try_get("timer_ends_at")?;
    let current_timer_remaining: Option<i32> = context.try_get("timer_remaining_seconds")?;
    let database_now: DateTime<Utc> = context.try_get("database_now")?;
    let config: Value = context.try_get("config")?;
    let options: Value = context.try_get("options")?;
    let click_step: i32 = context.try_get("click_step")?;
    let requires_click_reveal = config.get("reveal").and_then(Value::as_str) == Some("click");
    let option_count = options.as_array().map_or(0, Vec::len) as i32;
    let timer_duration = config
        .get("timer")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<i32>().ok());
    let control = if request.action.starts_with("timer-") {
        let duration = timer_duration.ok_or_else(|| {
            AppError::BadRequest("This interaction does not configure a timer".to_owned())
        })?;
        match request.action.as_str() {
            "timer-start" => {
                let seconds = current_timer_remaining
                    .filter(|remaining| *remaining > 0)
                    .unwrap_or(duration);
                InteractionControl {
                    phase: current_phase.clone(),
                    accepting_responses: current_accepting,
                    replays_reveal: false,
                    results_revealed: current_results_revealed,
                    timer_ends_at: Some(database_now + Duration::seconds(seconds.into())),
                    timer_remaining_seconds: None,
                }
            }
            "timer-pause" => {
                let ends_at = current_timer_ends_at
                    .ok_or_else(|| AppError::Conflict("The countdown is not running".to_owned()))?;
                let milliseconds = (ends_at - database_now).num_milliseconds().max(0);
                InteractionControl {
                    phase: current_phase.clone(),
                    accepting_responses: current_accepting,
                    replays_reveal: false,
                    results_revealed: current_results_revealed,
                    timer_ends_at: None,
                    timer_remaining_seconds: Some(((milliseconds + 999) / 1000) as i32),
                }
            }
            "timer-reset" => InteractionControl {
                phase: current_phase.clone(),
                accepting_responses: current_accepting,
                replays_reveal: false,
                results_revealed: current_results_revealed,
                timer_ends_at: None,
                timer_remaining_seconds: Some(duration),
            },
            _ => {
                return Err(AppError::BadRequest(
                    "Timer action must be start, pause, or reset".to_owned(),
                ));
            }
        }
    } else {
        resolve_interaction_control(
            &kind,
            &request.action,
            &current_phase,
            current_accepting,
            current_results_revealed,
            config.get("results").and_then(Value::as_str) == Some("on-close"),
            current_timer_ends_at,
            current_timer_remaining,
            requires_click_reveal,
            click_step,
            option_count,
        )?
    };

    let mut tx = state.db.begin().await?;
    sqlx::query(
        r#"
        INSERT INTO interaction_live_states (
            run_id, interaction_key, phase, accepting_responses, results_revealed,
            timer_ends_at, timer_remaining_seconds
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (run_id, interaction_key) DO UPDATE
        SET phase = EXCLUDED.phase,
            accepting_responses = EXCLUDED.accepting_responses,
            results_revealed = EXCLUDED.results_revealed,
            timer_ends_at = EXCLUDED.timer_ends_at,
            timer_remaining_seconds = EXCLUDED.timer_remaining_seconds,
            updated_at = now()
        "#,
    )
    .bind(run_id)
    .bind(&interaction_id)
    .bind(&control.phase)
    .bind(control.accepting_responses)
    .bind(control.results_revealed)
    .bind(control.timer_ends_at)
    .bind(control.timer_remaining_seconds)
    .execute(&mut *tx)
    .await?;
    let sequence: i64 = sqlx::query_scalar(
        "UPDATE live_deck_states SET click_step = CASE WHEN $2 THEN 0 ELSE click_step END, sequence = sequence + 1, updated_at = now() WHERE deck_id = $1 RETURNING sequence",
    )
    .bind(deck_id)
    .bind(control.replays_reveal)
    .fetch_one(&mut *tx)
    .await?;
    audit(
        &mut tx,
        Some(deck_id),
        Some(user.id),
        "interaction.controlled",
        json!({
            "interaction_id": interaction_id,
            "action": request.action,
            "phase": control.phase,
            "accepting_responses": control.accepting_responses,
            "results_revealed": control.results_revealed,
            "timer_remaining_seconds": control.timer_remaining_seconds,
        }),
    )
    .await?;
    tx.commit().await?;
    state.notify(deck_id, sequence);
    Ok(Json(json!({
        "phase": control.phase,
        "accepting_responses": control.accepting_responses,
        "results_revealed": control.results_revealed,
        "timer_running": control.timer_ends_at.is_some(),
        "timer_remaining_seconds": control.timer_remaining_seconds,
        "sequence": sequence,
    })))
}

/// Resolves a presenter control action into the run-scoped state an interaction
/// should hold. Ranked lists move through reveal/vote/rank phases; every other
/// kind keeps its phase and only toggles whether it accepts new responses.
#[allow(clippy::too_many_arguments)]
fn resolve_interaction_control(
    kind: &str,
    action: &str,
    current_phase: &str,
    current_accepting: bool,
    current_results_revealed: bool,
    reveal_on_close: bool,
    timer_ends_at: Option<DateTime<Utc>>,
    timer_remaining_seconds: Option<i32>,
    requires_click_reveal: bool,
    click_step: i32,
    option_count: i32,
) -> AppResult<InteractionControl> {
    let unrevealed = requires_click_reveal && click_step < option_count;
    if matches!(action, "reveal" | "hide") {
        return Ok(InteractionControl {
            phase: current_phase.to_owned(),
            accepting_responses: current_accepting,
            replays_reveal: false,
            results_revealed: action == "reveal",
            timer_ends_at,
            timer_remaining_seconds,
        });
    }
    if kind != "ranked-list" {
        return match action {
            "open" | "reset" => Ok(InteractionControl {
                phase: current_phase.to_owned(),
                accepting_responses: true,
                replays_reveal: false,
                results_revealed: if action == "reset" {
                    false
                } else {
                    current_results_revealed
                },
                timer_ends_at,
                timer_remaining_seconds,
            }),
            "close" => Ok(InteractionControl {
                phase: current_phase.to_owned(),
                accepting_responses: false,
                replays_reveal: false,
                results_revealed: reveal_on_close || current_results_revealed,
                timer_ends_at,
                timer_remaining_seconds,
            }),
            _ => Err(AppError::BadRequest(
                "Action must be open, close, reset, reveal, or hide".to_owned(),
            )),
        };
    }
    match action {
        "open" => {
            if unrevealed {
                return Err(AppError::Conflict(format!(
                    "Reveal all {option_count} items before opening voting"
                )));
            }
            Ok(InteractionControl {
                phase: "voting".to_owned(),
                accepting_responses: true,
                replays_reveal: false,
                results_revealed: current_results_revealed,
                timer_ends_at,
                timer_remaining_seconds,
            })
        }
        "close" => {
            if current_phase != "voting" {
                return Err(AppError::Conflict("Voting is not open".to_owned()));
            }
            if unrevealed {
                return Err(AppError::Conflict(format!(
                    "Reveal all {option_count} items before closing voting"
                )));
            }
            Ok(InteractionControl {
                phase: "ranked".to_owned(),
                accepting_responses: false,
                replays_reveal: false,
                results_revealed: true,
                timer_ends_at,
                timer_remaining_seconds,
            })
        }
        "reset" => Ok(InteractionControl {
            phase: "voting".to_owned(),
            accepting_responses: true,
            replays_reveal: true,
            results_revealed: false,
            timer_ends_at,
            timer_remaining_seconds,
        }),
        _ => Err(AppError::BadRequest(
            "Action must be open, close, reset, reveal, or hide".to_owned(),
        )),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct InteractionControl {
    phase: String,
    accepting_responses: bool,
    replays_reveal: bool,
    results_revealed: bool,
    timer_ends_at: Option<DateTime<Utc>>,
    timer_remaining_seconds: Option<i32>,
}

pub async fn reset_results(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let user = require_user(&state, &headers).await?;
    assert_owner(&state.db, deck_id, user.id).await?;
    let mut tx = state.db.begin().await?;
    let run = sqlx::query(
        "SELECT id, run_mode FROM presentation_runs WHERE deck_id = $1 AND status = 'live' FOR UPDATE",
    )
    .bind(deck_id)
    .fetch_optional(&mut *tx)
    .await?;
    let run_id = run
        .as_ref()
        .map(|row| row.try_get::<Uuid, _>("id"))
        .transpose()?;
    let run_mode = run
        .as_ref()
        .map(|row| row.try_get::<String, _>("run_mode"))
        .transpose()?;
    let epoch: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(epoch), 0)::integer + 1 FROM result_epochs WHERE deck_id = $1",
    )
    .bind(deck_id)
    .fetch_one(&mut *tx)
    .await?;
    if run_mode.as_deref() != Some("rehearsal") {
        sqlx::query("UPDATE decks SET result_epoch = $1, updated_at = now() WHERE id = $2")
            .bind(epoch)
            .bind(deck_id)
            .execute(&mut *tx)
            .await?;
    }
    let reason = match run_mode.as_deref() {
        Some("rehearsal") => "rehearsal reset",
        Some("live") => "creator live reset",
        _ => "creator settings reset",
    };
    sqlx::query(
        "INSERT INTO result_epochs (deck_id, epoch, created_by, reason) VALUES ($1, $2, $3, $4)",
    )
    .bind(deck_id)
    .bind(epoch)
    .bind(user.id)
    .bind(reason)
    .execute(&mut *tx)
    .await?;
    let sequence = if let Some(run_id) = run_id {
        sqlx::query(
            "UPDATE presentation_runs SET result_epoch = $1, input_frozen = false WHERE id = $2",
        )
        .bind(epoch)
        .bind(run_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            UPDATE interaction_live_states s
            SET phase = 'voting', accepting_responses = true, results_revealed = false,
                timer_ends_at = NULL, timer_remaining_seconds = NULL, updated_at = now()
            FROM presentation_runs r
            WHERE s.run_id = r.id AND r.deck_id = $1 AND r.status = 'live'
            "#,
        )
        .bind(deck_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query_scalar(
            "UPDATE live_deck_states SET click_step = 0, sequence = sequence + 1, updated_at = now() WHERE deck_id = $1 RETURNING sequence",
        )
        .bind(deck_id)
        .fetch_optional(&mut *tx)
        .await?
    } else {
        None
    };
    audit(
        &mut tx,
        Some(deck_id),
        Some(user.id),
        "results.reset",
        json!({ "epoch": epoch, "run_mode": run_mode.as_deref().unwrap_or("inactive") }),
    )
    .await?;
    tx.commit().await?;
    if let Some(sequence) = sequence {
        state.notify(deck_id, sequence);
    }
    Ok(Json(json!({ "epoch": epoch })))
}

pub async fn update_qa_settings(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<QaSettingsRequest>,
) -> AppResult<Json<Value>> {
    let user = require_user(&state, &headers).await?;
    assert_owner(&state.db, deck_id, user.id).await?;
    if !matches!(request.display_mode.as_str(), "verbatim" | "ai_grouped") {
        return Err(AppError::BadRequest(
            "Q&A display mode must be verbatim or ai_grouped".to_owned(),
        ));
    }

    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE decks SET qa_display_mode = $1, updated_at = now() WHERE id = $2")
        .bind(&request.display_mode)
        .bind(deck_id)
        .execute(&mut *tx)
        .await?;
    audit(
        &mut tx,
        Some(deck_id),
        Some(user.id),
        "deck.qa_display_mode_changed",
        json!({ "display_mode": request.display_mode }),
    )
    .await?;
    let sequence: Option<i64> = sqlx::query_scalar(
        "UPDATE live_deck_states SET sequence = sequence + 1, updated_at = now() WHERE deck_id = $1 RETURNING sequence",
    )
    .bind(deck_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    if let Some(sequence) = sequence {
        state.notify(deck_id, sequence);
    }
    Ok(Json(json!({ "qa_display_mode": request.display_mode })))
}

pub async fn generate_qa_insights(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<(StatusCode, Json<Value>)> {
    let user = require_user(&state, &headers).await?;
    assert_owner(&state.db, deck_id, user.id).await?;
    if state.config.gemini_moderator_key.is_none() {
        return Err(AppError::Conflict(
            "Gemini moderation is not configured".to_owned(),
        ));
    }

    let result_epoch: i32 = sqlx::query_scalar("SELECT result_epoch FROM decks WHERE id = $1")
        .bind(deck_id)
        .fetch_one(&state.db)
        .await?;
    let question_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM questions
        WHERE deck_id = $1 AND result_epoch = $2
          AND moderation_status = 'approved'
          AND lifecycle_status <> 'archived'
          AND analysis_status = 'complete'
        "#,
    )
    .bind(deck_id)
    .bind(result_epoch)
    .fetch_one(&state.db)
    .await?;
    if question_count == 0 {
        return Err(AppError::BadRequest(
            "There are no approved, analyzed questions to summarize yet".to_owned(),
        ));
    }

    let mut tx = state.db.begin().await?;
    let queued = crate::ai::enqueue_insights(&mut tx, deck_id, result_epoch).await?;
    audit(
        &mut tx,
        Some(deck_id),
        Some(user.id),
        "deck.qa_insights_requested",
        json!({ "result_epoch": result_epoch, "question_count": question_count, "queued": queued }),
    )
    .await?;
    let sequence: Option<i64> = sqlx::query_scalar(
        "UPDATE live_deck_states SET sequence = sequence + 1, updated_at = now() WHERE deck_id = $1 RETURNING sequence",
    )
    .bind(deck_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    if let Some(sequence) = sequence {
        state.notify(deck_id, sequence);
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "queued": queued,
            "status": if queued { "queued" } else { "already_in_progress" },
            "question_count": question_count,
        })),
    ))
}

pub async fn presenter_state(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Json<LiveState>> {
    let user = require_user(&state, &headers).await?;
    assert_owner(&state.db, deck_id, user.id).await?;
    Ok(Json(
        build_live_state(
            &state.db,
            deck_id,
            None,
            true,
            state.config.gemini_moderator_key.is_some(),
            Some(&state.config.ai_moderation_mode),
        )
        .await?,
    ))
}

pub async fn live_summary(
    State(state): State<Arc<AppState>>,
    Path(deck_id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Json<LiveSummary>> {
    let user = require_user(&state, &headers).await?;
    assert_owner(&state.db, deck_id, user.id).await?;

    // Deliberately use the deck's canonical epoch. MAX(epoch) may point at an
    // isolated rehearsal and would make a real presentation summary lie.
    let result_epoch: i32 = sqlx::query_scalar("SELECT result_epoch FROM decks WHERE id = $1")
        .bind(deck_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;

    let definitions = sqlx::query(
        r#"
        SELECT interaction_key, slide_key, slide_number, kind, title, options, config
        FROM interaction_definitions
        WHERE deck_id = $1 AND is_archived = false
        ORDER BY slide_number, source_order, interaction_key
        "#,
    )
    .bind(deck_id)
    .fetch_all(&state.db)
    .await?;
    let response_rows = sqlx::query(
        r#"
        SELECT interaction_key, participant_id, payload
        FROM current_responses
        WHERE deck_id = $1 AND result_epoch = $2
        "#,
    )
    .bind(deck_id)
    .bind(result_epoch)
    .fetch_all(&state.db)
    .await?;
    let response_count = response_rows.len() as i64;
    let mut grouped = HashMap::<String, Vec<(Uuid, Value)>>::new();
    for row in response_rows {
        grouped
            .entry(row.try_get("interaction_key")?)
            .or_default()
            .push((row.try_get("participant_id")?, row.try_get("payload")?));
    }

    let interactions = definitions
        .into_iter()
        .map(|row| {
            let id: String = row.try_get("interaction_key")?;
            let kind: String = row.try_get("kind")?;
            let options: Vec<InteractionOption> =
                serde_json::from_value(row.try_get("options")?).unwrap_or_default();
            let config: Value = row.try_get("config")?;
            let values = grouped.get(&id).cloned().unwrap_or_default();
            // This owner-only aggregate may calculate sensitive details such as
            // quiz correctness, but only the curated headline/highlights below
            // leave the endpoint. Raw free text is never returned.
            let result = aggregate(&kind, &options, &config, &values, true);
            let (headline, highlights) = summarize_interaction(&kind, &options, &config, &result);
            Ok(LiveSummaryInteraction {
                id,
                slide_key: row.try_get("slide_key")?,
                slide_number: row.try_get("slide_number")?,
                kind,
                title: row.try_get("title")?,
                response_count: values.len() as i64,
                headline,
                highlights,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;

    let question_counts = sqlx::query(
        r#"
        SELECT COUNT(*)::bigint AS submitted,
               COUNT(*) FILTER (WHERE moderation_status = 'approved')::bigint AS approved,
               COUNT(*) FILTER (WHERE moderation_status = 'pending')::bigint AS pending,
               COUNT(*) FILTER (WHERE moderation_status = 'rejected')::bigint AS rejected,
               COUNT(*) FILTER (
                   WHERE moderation_status = 'approved' AND lifecycle_status = 'answered'
               )::bigint AS answered
        FROM questions
        WHERE deck_id = $1 AND result_epoch = $2
        "#,
    )
    .bind(deck_id)
    .bind(result_epoch)
    .fetch_one(&state.db)
    .await?;
    let total_question_votes: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)::bigint
        FROM question_votes votes
        JOIN questions question ON question.id = votes.question_id
        WHERE question.deck_id = $1 AND question.result_epoch = $2
          AND question.moderation_status = 'approved'
          AND question.lifecycle_status <> 'archived'
        "#,
    )
    .bind(deck_id)
    .bind(result_epoch)
    .fetch_one(&state.db)
    .await?;
    let top_questions = sqlx::query(
        r#"
        SELECT q.id, q.body, p.display_name, q.slide_number, q.is_pinned,
               q.lifecycle_status, COUNT(v.question_id)::bigint AS votes
        FROM questions q
        JOIN participant_sessions p ON p.id = q.participant_id
        LEFT JOIN question_votes v ON v.question_id = q.id
        WHERE q.deck_id = $1 AND q.result_epoch = $2
          AND q.moderation_status = 'approved'
          AND q.lifecycle_status <> 'archived'
        GROUP BY q.id, p.display_name
        ORDER BY q.is_pinned DESC, COUNT(v.question_id) DESC, q.created_at ASC
        LIMIT 50
        "#,
    )
    .bind(deck_id)
    .bind(result_epoch)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|row| {
        Ok(LiveSummaryQuestion {
            id: row.try_get("id")?,
            body: row.try_get("body")?,
            display_name: row.try_get("display_name")?,
            slide_number: row.try_get("slide_number")?,
            votes: row.try_get("votes")?,
            is_pinned: row.try_get("is_pinned")?,
            lifecycle_status: row.try_get("lifecycle_status")?,
        })
    })
    .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let participant_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)::bigint FROM (
            SELECT participant_id FROM current_responses
            WHERE deck_id = $1 AND result_epoch = $2
            UNION
            SELECT participant_id FROM questions
            WHERE deck_id = $1 AND result_epoch = $2
        ) participants
        "#,
    )
    .bind(deck_id)
    .bind(result_epoch)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(LiveSummary {
        deck_id,
        result_epoch,
        generated_at: Utc::now(),
        participant_count,
        response_count,
        interactions,
        questions: LiveSummaryQuestions {
            submitted: question_counts.try_get("submitted")?,
            approved: question_counts.try_get("approved")?,
            pending: question_counts.try_get("pending")?,
            rejected: question_counts.try_get("rejected")?,
            answered: question_counts.try_get("answered")?,
            total_votes: total_question_votes,
            top: top_questions,
        },
    }))
}

pub async fn join(
    State(state): State<Arc<AppState>>,
    Path(join_code): Path<String>,
    Json(request): Json<JoinRequest>,
) -> AppResult<Response> {
    let deck_id = deck_id_from_code(&state.db, &join_code).await?;
    state
        .check_rate_limit("audience-join", deck_id, 2_000, StdDuration::from_secs(60))
        .map_err(AppError::RateLimited)?;
    let display_name = request
        .display_name
        .map(|name| normalize_audience_text(&name))
        .filter(|name| !name.is_empty());
    if display_name
        .as_ref()
        .is_some_and(|name| name.chars().count() > 80 || name.len() > 320)
    {
        return Err(AppError::BadRequest("Display name is too long".to_owned()));
    }
    if let Some(display_name) = &display_name {
        reject_unsafe_audience_text(display_name, "Display name")?;
    }
    let token = random_token(48);
    let participant_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO participant_sessions (id, token_hash, deck_id, display_name) VALUES ($1, $2, $3, $4)",
    )
    .bind(participant_id)
    .bind(hash_token(&token))
    .bind(deck_id)
    .bind(display_name)
    .execute(&state.db)
    .await?;

    // Joining changes the projected and presenter audience count even though
    // it does not advance slide state. Schedule one coalesced shared snapshot
    // so existing SSE clients see the count without each reloading full state.
    state.notify(deck_id, 0);

    let live_state =
        build_live_state(&state.db, deck_id, Some(participant_id), false, false, None).await?;
    let mut response = Json(live_state).into_response();
    let cookie = audience_cookie(&token, state.config.secure_cookies());
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|error| AppError::Internal(error.into()))?,
    );
    Ok(response)
}

pub async fn audience_state(
    State(state): State<Arc<AppState>>,
    Path(join_code): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<LiveState>> {
    let deck_id = deck_id_from_code(&state.db, &join_code).await?;
    let participant = participant_from_headers(&state.db, deck_id, &headers).await?;
    Ok(Json(
        build_live_state(&state.db, deck_id, participant, false, false, None).await?,
    ))
}

pub async fn audience_events(
    State(state): State<Arc<AppState>>,
    Path(join_code): Path<String>,
) -> AppResult<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>> {
    let deck_id = deck_id_from_code(&state.db, &join_code).await?;
    // A fresh process builds this once for the deck. Concurrent reconnects wait
    // on the same build rather than stampeding PostgreSQL.
    let initial_snapshot = state.ensure_audience_snapshot(deck_id).await;
    let mut receiver = state.subscribe(deck_id);
    let mut shutdown = state.shutdown_receiver();
    let connection = state.track_sse();
    let stream = async_stream::stream! {
        let _connection = connection;
        // Flush immediately so EventSource reports an open connection without
        // waiting for the first state change or 15-second keepalive.
        yield Ok::<Event, Infallible>(Event::default().event("connected").data("ready"));
        if let Some((sequence, snapshot)) = initial_snapshot {
            yield Ok(Event::default()
                .id(sequence.to_string())
                .event("state")
                .data(&snapshot));
        }
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        yield Ok(Event::default()
                            .event("reconnect")
                            .retry(StdDuration::from_secs(1))
                            .data("deploy"));
                        break;
                    }
                }
                item = receiver.recv() => match item {
                    Ok(event) => yield Ok(Event::default()
                        .id(event.sequence.to_string())
                        .event("state")
                        .data(event.audience_snapshot
                            .map_or_else(|| event.sequence.to_string(), |snapshot| snapshot.to_string()))),
                    Err(RecvError::Lagged(_)) => yield Ok(Event::default()
                        .event("state")
                        .data("resync")),
                    Err(RecvError::Closed) => break,
                }
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(StdDuration::from_secs(15))
            .text("keep-alive"),
    ))
}

pub async fn submit_response(
    State(state): State<Arc<AppState>>,
    Path((join_code, interaction_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(mut request): Json<ResponseRequest>,
) -> AppResult<Json<Value>> {
    if request.idempotency_key.len() < 8 || request.idempotency_key.len() > 128 {
        return Err(AppError::BadRequest("Invalid idempotency key".to_owned()));
    }
    if request.payload.to_string().len() > MAX_INTERACTION_PAYLOAD_BYTES {
        return Err(AppError::BadRequest(
            "Interaction response is too large".to_owned(),
        ));
    }
    let deck_id = deck_id_from_code(&state.db, &join_code).await?;
    let participant_id = participant_from_headers(&state.db, deck_id, &headers)
        .await?
        .ok_or(AppError::Unauthorized)?;
    state
        .check_rate_limit(
            "interaction-response",
            participant_id,
            30,
            StdDuration::from_secs(60),
        )
        .map_err(AppError::RateLimited)?;
    let context = sqlx::query(
        r#"
        SELECT r.result_epoch, r.input_frozen, i.kind, i.options, i.config, s.phase,
               COALESCE(s.accepting_responses, true) AS accepting_responses, l.click_step,
               l.visible_element_interaction_ids,
               l.sequence AS durable_sequence
        FROM decks d
        JOIN live_deck_states l ON l.deck_id = d.id
        JOIN presentation_runs r ON r.id = l.run_id AND r.status = 'live'
        JOIN interaction_definitions i ON i.deck_id = d.id AND i.interaction_key = $2
        LEFT JOIN interaction_live_states s
          ON s.run_id = r.id AND s.interaction_key = i.interaction_key
        WHERE d.id = $1 AND i.slide_key = l.slide_key AND i.is_archived = false
        "#,
    )
    .bind(deck_id)
    .bind(&interaction_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Conflict("This interaction is not currently available".to_owned()))?;
    let epoch: i32 = context.try_get("result_epoch")?;
    let kind: String = context.try_get("kind")?;
    let options: Value = context.try_get("options")?;
    let config: Value = context.try_get("config")?;
    let phase: Option<String> = context.try_get("phase")?;
    let click_step: i32 = context.try_get("click_step")?;
    let visible_element_interaction_ids: Value =
        context.try_get("visible_element_interaction_ids")?;
    let durable_sequence: i64 = context.try_get("durable_sequence")?;
    let input_frozen: bool = context.try_get("input_frozen")?;
    if input_frozen {
        return Err(AppError::Conflict(
            "The presenter has temporarily frozen audience input".to_owned(),
        ));
    }
    if !context.try_get::<bool, _>("accepting_responses")? {
        return Err(AppError::Conflict(
            "The presenter has closed this interaction".to_owned(),
        ));
    }
    let element = config
        .get("element")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let element_is_visible = visible_element_interaction_ids
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|id| id == interaction_id);
    if element && !element_is_visible {
        return Err(AppError::Conflict(
            "This element interaction has not been revealed yet".to_owned(),
        ));
    }
    if kind == "ranked-list" && phase.as_deref() != Some("voting") {
        return Err(AppError::Conflict("Voting is not open".to_owned()));
    }
    normalize_response_payload(&kind, &mut request.payload)?;
    validate_response_payload(&kind, &options, &config, &request.payload)?;
    if kind == "ranked-list" {
        let revealed_count = if config.get("reveal").and_then(Value::as_str) == Some("click") {
            click_step
        } else {
            options.as_array().map_or(0, Vec::len) as i32
        };
        validate_ranked_revealed_votes(&options, &request.payload, revealed_count)?;
    }

    let mut tx = state.db.begin().await?;
    let inserted = sqlx::query(
        r#"
        INSERT INTO response_events
            (id, deck_id, interaction_key, participant_id, result_epoch, idempotency_key, payload)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (participant_id, idempotency_key) DO NOTHING
        "#,
    )
    .bind(Uuid::now_v7())
    .bind(deck_id)
    .bind(&interaction_id)
    .bind(participant_id)
    .bind(epoch)
    .bind(&request.idempotency_key)
    .bind(&request.payload)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;

    if inserted {
        sqlx::query(
            r#"
            INSERT INTO current_responses
                (deck_id, interaction_key, participant_id, result_epoch, payload)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (deck_id, interaction_key, participant_id, result_epoch)
            DO UPDATE SET payload = EXCLUDED.payload, updated_at = now()
            "#,
        )
        .bind(deck_id)
        .bind(&interaction_id)
        .bind(participant_id)
        .bind(epoch)
        .bind(&request.payload)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE participant_sessions SET last_seen_at = now() WHERE id = $1")
        .bind(participant_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    // Responses are already durable in response_events/current_responses. Using
    // the live_deck_states row as a notification counter serialized every vote
    // for a deck, so shared SSE snapshots use a cheap in-process monotonic ID.
    // Navigation remains durably ordered in live_deck_states.
    let sequence = if inserted {
        state.notify_response(deck_id, durable_sequence)
    } else {
        durable_sequence
    };
    let correct_option_id = (kind == "quiz")
        .then(|| config.get("correct").and_then(Value::as_str))
        .flatten();
    Ok(Json(json!({
        "saved": true,
        "duplicate": !inserted,
        "sequence": sequence,
        "correct_option_id": correct_option_id,
    })))
}

pub async fn submit_question(
    State(state): State<Arc<AppState>>,
    Path(join_code): Path<String>,
    headers: HeaderMap,
    Json(request): Json<QuestionRequest>,
) -> AppResult<(axum::http::StatusCode, Json<Value>)> {
    let body = normalize_audience_text(&request.body);
    if body.chars().count() < 3 || body.chars().count() > 500 {
        return Err(AppError::BadRequest(
            "Question must be between 3 and 500 characters".to_owned(),
        ));
    }
    reject_unsafe_audience_text(&body, "Question")?;
    let scope = request.scope.as_deref().unwrap_or("general");
    if !matches!(scope, "general" | "slide") {
        return Err(AppError::BadRequest(
            "Question scope must be general or slide".to_owned(),
        ));
    }
    let deck_id = deck_id_from_code(&state.db, &join_code).await?;
    let participant_id = participant_from_headers(&state.db, deck_id, &headers)
        .await?
        .ok_or(AppError::Unauthorized)?;
    state
        .check_rate_limit(
            "question-submit",
            participant_id,
            10,
            StdDuration::from_secs(300),
        )
        .map_err(AppError::RateLimited)?;
    state
        .check_rate_limit("question-deck", deck_id, 1_000, StdDuration::from_secs(60))
        .map_err(AppError::RateLimited)?;
    let run = sqlx::query(
        r#"
        SELECT r.result_epoch, r.input_frozen, l.slide_key, l.slide_number
        FROM presentation_runs r
        JOIN live_deck_states l ON l.run_id = r.id
        WHERE r.deck_id = $1 AND r.status = 'live'
        "#,
    )
    .bind(deck_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Conflict("The presentation is not live".to_owned()))?;
    if run.try_get::<bool, _>("input_frozen")? {
        return Err(AppError::Conflict(
            "The presenter has temporarily frozen audience input".to_owned(),
        ));
    }
    let epoch: i32 = run.try_get("result_epoch")?;
    let slide_key: Option<String> = (scope == "slide")
        .then(|| run.try_get("slide_key"))
        .transpose()?;
    let slide_number: Option<i32> = (scope == "slide")
        .then(|| run.try_get("slide_number"))
        .transpose()?;

    let question_id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    let ai_enabled = state.config.gemini_moderator_key.is_some();
    sqlx::query(
        r#"
        INSERT INTO questions
            (id, deck_id, participant_id, result_epoch, body, analysis_status, slide_key, slide_number)
        VALUES ($1, $2, $3, $4, $5, CASE WHEN $6 THEN 'queued' ELSE 'not_requested' END, $7, $8)
        "#,
    )
    .bind(question_id)
    .bind(deck_id)
    .bind(participant_id)
    .bind(epoch)
    .bind(&body)
    .bind(ai_enabled)
    .bind(&slide_key)
    .bind(slide_number)
    .execute(&mut *tx)
    .await?;
    if ai_enabled {
        sqlx::query(
            "INSERT INTO background_jobs (id, deck_id, kind, payload) VALUES ($1, $2, 'question_moderation', $3)",
        )
        .bind(Uuid::now_v7())
        .bind(deck_id)
        .bind(json!({ "question_id": question_id }))
        .execute(&mut *tx)
        .await?;
    }
    let sequence: Option<i64> = sqlx::query_scalar(
        "UPDATE live_deck_states SET sequence = sequence + 1, updated_at = now() WHERE deck_id = $1 RETURNING sequence",
    )
    .bind(deck_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    if let Some(sequence) = sequence {
        state.notify(deck_id, sequence);
    }
    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({
            "id": question_id,
            "moderation_status": "pending",
            "analysis_status": if ai_enabled { "queued" } else { "not_requested" },
            "scope": scope,
            "slide_number": slide_number,
        })),
    ))
}

pub async fn vote_question(
    State(state): State<Arc<AppState>>,
    Path((join_code, question_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let deck_id = deck_id_from_code(&state.db, &join_code).await?;
    let participant_id = participant_from_headers(&state.db, deck_id, &headers)
        .await?
        .ok_or(AppError::Unauthorized)?;
    state
        .check_rate_limit(
            "question-vote",
            participant_id,
            120,
            StdDuration::from_secs(60),
        )
        .map_err(AppError::RateLimited)?;
    let input_frozen: bool = sqlx::query_scalar(
        "SELECT input_frozen FROM presentation_runs WHERE deck_id = $1 AND status = 'live'",
    )
    .bind(deck_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Conflict("The presentation is not live".to_owned()))?;
    if input_frozen {
        return Err(AppError::Conflict(
            "The presenter has temporarily frozen audience input".to_owned(),
        ));
    }
    let mut tx = state.db.begin().await?;
    let deleted =
        sqlx::query("DELETE FROM question_votes WHERE question_id = $1 AND participant_id = $2")
            .bind(question_id)
            .bind(participant_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    let voted = if deleted == 0 {
        let inserted = sqlx::query(
            r#"
            INSERT INTO question_votes (question_id, participant_id)
            SELECT q.id, $2 FROM questions q
            WHERE q.id = $1 AND q.deck_id = $3 AND q.moderation_status = 'approved'
              AND q.lifecycle_status = 'open'
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(question_id)
        .bind(participant_id)
        .bind(deck_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        inserted > 0
    } else {
        false
    };
    let sequence: i64 = sqlx::query_scalar(
        "UPDATE live_deck_states SET sequence = sequence + 1, updated_at = now() WHERE deck_id = $1 RETURNING sequence",
    )
    .bind(deck_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    state.notify(deck_id, sequence);
    Ok(Json(json!({ "voted": voted })))
}

pub async fn moderate_question(
    State(state): State<Arc<AppState>>,
    Path((deck_id, question_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(request): Json<ModerateQuestionRequest>,
) -> AppResult<Json<Value>> {
    let user = require_user(&state, &headers).await?;
    assert_owner(&state.db, deck_id, user.id).await?;
    if request
        .moderation_status
        .as_deref()
        .is_some_and(|value| !matches!(value, "pending" | "approved" | "rejected"))
        || request
            .lifecycle_status
            .as_deref()
            .is_some_and(|value| !matches!(value, "open" | "answered" | "archived"))
    {
        return Err(AppError::BadRequest("Invalid question status".to_owned()));
    }
    let mut tx = state.db.begin().await?;
    let moderation_status = request.moderation_status.clone();
    let lifecycle_status = request.lifecycle_status.clone();
    let updated_id: Option<Uuid> = sqlx::query_scalar(
        r#"
        UPDATE questions SET
            moderation_status = COALESCE($1, moderation_status),
            lifecycle_status = COALESCE($2, lifecycle_status),
            is_pinned = CASE
                WHEN $2::text = 'archived' THEN false
                ELSE COALESCE($3, is_pinned)
            END,
            moderation_source = CASE WHEN $1::text IS NOT NULL THEN 'human' ELSE moderation_source END,
            updated_at = now()
        WHERE id = $4 AND deck_id = $5
        RETURNING id
        "#,
    )
    .bind(request.moderation_status)
    .bind(request.lifecycle_status)
    .bind(request.is_pinned)
    .bind(question_id)
    .bind(deck_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(_) = updated_id else {
        return Err(AppError::NotFound);
    };
    audit(
        &mut tx,
        Some(deck_id),
        Some(user.id),
        "question.updated",
        json!({
            "question_id": question_id,
            "moderation_status": moderation_status,
            "lifecycle_status": lifecycle_status,
            "is_pinned": request.is_pinned,
        }),
    )
    .await?;
    let sequence: Option<i64> = sqlx::query_scalar(
        "UPDATE live_deck_states SET sequence = sequence + 1, updated_at = now() WHERE deck_id = $1 RETURNING sequence",
    )
    .bind(deck_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    if let Some(sequence) = sequence {
        state.notify(deck_id, sequence);
    }
    Ok(Json(json!({ "ok": true })))
}

async fn fetch_deck_detail(db: &PgPool, deck_id: Uuid, owner_id: Uuid) -> AppResult<DeckDetail> {
    let row = sqlx::query(
        r#"
        SELECT d.id, d.title, d.join_code, d.markdown, d.deck_css, d.version, d.verified_version,
               d.last_verified_markdown, d.last_verified_css, d.result_epoch, d.qa_display_mode,
               EXISTS(SELECT 1 FROM presentation_runs r WHERE r.deck_id = d.id AND r.status = 'live') AS is_live
        FROM decks d
        WHERE d.id = $1 AND d.owner_id = $2
        "#,
    )
    .bind(deck_id)
    .bind(owner_id)
    .fetch_optional(db)
    .await?
    .ok_or(AppError::NotFound)?;
    let markdown: String = row.try_get("markdown")?;
    let version: i64 = row.try_get("version")?;
    let verified_version: Option<i64> = row.try_get("verified_version")?;
    let last_verified_markdown: Option<String> = row.try_get("last_verified_markdown")?;
    let css: String = row.try_get("deck_css")?;
    let last_verified_css: Option<String> = row.try_get("last_verified_css")?;
    let parsed = match parse_deck(&markdown) {
        Ok(parsed) => parsed,
        Err(draft_error) => {
            let fallback = last_verified_markdown.as_deref().ok_or_else(|| {
                AppError::BadRequest(format!("The saved deck is not valid: {draft_error}"))
            })?;
            let mut parsed = parse_deck(fallback).map_err(AppError::BadRequest)?;
            parsed.warnings.insert(
                0,
                format!("The saved draft is not ready to preview: {draft_error}"),
            );
            parsed
        }
    };
    let can_restore_last_verified = last_verified_markdown
        .as_deref()
        .is_some_and(|source| source != markdown)
        || last_verified_css
            .as_deref()
            .is_some_and(|source| source != css);
    Ok(DeckDetail {
        id: row.try_get("id")?,
        title: row.try_get("title")?,
        join_code: row.try_get("join_code")?,
        markdown,
        css,
        version,
        verified_version,
        is_verified: verified_version == Some(version),
        can_restore_last_verified,
        result_epoch: row.try_get("result_epoch")?,
        qa_display_mode: row.try_get("qa_display_mode")?,
        slides: parsed.slides,
        interactions: parsed.interactions,
        warnings: parsed.warnings,
        is_live: row.try_get("is_live")?,
    })
}

async fn build_live_state(
    db: &PgPool,
    deck_id: Uuid,
    participant_id: Option<Uuid>,
    presenter: bool,
    ai_configured: bool,
    ai_moderation_mode: Option<&str>,
) -> AppResult<LiveState> {
    let deck = sqlx::query(
        "SELECT title, join_code, result_epoch, qa_display_mode FROM decks WHERE id = $1",
    )
    .bind(deck_id)
    .fetch_optional(db)
    .await?
    .ok_or(AppError::NotFound)?;
    let title: String = deck.try_get("title")?;
    let join_code: String = deck.try_get("join_code")?;
    let default_epoch: i32 = deck.try_get("result_epoch")?;
    let qa_display_mode: String = deck.try_get("qa_display_mode")?;
    let live = sqlx::query(
        r#"
        SELECT l.run_id, l.slide_key, l.slide_number, l.click_step,
               l.visible_element_interaction_ids, l.sequence,
               r.run_mode, r.input_frozen, r.result_epoch
        FROM live_deck_states l
        JOIN presentation_runs r ON r.id = l.run_id AND r.status = 'live'
        WHERE l.deck_id = $1
        "#,
    )
    .bind(deck_id)
    .fetch_optional(db)
    .await?;

    let (
        run_id,
        run_mode,
        input_frozen,
        epoch,
        slide_key,
        slide_number,
        click_step,
        visible_element_interaction_ids,
        sequence,
    ) = if let Some(row) = live {
        (
            Some(row.try_get("run_id")?),
            Some(row.try_get("run_mode")?),
            row.try_get("input_frozen")?,
            row.try_get("result_epoch")?,
            Some(row.try_get::<String, _>("slide_key")?),
            Some(row.try_get("slide_number")?),
            row.try_get("click_step")?,
            row.try_get("visible_element_interaction_ids")?,
            row.try_get("sequence")?,
        )
    } else {
        (
            None,
            None,
            false,
            default_epoch,
            None,
            None,
            0,
            json!([]),
            0,
        )
    };
    let visible_element_interaction_ids = visible_element_interaction_ids
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<HashSet<_>>();

    if let Some(participant_id) = participant_id {
        sqlx::query("UPDATE participant_sessions SET last_seen_at = now() WHERE id = $1")
            .bind(participant_id)
            .execute(db)
            .await?;
    }

    let definitions = if let (Some(slide_key), Some(run_id)) = (&slide_key, run_id) {
        sqlx::query(
            r#"
            SELECT definitions.interaction_key, definitions.kind, definitions.title,
                   definitions.options, definitions.config,
                   COALESCE((definitions.config->>'element')::boolean, false) AS element,
                   CASE WHEN definitions.kind = 'ranked-list'
                        THEN COALESCE(live.phase, 'voting')
                   END AS phase,
                   COALESCE(live.accepting_responses, true) AS interaction_accepting,
                   COALESCE(live.results_revealed, false) AS results_revealed,
                   live.timer_ends_at IS NOT NULL AND live.timer_ends_at > now() AS timer_running,
                   CASE
                     WHEN live.timer_ends_at IS NOT NULL THEN
                       GREATEST(0, CEIL(EXTRACT(EPOCH FROM (live.timer_ends_at - now()))))::integer
                     ELSE live.timer_remaining_seconds
                   END AS timer_remaining_seconds
            FROM interaction_definitions definitions
            LEFT JOIN interaction_live_states live
              ON live.run_id = $3 AND live.interaction_key = definitions.interaction_key
            WHERE definitions.deck_id = $1 AND definitions.slide_key = $2
              AND definitions.is_archived = false
            ORDER BY definitions.source_order, definitions.interaction_key
            "#,
        )
        .bind(deck_id)
        .bind(slide_key)
        .bind(run_id)
        .fetch_all(db)
        .await?
    } else {
        Vec::new()
    };

    let definition_ids = definitions
        .iter()
        .map(|row| row.try_get::<String, _>("interaction_key"))
        .collect::<Result<Vec<_>, _>>()?;
    let responses = if definition_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query(
            r#"
            SELECT interaction_key, participant_id, payload
            FROM current_responses
            WHERE deck_id = $1 AND result_epoch = $2
              AND interaction_key = ANY($3)
            "#,
        )
        .bind(deck_id)
        .bind(epoch)
        .bind(&definition_ids)
        .fetch_all(db)
        .await?
    };

    let mut grouped: HashMap<String, Vec<(Uuid, Value)>> = HashMap::new();
    for row in responses {
        grouped
            .entry(row.try_get("interaction_key")?)
            .or_default()
            .push((row.try_get("participant_id")?, row.try_get("payload")?));
    }

    let interactions = definitions
        .into_iter()
        .map(|row| {
            let id: String = row.try_get("interaction_key")?;
            let kind: String = row.try_get("kind")?;
            let options_value: Value = row.try_get("options")?;
            let options =
                serde_json::from_value::<Vec<InteractionOption>>(options_value).unwrap_or_default();
            let config: Value = row.try_get("config")?;
            let element: bool = row.try_get("element")?;
            if !presenter && element && !visible_element_interaction_ids.contains(id.as_str()) {
                return Ok(None);
            }
            let phase: Option<String> = row.try_get("phase")?;
            let ranked_list = kind == "ranked-list";
            let revealed_count = ranked_list.then(|| {
                if config.get("reveal").and_then(Value::as_str) == Some("click") {
                    click_step.clamp(0, options.len() as i32)
                } else {
                    options.len() as i32
                }
            });
            let interaction_accepting: bool = row.try_get("interaction_accepting")?;
            let results_revealed: bool = row.try_get("results_revealed")?;
            let timer_running: bool = row.try_get("timer_running")?;
            let timer_remaining_seconds: Option<i32> = row.try_get("timer_remaining_seconds")?;
            let accepting_responses = !input_frozen
                && interaction_accepting
                && (!ranked_list || phase.as_deref() == Some("voting"));
            let values = grouped.get(&id).cloned().unwrap_or_default();
            let own = participant_id.and_then(|participant| {
                values
                    .iter()
                    .find(|(id, _)| *id == participant)
                    .map(|(_, value)| value.clone())
            });
            let results_visible = if ranked_list {
                presenter || phase.as_deref() == Some("ranked")
            } else {
                presenter || audience_can_see_results(&config, own.is_some(), results_revealed)
            };
            let result = if results_visible {
                aggregate(
                    &kind,
                    &options,
                    &config,
                    &values,
                    presenter || own.is_some() || results_revealed,
                )
            } else if ranked_list {
                json!({ "hidden": true, "count": values.len() })
            } else {
                json!({ "hidden": true })
            };
            let public_config = if presenter {
                config
            } else {
                let mut config = config;
                if let Some(config) = config.as_object_mut() {
                    config.remove("correct");
                    if kind == "image-hotspot"
                        && let Some(image) = config.get("image").and_then(Value::as_str)
                        && let Some(audience_url) = audience_asset_url(&join_code, image)
                    {
                        config.insert("image".to_owned(), Value::String(audience_url));
                    }
                }
                config
            };
            Ok(Some(AudienceInteraction {
                id,
                kind: kind.clone(),
                title: row.try_get("title")?,
                options: options.clone(),
                config: public_config,
                element,
                phase,
                revealed_count,
                accepting_responses,
                presenter_closed: !interaction_accepting,
                results_revealed,
                timer_running,
                timer_remaining_seconds,
                result,
                response: own,
            }))
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?
        .into_iter()
        .flatten()
        .collect();

    let question_rows = sqlx::query(
        r#"
        WITH question_clusters AS (
            SELECT COALESCE(duplicate_of, id) AS root_id, COUNT(*)::bigint AS repeat_count
            FROM questions
            WHERE deck_id = $1 AND result_epoch = $2
              AND moderation_status = 'approved'
              AND lifecycle_status <> 'archived'
            GROUP BY COALESCE(duplicate_of, id)
        )
        SELECT q.id, q.body, p.display_name, q.moderation_status, q.lifecycle_status,
               q.is_pinned, q.created_at, q.participant_id, q.analysis_status,
               q.moderation_source, q.moderation_labels, q.moderation_reason,
               q.topic, q.duplicate_of, q.slide_key, q.slide_number,
               COUNT(v.question_id)::bigint AS votes,
               COALESCE(cluster.repeat_count, 0)::bigint AS repeat_count
        FROM questions q
        JOIN participant_sessions p ON p.id = q.participant_id
        LEFT JOIN question_votes v ON v.question_id = q.id
        LEFT JOIN question_clusters cluster ON cluster.root_id = COALESCE(q.duplicate_of, q.id)
        WHERE q.deck_id = $1 AND q.result_epoch = $2
          AND ($3 OR q.moderation_status = 'approved' OR q.participant_id = $4)
          AND ($3 OR q.lifecycle_status <> 'archived')
          AND ($3 OR q.slide_key IS NULL OR q.slide_key = $5)
        GROUP BY q.id, p.display_name, cluster.repeat_count
        ORDER BY q.is_pinned DESC, COUNT(v.question_id) DESC, q.created_at DESC
        "#,
    )
    .bind(deck_id)
    .bind(epoch)
    .bind(presenter)
    .bind(participant_id)
    .bind(&slide_key)
    .fetch_all(db)
    .await?;
    let questions = question_rows
        .into_iter()
        .map(|row| {
            let author_id: Uuid = row.try_get("participant_id")?;
            let display_name: Option<String> = row.try_get("display_name")?;
            let votes: i64 = row.try_get("votes")?;
            let repeat_count: i64 = row.try_get("repeat_count")?;
            let is_novel = repeat_count <= 1;
            let named = display_name
                .as_deref()
                .is_some_and(|name| !name.trim().is_empty());
            Ok(QuestionView {
                id: row.try_get("id")?,
                body: row.try_get("body")?,
                display_name,
                votes,
                moderation_status: row.try_get("moderation_status")?,
                lifecycle_status: row.try_get("lifecycle_status")?,
                is_pinned: row.try_get("is_pinned")?,
                slide_key: row.try_get("slide_key")?,
                slide_number: row.try_get("slide_number")?,
                analysis_status: presenter
                    .then(|| row.try_get("analysis_status"))
                    .transpose()?,
                moderation_source: presenter
                    .then(|| row.try_get("moderation_source"))
                    .transpose()?,
                moderation_labels: presenter
                    .then(|| row.try_get("moderation_labels"))
                    .transpose()?,
                moderation_reason: if presenter {
                    row.try_get("moderation_reason")?
                } else {
                    None
                },
                topic: if presenter {
                    row.try_get("topic")?
                } else {
                    None
                },
                duplicate_of: if presenter {
                    row.try_get("duplicate_of")?
                } else {
                    None
                },
                is_novel: presenter.then_some(is_novel),
                repeat_count: presenter.then_some(repeat_count),
                priority_score: presenter
                    .then_some(votes * 10 + i64::from(is_novel) * 4 + i64::from(named)),
                mine: participant_id == Some(author_id),
                created_at: row.try_get("created_at")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let ai_insights = if presenter {
        sqlx::query(
            r#"
            SELECT summary, themes, suggested_answers, question_count, created_at
            FROM deck_insight_versions
            WHERE deck_id = $1 AND result_epoch = $2
            ORDER BY created_at DESC LIMIT 1
            "#,
        )
        .bind(deck_id)
        .bind(epoch)
        .fetch_optional(db)
        .await?
        .map(|row| {
            Ok::<_, sqlx::Error>(DeckInsightView {
                summary: row.try_get("summary")?,
                themes: row.try_get("themes")?,
                suggested_answers: row.try_get("suggested_answers")?,
                question_count: row.try_get("question_count")?,
                generated_at: row.try_get("created_at")?,
            })
        })
        .transpose()?
    } else {
        None
    };
    let ai_insights_status = if presenter {
        sqlx::query_scalar(
            r#"
            SELECT status FROM background_jobs
            WHERE deck_id = $1 AND kind = 'deck_insights'
              AND payload ->> 'result_epoch' = $2::text
            ORDER BY created_at DESC LIMIT 1
            "#,
        )
        .bind(deck_id)
        .bind(epoch)
        .fetch_optional(db)
        .await?
    } else {
        None
    };
    let participant_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM participant_sessions WHERE deck_id = $1 AND last_seen_at > now() - interval '5 minutes'",
    )
    .bind(deck_id)
    .fetch_one(db)
    .await?;

    Ok(LiveState {
        live: run_id.is_some(),
        run_id,
        run_mode,
        input_frozen,
        result_epoch: epoch,
        deck_id,
        deck_title: title,
        join_code,
        slide_key,
        slide_number,
        click_step,
        sequence,
        interactions,
        questions,
        ai_configured,
        ai_moderation_mode: presenter.then(|| ai_moderation_mode.unwrap_or("assist").to_owned()),
        qa_display_mode,
        ai_insights,
        ai_insights_status,
        participant_count,
        audience_session_active: participant_id.is_some(),
    })
}

pub(crate) async fn build_shared_audience_state(
    db: &PgPool,
    deck_id: Uuid,
) -> AppResult<LiveState> {
    build_live_state(db, deck_id, None, false, false, None).await
}

fn audience_can_see_results(config: &Value, has_responded: bool, results_revealed: bool) -> bool {
    match config.get("results").and_then(Value::as_str) {
        Some("always" | "live") => true,
        Some("hidden" | "presenter") => false,
        Some("manual" | "on-close") => results_revealed,
        Some("after-vote") | None => has_responded,
        Some(_) => has_responded,
    }
}

fn aggregate(
    kind: &str,
    options: &[InteractionOption],
    config: &Value,
    values: &[(Uuid, Value)],
    include_sensitive: bool,
) -> Value {
    match kind {
        "poll" | "quiz" | "reaction" | "image-choice" => {
            let mut counts = options
                .iter()
                .map(|option| (option.id.clone(), 0_i64))
                .collect::<HashMap<_, _>>();
            for (_, value) in values {
                if let Some(option) = value.get("option_id").and_then(Value::as_str) {
                    *counts.entry(option.to_owned()).or_default() += 1;
                }
                if let Some(selected) = value.get("option_ids").and_then(Value::as_array) {
                    let mut unique = HashSet::new();
                    for option in selected.iter().filter_map(Value::as_str) {
                        if !unique.insert(option) {
                            continue;
                        }
                        *counts.entry(option.to_owned()).or_default() += 1;
                    }
                }
            }
            let correct_option_id = (kind == "quiz")
                .then(|| config.get("correct").and_then(Value::as_str))
                .flatten();
            let correct_count = correct_option_id.map(|correct| {
                values
                    .iter()
                    .filter(|(_, value)| {
                        value.get("option_id").and_then(Value::as_str) == Some(correct)
                    })
                    .count()
            });
            json!({
                "count": values.len(),
                "counts": counts,
                "correct_count": include_sensitive.then_some(correct_count).flatten(),
                "correct_option_id": include_sensitive.then_some(correct_option_id).flatten(),
            })
        }
        "vote" | "updown" => {
            let mut up = 0_i64;
            let mut down = 0_i64;
            for value in values
                .iter()
                .filter_map(|(_, value)| value.get("value").and_then(Value::as_i64))
            {
                if value > 0 {
                    up += 1
                } else if value < 0 {
                    down += 1
                }
            }
            json!({ "count": values.len(), "up": up, "down": down, "score": up - down })
        }
        "ranked-list" => {
            let mut tallies = options
                .iter()
                .map(|option| (option.id.clone(), (0_i64, 0_i64)))
                .collect::<HashMap<_, _>>();
            for (_, response) in values {
                let Some(votes) = response.get("votes").and_then(Value::as_object) else {
                    continue;
                };
                for (option_id, vote) in votes {
                    let Some((up, down)) = tallies.get_mut(option_id) else {
                        continue;
                    };
                    match vote.as_i64() {
                        Some(1) => *up += 1,
                        Some(-1) => *down += 1,
                        _ => {}
                    }
                }
            }
            let scores = options
                .iter()
                .map(|option| {
                    let (up, down) = tallies.get(&option.id).copied().unwrap_or_default();
                    (
                        option.id.clone(),
                        json!({ "up": up, "down": down, "score": up - down }),
                    )
                })
                .collect::<serde_json::Map<_, _>>();
            let mut ranking = options.iter().enumerate().collect::<Vec<_>>();
            ranking.sort_by(|(left_index, left), (right_index, right)| {
                let left_score = scores
                    .get(&left.id)
                    .and_then(|score| score.get("score"))
                    .and_then(Value::as_i64)
                    .unwrap_or_default();
                let right_score = scores
                    .get(&right.id)
                    .and_then(|score| score.get("score"))
                    .and_then(Value::as_i64)
                    .unwrap_or_default();
                right_score
                    .cmp(&left_score)
                    .then_with(|| left_index.cmp(right_index))
            });
            let ranking = ranking
                .into_iter()
                .map(|(_, option)| option.id.clone())
                .collect::<Vec<_>>();
            json!({ "count": values.len(), "scores": scores, "ranking": ranking })
        }
        "ranking" => {
            let mut totals = options
                .iter()
                .map(|option| (option.id.clone(), (0_i64, 0_i64)))
                .collect::<HashMap<_, _>>();
            for (_, response) in values {
                let Some(ranking) = response.get("ranking").and_then(Value::as_array) else {
                    continue;
                };
                for (index, option_id) in ranking.iter().filter_map(Value::as_str).enumerate() {
                    if let Some((rank_total, points_total)) = totals.get_mut(option_id) {
                        *rank_total += index as i64 + 1;
                        *points_total += options.len() as i64 - index as i64;
                    }
                }
            }
            let divisor = values.len().max(1) as f64;
            let scores = options
                .iter()
                .map(|option| {
                    let (rank_total, points) = totals.get(&option.id).copied().unwrap_or_default();
                    (
                        option.id.clone(),
                        json!({ "average_rank": rank_total as f64 / divisor, "points": points }),
                    )
                })
                .collect::<serde_json::Map<_, _>>();
            let mut ranking = options.iter().enumerate().collect::<Vec<_>>();
            ranking.sort_by(|(left_index, left), (right_index, right)| {
                let left_rank = scores[&left.id]["average_rank"]
                    .as_f64()
                    .unwrap_or(f64::MAX);
                let right_rank = scores[&right.id]["average_rank"]
                    .as_f64()
                    .unwrap_or(f64::MAX);
                left_rank
                    .total_cmp(&right_rank)
                    .then_with(|| left_index.cmp(right_index))
            });
            json!({
                "count": values.len(),
                "scores": scores,
                "ranking": ranking.into_iter().map(|(_, option)| option.id.clone()).collect::<Vec<_>>()
            })
        }
        "rating" => {
            let ratings = values
                .iter()
                .filter_map(|(_, value)| value.get("value").and_then(Value::as_f64))
                .collect::<Vec<_>>();
            let average =
                (!ratings.is_empty()).then(|| ratings.iter().sum::<f64>() / ratings.len() as f64);
            json!({ "count": ratings.len(), "average": average })
        }
        "number" => {
            let mut numbers = values
                .iter()
                .filter_map(|(_, value)| value.get("value").and_then(Value::as_f64))
                .filter(|value| value.is_finite())
                .collect::<Vec<_>>();
            numbers.sort_by(f64::total_cmp);
            let average =
                (!numbers.is_empty()).then(|| numbers.iter().sum::<f64>() / numbers.len() as f64);
            let median = match numbers.len() {
                0 => None,
                length if length % 2 == 1 => Some(numbers[length / 2]),
                length => Some((numbers[length / 2 - 1] + numbers[length / 2]) / 2.0),
            };
            json!({
                "count": numbers.len(),
                "average": average,
                "median": median,
                "min": numbers.first(),
                "max": numbers.last(),
            })
        }
        "allocation" => {
            let mut totals = options
                .iter()
                .map(|option| (option.id.clone(), 0.0_f64))
                .collect::<HashMap<_, _>>();
            for (_, response) in values {
                let Some(allocations) = response.get("allocations").and_then(Value::as_object)
                else {
                    continue;
                };
                for (option_id, value) in allocations {
                    if let Some(total) = totals.get_mut(option_id) {
                        *total += value.as_f64().unwrap_or_default();
                    }
                }
            }
            let divisor = values.len().max(1) as f64;
            let averages = totals
                .iter()
                .map(|(id, total)| (id.clone(), json!(total / divisor)))
                .collect::<serde_json::Map<_, _>>();
            json!({ "count": values.len(), "totals": totals, "averages": averages })
        }
        "matrix" => {
            let points = values
                .iter()
                .rev()
                .filter_map(|(_, value)| {
                    Some(json!({
                        "x": value.get("x")?.as_f64()?,
                        "y": value.get("y")?.as_f64()?,
                    }))
                })
                .take(300)
                .collect::<Vec<_>>();
            let average_x = (!points.is_empty()).then(|| {
                points
                    .iter()
                    .filter_map(|point| point["x"].as_f64())
                    .sum::<f64>()
                    / points.len() as f64
            });
            let average_y = (!points.is_empty()).then(|| {
                points
                    .iter()
                    .filter_map(|point| point["y"].as_f64())
                    .sum::<f64>()
                    / points.len() as f64
            });
            json!({
                "count": values.len(),
                "average_x": average_x,
                "average_y": average_y,
                "points": points,
            })
        }
        "image-hotspot" => {
            let points = values
                .iter()
                .rev()
                .filter_map(|(_, value)| {
                    Some(json!({
                        "x": value.get("x")?.as_f64()?,
                        "y": value.get("y")?.as_f64()?,
                    }))
                })
                .take(300)
                .collect::<Vec<_>>();
            let average_x = (!points.is_empty()).then(|| {
                points
                    .iter()
                    .filter_map(|point| point["x"].as_f64())
                    .sum::<f64>()
                    / points.len() as f64
            });
            let average_y = (!points.is_empty()).then(|| {
                points
                    .iter()
                    .filter_map(|point| point["y"].as_f64())
                    .sum::<f64>()
                    / points.len() as f64
            });
            json!({
                "count": values.len(),
                "average_x": average_x,
                "average_y": average_y,
                "points": points,
            })
        }
        "survey" => {
            let questions = config
                .get("questions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut results = serde_json::Map::new();
            for question in questions {
                let Some(id) = question.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let kind = question.get("type").and_then(Value::as_str).unwrap_or("");
                let answers = values
                    .iter()
                    .filter_map(|(_, response)| response.get("answers")?.get(id))
                    .collect::<Vec<_>>();
                let result = match kind {
                    "rating" => {
                        let ratings = answers
                            .iter()
                            .filter_map(|value| value.as_f64())
                            .collect::<Vec<_>>();
                        let average = (!ratings.is_empty())
                            .then(|| ratings.iter().sum::<f64>() / ratings.len() as f64);
                        json!({ "count": ratings.len(), "average": average })
                    }
                    "choice" => {
                        let mut counts = question
                            .get("options")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|option| option.get("id").and_then(Value::as_str))
                            .map(|id| (id.to_owned(), 0_i64))
                            .collect::<HashMap<_, _>>();
                        for answer in &answers {
                            if let Some(id) = answer.as_str()
                                && let Some(count) = counts.get_mut(id)
                            {
                                *count += 1;
                            }
                        }
                        json!({ "count": answers.len(), "counts": counts })
                    }
                    "text" => {
                        let texts = include_sensitive.then(|| {
                            answers
                                .iter()
                                .rev()
                                .take(100)
                                .filter_map(|value| value.as_str())
                                .collect::<Vec<_>>()
                        });
                        json!({ "count": answers.len(), "texts": texts })
                    }
                    _ => json!({ "count": 0 }),
                };
                results.insert(id.to_owned(), result);
            }
            json!({ "count": values.len(), "questions": results })
        }
        "word-cloud" => {
            let multiple_entries = config
                .get("entries")
                .and_then(Value::as_str)
                .unwrap_or("multiple")
                != "one";
            let mut words = HashMap::<String, i64>::new();
            let mut participant_count = 0_usize;
            let mut submission_count = 0_usize;
            for (_, value) in values {
                let mut participant_words = HashSet::<String>::new();
                if let Some(texts) = value.get("texts").and_then(Value::as_array) {
                    let texts = texts.iter().filter_map(Value::as_str);
                    for text in if multiple_entries {
                        texts.collect::<Vec<_>>()
                    } else {
                        texts.rev().take(1).collect::<Vec<_>>()
                    } {
                        let normalized = text.trim().to_lowercase();
                        if !normalized.is_empty() {
                            participant_words.insert(normalized);
                        }
                    }
                } else if let Some(text) = value.get("text").and_then(Value::as_str) {
                    let normalized = text.trim().to_lowercase();
                    if !normalized.is_empty() {
                        participant_words.insert(normalized);
                    }
                }
                if !participant_words.is_empty() {
                    participant_count += 1;
                    submission_count += participant_words.len();
                    for word in participant_words {
                        *words.entry(word).or_default() += 1;
                    }
                }
            }
            let mut words = words
                .into_iter()
                .map(|(text, count)| json!({ "text": text, "count": count }))
                .collect::<Vec<_>>();
            words.sort_by(|left, right| {
                right["count"]
                    .as_i64()
                    .unwrap_or_default()
                    .cmp(&left["count"].as_i64().unwrap_or_default())
                    .then_with(|| {
                        left["text"]
                            .as_str()
                            .unwrap_or_default()
                            .cmp(right["text"].as_str().unwrap_or_default())
                    })
            });
            words.truncate(50);
            json!({
                "count": participant_count,
                "submission_count": submission_count,
                "words": words
            })
        }
        "free-text" => {
            let texts = include_sensitive.then(|| {
                values
                    .iter()
                    .rev()
                    .take(100)
                    .filter_map(|(_, value)| value.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
            });
            json!({ "count": values.len(), "texts": texts })
        }
        _ => json!({ "count": values.len() }),
    }
}

fn summarize_interaction(
    kind: &str,
    options: &[InteractionOption],
    config: &Value,
    result: &Value,
) -> (String, Vec<String>) {
    let count = result
        .get("count")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    if count == 0 {
        return ("No responses yet".to_owned(), Vec::new());
    }
    let option_label = |id: &str| {
        options
            .iter()
            .find(|option| option.id == id)
            .map(|option| option.label.clone())
            .unwrap_or_else(|| id.to_owned())
    };
    let format_number = |value: f64| {
        if (value - value.round()).abs() < 0.005 {
            format!("{value:.0}")
        } else {
            format!("{value:.1}")
        }
    };

    match kind {
        "poll" | "quiz" | "reaction" | "image-choice" => {
            let counts = result
                .get("counts")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let mut ranked = options
                .iter()
                .enumerate()
                .map(|(index, option)| {
                    (
                        index,
                        option.label.clone(),
                        counts
                            .get(&option.id)
                            .and_then(Value::as_i64)
                            .unwrap_or_default(),
                    )
                })
                .collect::<Vec<_>>();
            ranked.sort_by(|left, right| right.2.cmp(&left.2).then(left.0.cmp(&right.0)));
            let highlights = ranked
                .iter()
                .take(4)
                .map(|(_, label, votes)| format!("{label}: {votes}"))
                .collect::<Vec<_>>();
            if kind == "quiz" {
                let correct = result
                    .get("correct_count")
                    .and_then(Value::as_i64)
                    .unwrap_or_default();
                let percentage = (correct as f64 / count as f64 * 100.0).round() as i64;
                return (
                    format!("{correct} of {count} answered correctly ({percentage}%)"),
                    highlights,
                );
            }
            let (label, votes) = ranked
                .first()
                .map(|(_, label, votes)| (label.as_str(), *votes))
                .unwrap_or(("No option", 0));
            (
                format!(
                    "{label} led with {votes} selection{}",
                    if votes == 1 { "" } else { "s" }
                ),
                highlights,
            )
        }
        "vote" | "updown" => {
            let up = result.get("up").and_then(Value::as_i64).unwrap_or_default();
            let down = result
                .get("down")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            let score = up - down;
            (
                format!("{up} up · {down} down · net {score:+}"),
                vec![format!(
                    "{count} voter{}",
                    if count == 1 { "" } else { "s" }
                )],
            )
        }
        "ranked-list" => {
            let scores = result.get("scores").and_then(Value::as_object);
            let ranked = result
                .get("ranking")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            let highlights = ranked
                .iter()
                .take(4)
                .map(|id| {
                    let score = scores
                        .and_then(|scores| scores.get(*id))
                        .and_then(|value| value.get("score"))
                        .and_then(Value::as_i64)
                        .unwrap_or_default();
                    format!("{}: {score:+}", option_label(id))
                })
                .collect::<Vec<_>>();
            let leader = ranked
                .first()
                .map(|id| option_label(id))
                .unwrap_or_default();
            (
                format!("{leader} ranked highest across {count} voters"),
                highlights,
            )
        }
        "ranking" => {
            let scores = result.get("scores").and_then(Value::as_object);
            let ranked = result
                .get("ranking")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            let highlights = ranked
                .iter()
                .take(4)
                .map(|id| {
                    let average = scores
                        .and_then(|scores| scores.get(*id))
                        .and_then(|value| value.get("average_rank"))
                        .and_then(Value::as_f64)
                        .unwrap_or_default();
                    format!("{}: avg rank {}", option_label(id), format_number(average))
                })
                .collect::<Vec<_>>();
            let leader = ranked
                .first()
                .map(|id| option_label(id))
                .unwrap_or_default();
            (format!("{leader} was ranked first overall"), highlights)
        }
        "rating" => {
            let average = result
                .get("average")
                .and_then(Value::as_f64)
                .unwrap_or_default();
            let maximum = config.get("max").and_then(Value::as_f64).unwrap_or(5.0);
            (
                format!(
                    "Average rating {} out of {}",
                    format_number(average),
                    format_number(maximum)
                ),
                vec![format!(
                    "{count} rating{}",
                    if count == 1 { "" } else { "s" }
                )],
            )
        }
        "number" => {
            let average = result
                .get("average")
                .and_then(Value::as_f64)
                .unwrap_or_default();
            let median = result
                .get("median")
                .and_then(Value::as_f64)
                .unwrap_or_default();
            let unit = config.get("unit").and_then(Value::as_str).unwrap_or("");
            (
                format!("Average {}{unit}", format_number(average)),
                vec![
                    format!("Median {}{unit}", format_number(median)),
                    format!("{count} responses"),
                ],
            )
        }
        "allocation" => {
            let averages = result
                .get("averages")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let mut ranked = options
                .iter()
                .enumerate()
                .map(|(index, option)| {
                    (
                        index,
                        option.label.clone(),
                        averages
                            .get(&option.id)
                            .and_then(Value::as_f64)
                            .unwrap_or_default(),
                    )
                })
                .collect::<Vec<_>>();
            ranked.sort_by(|left, right| right.2.total_cmp(&left.2).then(left.0.cmp(&right.0)));
            let highlights = ranked
                .iter()
                .take(4)
                .map(|(_, label, average)| format!("{label}: {} average", format_number(*average)))
                .collect::<Vec<_>>();
            let leader = ranked
                .first()
                .map(|(_, label, _)| label.as_str())
                .unwrap_or("");
            (
                format!("{leader} received the largest average allocation"),
                highlights,
            )
        }
        "matrix" => {
            let x = result
                .get("average_x")
                .and_then(Value::as_f64)
                .unwrap_or_default();
            let y = result
                .get("average_y")
                .and_then(Value::as_f64)
                .unwrap_or_default();
            let x_label = config
                .get("x_label")
                .or_else(|| config.get("x-label"))
                .and_then(Value::as_str)
                .unwrap_or("X");
            let y_label = config
                .get("y_label")
                .or_else(|| config.get("y-label"))
                .and_then(Value::as_str)
                .unwrap_or("Y");
            (
                format!(
                    "The average point was {}, {}",
                    format_number(x),
                    format_number(y)
                ),
                vec![
                    format!("{x_label}: {}", format_number(x)),
                    format!("{y_label}: {}", format_number(y)),
                ],
            )
        }
        "image-hotspot" => (
            format!(
                "{count} pin{} placed",
                if count == 1 { " was" } else { "s were" }
            ),
            Vec::new(),
        ),
        "survey" => (
            format!(
                "{count} completed survey{}",
                if count == 1 { "" } else { "s" }
            ),
            Vec::new(),
        ),
        "word-cloud" => {
            let submission_count = result
                .get("submission_count")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            let words = result
                .get("words")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(6)
                .filter_map(|word| {
                    Some(format!(
                        "{}: {}",
                        word.get("text")?.as_str()?,
                        word.get("count")?.as_i64()?
                    ))
                })
                .collect::<Vec<_>>();
            let leader = result
                .get("words")
                .and_then(Value::as_array)
                .and_then(|words| words.first())
                .and_then(|word| word.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("No word");
            (
                format!("“{leader}” was the most common of {submission_count} words"),
                words,
            )
        }
        "free-text" => (
            format!(
                "{count} written response{} collected",
                if count == 1 { "" } else { "s" }
            ),
            vec!["Free text is not projected without moderation".to_owned()],
        ),
        _ => (
            format!("{count} response{}", if count == 1 { "" } else { "s" }),
            Vec::new(),
        ),
    }
}

fn validate_response_payload(
    kind: &str,
    options: &Value,
    config: &Value,
    payload: &Value,
) -> AppResult<()> {
    validate_response_shape(kind, payload)?;
    match kind {
        "poll" | "quiz" | "reaction" | "image-choice" => {
            let valid = options
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|option| {
                    option
                        .get("id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .collect::<Vec<_>>();
            let has_single = payload.get("option_id").is_some();
            let has_multiple = payload.get("option_ids").is_some();
            if has_single == has_multiple {
                return Err(AppError::BadRequest(
                    "Provide exactly one choice response shape".to_owned(),
                ));
            }
            let selected = payload
                .get("option_id")
                .and_then(Value::as_str)
                .map(|value| vec![value])
                .or_else(|| {
                    payload
                        .get("option_ids")
                        .and_then(Value::as_array)
                        .map(|values| values.iter().filter_map(Value::as_str).collect())
                })
                .unwrap_or_default();
            let unique = selected.iter().copied().collect::<HashSet<_>>();
            if selected.is_empty()
                || unique.len() != selected.len()
                || selected
                    .iter()
                    .any(|value| !valid.iter().any(|valid| valid == value))
            {
                return Err(AppError::BadRequest("Invalid poll response".to_owned()));
            }
            let multiple = kind == "poll"
                && (config.get("multiple").and_then(Value::as_str) == Some("true")
                    || config.get("max").is_some());
            if kind != "poll" || !multiple {
                if selected.len() != 1 || has_multiple {
                    return Err(AppError::BadRequest(
                        "This interaction accepts one option".to_owned(),
                    ));
                }
            } else {
                if has_single {
                    return Err(AppError::BadRequest(
                        "This poll accepts an option list".to_owned(),
                    ));
                }
                let max = config
                    .get("max")
                    .and_then(Value::as_str)
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(valid.len());
                if selected.len() > max {
                    return Err(AppError::BadRequest(format!(
                        "Choose no more than {max} options"
                    )));
                }
            }
        }
        "vote" | "updown" => {
            if !matches!(payload.get("value").and_then(Value::as_i64), Some(-1 | 1)) {
                return Err(AppError::BadRequest("Vote must be -1 or 1".to_owned()));
            }
        }
        "ranked-list" => {
            let valid = options
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|option| {
                    option
                        .get("id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .collect::<Vec<_>>();
            let votes = payload
                .get("votes")
                .and_then(Value::as_object)
                .ok_or_else(|| AppError::BadRequest("Ranked-list votes are required".to_owned()))?;
            if votes.is_empty()
                || votes.keys().any(|id| !valid.contains(id))
                || votes
                    .values()
                    .any(|vote| !matches!(vote.as_i64(), Some(-1 | 1)))
            {
                return Err(AppError::BadRequest("Invalid ranked-list vote".to_owned()));
            }
        }
        "ranking" => {
            let valid = options
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|option| {
                    option
                        .get("id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .collect::<HashSet<_>>();
            let ranking = payload
                .get("ranking")
                .and_then(Value::as_array)
                .ok_or_else(|| AppError::BadRequest("A complete ranking is required".to_owned()))?;
            let supplied = ranking
                .iter()
                .filter_map(Value::as_str)
                .collect::<HashSet<_>>();
            if ranking.len() != valid.len()
                || supplied.len() != ranking.len()
                || supplied.iter().any(|id| !valid.contains(*id))
            {
                return Err(AppError::BadRequest(
                    "Ranking must contain every option exactly once".to_owned(),
                ));
            }
        }
        "rating" => {
            let value = payload
                .get("value")
                .and_then(Value::as_f64)
                .ok_or_else(|| AppError::BadRequest("Rating is required".to_owned()))?;
            let min = config
                .get("min")
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(1.0);
            let max = config
                .get("max")
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(5.0);
            if value < min || value > max {
                return Err(AppError::BadRequest("Rating is out of range".to_owned()));
            }
        }
        "number" => {
            let value = payload
                .get("value")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite())
                .ok_or_else(|| AppError::BadRequest("A finite number is required".to_owned()))?;
            let min = config_decimal(config, "min", 0.0);
            let max = config_decimal(config, "max", 100.0);
            if value < min || value > max {
                return Err(AppError::BadRequest(
                    "The estimate is outside the configured range".to_owned(),
                ));
            }
        }
        "allocation" => {
            let valid = options
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|option| option.get("id").and_then(Value::as_str))
                .collect::<HashSet<_>>();
            let allocations = payload
                .get("allocations")
                .and_then(Value::as_object)
                .ok_or_else(|| AppError::BadRequest("Allocations are required".to_owned()))?;
            if allocations.len() != valid.len()
                || allocations.keys().any(|id| !valid.contains(id.as_str()))
                || allocations
                    .values()
                    .any(|value| value.as_i64().is_none_or(|value| value < 0))
            {
                return Err(AppError::BadRequest(
                    "Every allocation must be a non-negative whole number".to_owned(),
                ));
            }
            let total = config
                .get("total")
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(100);
            let supplied = allocations.values().filter_map(Value::as_i64).sum::<i64>();
            if supplied != total {
                return Err(AppError::BadRequest(format!(
                    "Allocations must add up to {total}"
                )));
            }
        }
        "matrix" => {
            let x = payload
                .get("x")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite())
                .ok_or_else(|| AppError::BadRequest("A finite x value is required".to_owned()))?;
            let y = payload
                .get("y")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite())
                .ok_or_else(|| AppError::BadRequest("A finite y value is required".to_owned()))?;
            if x < config_decimal(config, "x-min", 0.0)
                || x > config_decimal(config, "x-max", 10.0)
                || y < config_decimal(config, "y-min", 0.0)
                || y > config_decimal(config, "y-max", 10.0)
            {
                return Err(AppError::BadRequest(
                    "The matrix point is outside the configured range".to_owned(),
                ));
            }
        }
        "image-hotspot" => {
            let x = payload
                .get("x")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite())
                .ok_or_else(|| {
                    AppError::BadRequest("A finite x position is required".to_owned())
                })?;
            let y = payload
                .get("y")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite())
                .ok_or_else(|| {
                    AppError::BadRequest("A finite y position is required".to_owned())
                })?;
            if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
                return Err(AppError::BadRequest(
                    "Image hotspot coordinates must be normalized between 0 and 1".to_owned(),
                ));
            }
        }
        "survey" => {
            let answers = payload
                .get("answers")
                .and_then(Value::as_object)
                .ok_or_else(|| AppError::BadRequest("Survey answers are required".to_owned()))?;
            let questions = config
                .get("questions")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    AppError::BadRequest("Survey configuration is invalid".to_owned())
                })?;
            let valid_ids = questions
                .iter()
                .filter_map(|question| question.get("id").and_then(Value::as_str))
                .collect::<HashSet<_>>();
            if answers.keys().any(|id| !valid_ids.contains(id.as_str())) {
                return Err(AppError::BadRequest(
                    "Survey contains an unknown answer".to_owned(),
                ));
            }
            for question in questions {
                let id = question
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let required = question
                    .get("required")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let answer = answers.get(id);
                if answer.is_none() && required {
                    return Err(AppError::BadRequest(format!(
                        "Survey question `{id}` is required"
                    )));
                }
                let Some(answer) = answer else { continue };
                match question.get("type").and_then(Value::as_str) {
                    Some("rating") => {
                        let value = answer.as_i64().ok_or_else(|| {
                            AppError::BadRequest(format!(
                                "Survey rating `{id}` must be a whole number"
                            ))
                        })?;
                        let min = question.get("min").and_then(Value::as_i64).unwrap_or(1);
                        let max = question.get("max").and_then(Value::as_i64).unwrap_or(5);
                        if !(min..=max).contains(&value) {
                            return Err(AppError::BadRequest(format!(
                                "Survey rating `{id}` is outside its configured range"
                            )));
                        }
                    }
                    Some("choice") => {
                        let value = answer.as_str().unwrap_or_default();
                        let valid = question
                            .get("options")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .any(|option| option.get("id").and_then(Value::as_str) == Some(value));
                        if !valid {
                            return Err(AppError::BadRequest(format!(
                                "Survey choice `{id}` is invalid"
                            )));
                        }
                    }
                    Some("text") => {
                        let text = answer.as_str().unwrap_or_default().trim();
                        let max =
                            question.get("max").and_then(Value::as_i64).unwrap_or(500) as usize;
                        if text.is_empty() || text.chars().count() > max {
                            return Err(AppError::BadRequest(format!(
                                "Survey text `{id}` is empty or too long"
                            )));
                        }
                        reject_unsafe_audience_text(text, "Survey response")?;
                    }
                    _ => {
                        return Err(AppError::BadRequest(
                            "Survey configuration is invalid".to_owned(),
                        ));
                    }
                }
            }
        }
        "word-cloud" => {
            let maximum_entries = if config
                .get("entries")
                .and_then(Value::as_str)
                .unwrap_or("multiple")
                == "one"
            {
                1
            } else {
                MAX_WORD_CLOUD_ENTRIES_PER_PARTICIPANT
            };
            let has_texts = payload.get("texts").is_some();
            let has_text = payload.get("text").is_some();
            if has_texts == has_text {
                return Err(AppError::BadRequest(
                    "Provide exactly one word-cloud response shape".to_owned(),
                ));
            }
            if let Some(texts) = payload.get("texts").and_then(Value::as_array) {
                if texts.len() > maximum_entries {
                    return Err(AppError::BadRequest(if maximum_entries == 1 {
                        "This word cloud accepts one entry per participant".to_owned()
                    } else {
                        format!(
                            "A participant can keep up to {MAX_WORD_CLOUD_ENTRIES_PER_PARTICIPANT} word-cloud entries"
                        )
                    }));
                }
                let mut unique = HashSet::new();
                for text in texts {
                    let Some(text) = text.as_str() else {
                        return Err(AppError::BadRequest(
                            "Word-cloud entries must be text".to_owned(),
                        ));
                    };
                    let text = text.trim();
                    if text.is_empty()
                        || text.chars().count() > MAX_WORD_CLOUD_ENTRY_CHARS
                        || text.len() > MAX_WORD_CLOUD_ENTRY_BYTES
                    {
                        return Err(AppError::BadRequest(format!(
                            "Word-cloud entries must be at most {MAX_WORD_CLOUD_ENTRY_CHARS} characters"
                        )));
                    }
                    reject_unsafe_audience_text(text, "Word-cloud entry")?;
                    if !unique.insert(text.to_lowercase()) {
                        return Err(AppError::BadRequest(
                            "A word-cloud response cannot contain the same entry twice".to_owned(),
                        ));
                    }
                }
            } else {
                // Keep accepting the original single-entry payload while older audience
                // clients and stored responses roll forward.
                let text = payload
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim();
                if text.is_empty()
                    || text.chars().count() > MAX_WORD_CLOUD_ENTRY_CHARS
                    || text.len() > MAX_WORD_CLOUD_ENTRY_BYTES
                {
                    return Err(AppError::BadRequest(format!(
                        "Word-cloud entries must be at most {MAX_WORD_CLOUD_ENTRY_CHARS} characters"
                    )));
                }
                reject_unsafe_audience_text(text, "Word-cloud entry")?;
            }
        }
        "free-text" => {
            let text = payload
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            if text.is_empty() || text.chars().count() > 1000 {
                return Err(AppError::BadRequest(
                    "Text response is empty or too long".to_owned(),
                ));
            }
            reject_unsafe_audience_text(text, "Text response")?;
        }
        _ => return Err(AppError::BadRequest("Unsupported response type".to_owned())),
    }
    Ok(())
}

fn validate_response_shape(kind: &str, payload: &Value) -> AppResult<()> {
    let object = payload
        .as_object()
        .ok_or_else(|| AppError::BadRequest("Interaction response must be an object".to_owned()))?;
    let allowed: &[&str] = match kind {
        "poll" | "quiz" | "reaction" | "image-choice" => &["option_id", "option_ids"],
        "vote" | "updown" | "rating" | "number" => &["value"],
        "ranked-list" => &["votes"],
        "ranking" => &["ranking"],
        "allocation" => &["allocations"],
        "matrix" | "image-hotspot" => &["x", "y"],
        "survey" => &["answers"],
        "word-cloud" => &["text", "texts"],
        "free-text" => &["text"],
        _ => return Err(AppError::BadRequest("Unsupported response type".to_owned())),
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(AppError::BadRequest(
            "Interaction response contains unsupported fields".to_owned(),
        ));
    }
    Ok(())
}

fn normalize_response_payload(kind: &str, payload: &mut Value) -> AppResult<()> {
    if kind != "word-cloud" {
        return Ok(());
    }
    if let Some(text) = payload.get_mut("text") {
        let value = text
            .as_str()
            .ok_or_else(|| AppError::BadRequest("Word-cloud entries must be text".to_owned()))?;
        *text = Value::String(normalize_audience_text(value));
    }
    if let Some(texts) = payload.get_mut("texts") {
        let values = texts
            .as_array_mut()
            .ok_or_else(|| AppError::BadRequest("Word-cloud entries must be a list".to_owned()))?;
        for text in values {
            let value = text.as_str().ok_or_else(|| {
                AppError::BadRequest("Word-cloud entries must be text".to_owned())
            })?;
            *text = Value::String(normalize_audience_text(value));
        }
    }
    Ok(())
}

fn normalize_audience_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn reject_unsafe_audience_text(value: &str, subject: &str) -> AppResult<()> {
    let unsafe_character = value.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '\u{200B}'..='\u{200F}'
                    | '\u{202A}'..='\u{202E}'
                    | '\u{2060}'..='\u{206F}'
                    | '\u{FEFF}'
            )
    });
    if unsafe_character {
        return Err(AppError::BadRequest(format!(
            "{subject} contains unsupported control characters"
        )));
    }
    Ok(())
}

fn config_decimal(config: &Value, key: &str, default: f64) -> f64 {
    config
        .get(key)
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(default)
}

fn audience_asset_url(join_code: &str, source: &str) -> Option<String> {
    let segments = source
        .strip_prefix("/api/decks/")?
        .split('/')
        .collect::<Vec<_>>();
    if segments.len() != 4
        || Uuid::parse_str(segments[0]).is_err()
        || segments[1] != "assets"
        || Uuid::parse_str(segments[2]).is_err()
        || segments[3] != "content"
    {
        return None;
    }
    Some(format!("/api/join/{join_code}/assets/{}", segments[2]))
}

fn validate_ranked_revealed_votes(
    options: &Value,
    payload: &Value,
    revealed_count: i32,
) -> AppResult<()> {
    let available = options
        .as_array()
        .map(|options| {
            options
                .iter()
                .take(revealed_count.max(0) as usize)
                .filter_map(|option| option.get("id").and_then(Value::as_str))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let votes = payload
        .get("votes")
        .and_then(Value::as_object)
        .ok_or_else(|| AppError::BadRequest("Ranked-list votes are required".to_owned()))?;
    if votes.keys().any(|id| !available.contains(&id.as_str())) {
        return Err(AppError::Conflict(
            "One or more voted ideas have not been revealed yet".to_owned(),
        ));
    }
    Ok(())
}

async fn participant_from_headers(
    db: &PgPool,
    deck_id: Uuid,
    headers: &HeaderMap,
) -> AppResult<Option<Uuid>> {
    let Some(token) = cookie_value(headers, AUDIENCE_COOKIE) else {
        return Ok(None);
    };
    Ok(sqlx::query_scalar(
        "SELECT id FROM participant_sessions WHERE token_hash = $1 AND deck_id = $2 AND expires_at > now()",
    )
    .bind(hash_token(token))
    .bind(deck_id)
    .fetch_optional(db)
    .await?)
}

async fn deck_id_from_code(db: &PgPool, join_code: &str) -> AppResult<Uuid> {
    sqlx::query_scalar("SELECT id FROM decks WHERE join_code = $1")
        .bind(join_code.trim().to_uppercase())
        .fetch_optional(db)
        .await?
        .ok_or(AppError::NotFound)
}

async fn assert_owner(db: &PgPool, deck_id: Uuid, user_id: Uuid) -> AppResult<()> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM decks WHERE id = $1 AND owner_id = $2)")
            .bind(deck_id)
            .bind(user_id)
            .fetch_one(db)
            .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::NotFound)
    }
}

async fn insert_deck_with_unique_code(
    tx: &mut Transaction<'_, Postgres>,
    deck_id: Uuid,
    owner_id: Uuid,
    title: &str,
    markdown: &str,
    css: &str,
) -> AppResult<String> {
    for _ in 0..10 {
        let join_code = new_join_code();
        let result = sqlx::query(
            "INSERT INTO decks (id, owner_id, title, join_code, markdown, deck_css) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(deck_id)
        .bind(owner_id)
        .bind(title)
        .bind(&join_code)
        .bind(markdown)
        .bind(css)
        .execute(&mut **tx)
        .await;
        match result {
            Ok(_) => return Ok(join_code),
            Err(sqlx::Error::Database(error))
                if error.constraint() == Some("decks_join_code_key") =>
            {
                continue;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(AppError::Internal(anyhow::anyhow!(
        "could not allocate a join code"
    )))
}

async fn sync_interactions(
    tx: &mut Transaction<'_, Postgres>,
    deck_id: Uuid,
    version: i64,
    definitions: &[InteractionDefinition],
) -> AppResult<()> {
    sqlx::query("UPDATE interaction_definitions SET is_archived = true, updated_at = now() WHERE deck_id = $1")
        .bind(deck_id)
        .execute(&mut **tx)
        .await?;
    for (source_order, definition) in definitions.iter().enumerate() {
        let mut config = definition.config.clone();
        if let Value::Object(config) = &mut config {
            config.insert("element".to_owned(), Value::Bool(definition.element));
        }
        sqlx::query(
            r#"
            INSERT INTO interaction_definitions
                (id, deck_id, interaction_key, slide_key, slide_number, kind, title, config, options, source_revision, source_order)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (deck_id, interaction_key) DO UPDATE
            SET slide_key = EXCLUDED.slide_key,
                slide_number = EXCLUDED.slide_number,
                kind = EXCLUDED.kind,
                title = EXCLUDED.title,
                config = EXCLUDED.config,
                options = EXCLUDED.options,
                source_revision = EXCLUDED.source_revision,
                source_order = EXCLUDED.source_order,
                is_archived = false,
                updated_at = now()
            "#,
        )
        .bind(Uuid::now_v7())
        .bind(deck_id)
        .bind(&definition.id)
        .bind(&definition.slide_key)
        .bind(definition.slide_number)
        .bind(&definition.kind)
        .bind(&definition.title)
        .bind(config)
        .bind(serde_json::to_value(&definition.options).unwrap_or_else(|_| json!([])))
        .bind(version)
        .bind(source_order as i32)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn ensure_response_contract_integrity(
    tx: &mut Transaction<'_, Postgres>,
    deck_id: Uuid,
    definitions: &[InteractionDefinition],
) -> AppResult<()> {
    let existing = sqlx::query(
        r#"
        SELECT interaction_key, kind, config, options
        FROM interaction_definitions AS definition
        WHERE deck_id = $1
          AND EXISTS (
              SELECT 1
              FROM response_events AS response
              WHERE response.deck_id = definition.deck_id
                AND response.interaction_key = definition.interaction_key
          )
        "#,
    )
    .bind(deck_id)
    .fetch_all(&mut **tx)
    .await?;

    let proposed = definitions
        .iter()
        .map(|definition| (definition.id.as_str(), definition))
        .collect::<HashMap<_, _>>();

    for row in existing {
        let interaction_key: String = row.try_get("interaction_key")?;
        let Some(definition) = proposed.get(interaction_key.as_str()) else {
            // Removing an interaction archives its existing definition. Reusing that ID
            // later is still checked because archived definitions remain in this query.
            continue;
        };
        let old_kind: String = row.try_get("kind")?;
        let old_config: Value = row.try_get("config")?;
        let old_options: Value = row.try_get("options")?;
        if let Some(reason) =
            incompatible_response_contract(&old_kind, &old_config, &old_options, definition)
        {
            return Err(AppError::ResponseIntegrity(format!(
                "Interaction `{interaction_key}` already has saved responses and {reason}. Give the changed interaction a new ID, for example `{interaction_key}-v2`; the existing ID may still be moved or have its wording, labels, layout, timer, and result visibility changed."
            )));
        }
    }
    Ok(())
}

fn incompatible_response_contract(
    old_kind: &str,
    old_config: &Value,
    old_options: &Value,
    proposed: &InteractionDefinition,
) -> Option<String> {
    if old_kind != proposed.kind {
        return Some(format!(
            "its type changed from `{old_kind}` to `{}`",
            proposed.kind
        ));
    }

    let old_element = old_config
        .get("element")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if old_element != proposed.element {
        return Some("it changed between slide-level and element-level input".to_owned());
    }

    let old_option_ids = option_ids_from_value(old_options);
    let new_option_ids = proposed
        .options
        .iter()
        .map(|option| option.id.as_str())
        .collect::<HashSet<_>>();
    if let Some(removed) = old_option_ids
        .iter()
        .find(|option| !new_option_ids.contains(option.as_str()))
    {
        return Some(format!("option ID `{removed}` was removed or replaced"));
    }
    if matches!(old_kind, "allocation" | "ranking") && old_option_ids.len() != new_option_ids.len()
    {
        return Some(format!(
            "the option set changed for `{old_kind}`, whose responses must include every option"
        ));
    }

    if old_kind == "image-choice" {
        for old_option in old_options.as_array().into_iter().flatten() {
            let Some(id) = old_option.get("id").and_then(Value::as_str) else {
                continue;
            };
            let Some(new_option) = proposed.options.iter().find(|option| option.id == id) else {
                continue;
            };
            if old_option.get("image_url").and_then(Value::as_str)
                != new_option.image_url.as_deref()
            {
                return Some(format!("the image for option ID `{id}` changed"));
            }
        }
    }

    let changed = match old_kind {
        "poll" => {
            poll_accepts_multiple(old_config) != poll_accepts_multiple(&proposed.config)
                || optional_integer_config(old_config, "max")
                    != optional_integer_config(&proposed.config, "max")
        }
        "quiz" => {
            string_config(old_config, "correct") != string_config(&proposed.config, "correct")
        }
        "rating" => {
            decimal_contract(old_config, "min", 1.0)
                != decimal_contract(&proposed.config, "min", 1.0)
                || decimal_contract(old_config, "max", 5.0)
                    != decimal_contract(&proposed.config, "max", 5.0)
        }
        "number" => {
            decimal_contract(old_config, "min", 0.0)
                != decimal_contract(&proposed.config, "min", 0.0)
                || decimal_contract(old_config, "max", 100.0)
                    != decimal_contract(&proposed.config, "max", 100.0)
                || decimal_contract(old_config, "step", 1.0)
                    != decimal_contract(&proposed.config, "step", 1.0)
                || string_config(old_config, "unit") != string_config(&proposed.config, "unit")
        }
        "allocation" => {
            integer_contract(old_config, "total", 100)
                != integer_contract(&proposed.config, "total", 100)
        }
        "matrix" => {
            ["x-min", "x-max", "y-min", "y-max"]
                .into_iter()
                .zip([0.0, 10.0, 0.0, 10.0])
                .any(|(key, default)| {
                    decimal_contract(old_config, key, default)
                        != decimal_contract(&proposed.config, key, default)
                })
                || ["x-label", "y-label"].into_iter().any(|key| {
                    string_config(old_config, key) != string_config(&proposed.config, key)
                })
        }
        "image-hotspot" => {
            string_config(old_config, "image") != string_config(&proposed.config, "image")
        }
        "survey" => {
            return incompatible_survey_contract(old_config, &proposed.config);
        }
        _ => false,
    };

    changed.then(|| "its response validation or interpretation changed".to_owned())
}

fn option_ids_from_value(options: &Value) -> HashSet<String> {
    options
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|option| option.get("id").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect()
}

fn poll_accepts_multiple(config: &Value) -> bool {
    config.get("multiple").and_then(Value::as_str) == Some("true") || config.get("max").is_some()
}

fn optional_integer_config(config: &Value, key: &str) -> Option<i64> {
    config
        .get(key)
        .and_then(Value::as_str)
        .and_then(|value| value.parse().ok())
}

fn integer_contract(config: &Value, key: &str, default: i64) -> i64 {
    optional_integer_config(config, key).unwrap_or(default)
}

fn decimal_contract(config: &Value, key: &str, default: f64) -> u64 {
    config_decimal(config, key, default).to_bits()
}

fn string_config<'a>(config: &'a Value, key: &str) -> Option<&'a str> {
    config.get(key).and_then(Value::as_str)
}

fn incompatible_survey_contract(old_config: &Value, new_config: &Value) -> Option<String> {
    let old_questions = old_config
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|question| {
            question
                .get("id")
                .and_then(Value::as_str)
                .map(|id| (id, question))
        })
        .collect::<HashMap<_, _>>();
    let new_questions = new_config
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|question| {
            question
                .get("id")
                .and_then(Value::as_str)
                .map(|id| (id, question))
        })
        .collect::<HashMap<_, _>>();

    for (id, old_question) in old_questions {
        let Some(new_question) = new_questions.get(id) else {
            return Some(format!("survey question ID `{id}` was removed or replaced"));
        };
        let old_type = old_question.get("type").and_then(Value::as_str);
        let new_type = new_question.get("type").and_then(Value::as_str);
        if old_type != new_type {
            return Some(format!("survey question `{id}` changed type"));
        }
        if old_question.get("required").and_then(Value::as_bool)
            != new_question.get("required").and_then(Value::as_bool)
        {
            return Some(format!(
                "survey question `{id}` changed whether it is required"
            ));
        }
        match old_type {
            Some("rating") => {
                if old_question.get("min").and_then(Value::as_i64)
                    != new_question.get("min").and_then(Value::as_i64)
                    || old_question.get("max").and_then(Value::as_i64)
                        != new_question.get("max").and_then(Value::as_i64)
                {
                    return Some(format!("survey rating `{id}` changed its range"));
                }
            }
            Some("choice") => {
                let old_ids =
                    option_ids_from_value(old_question.get("options").unwrap_or(&Value::Null));
                let new_ids =
                    option_ids_from_value(new_question.get("options").unwrap_or(&Value::Null));
                if let Some(removed) = old_ids
                    .iter()
                    .find(|option| !new_ids.contains(option.as_str()))
                {
                    return Some(format!(
                        "survey choice `{id}` removed or replaced option ID `{removed}`"
                    ));
                }
            }
            Some("text") => {
                if old_question.get("max").and_then(Value::as_i64)
                    != new_question.get("max").and_then(Value::as_i64)
                {
                    return Some(format!(
                        "survey text question `{id}` changed its length limit"
                    ));
                }
            }
            _ => return Some(format!("survey question `{id}` has an invalid saved type")),
        }
    }
    None
}

async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    deck_id: Option<Uuid>,
    actor: Option<Uuid>,
    action: &str,
    data: Value,
) -> AppResult<()> {
    sqlx::query("INSERT INTO audit_events (id, deck_id, actor_user_id, action, data) VALUES ($1, $2, $3, $4, $5)")
        .bind(Uuid::now_v7())
        .bind(deck_id)
        .bind(actor)
        .bind(action)
        .bind(data)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn clean_title(title: &str) -> AppResult<String> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 120 {
        Err(AppError::BadRequest(
            "Title must be between 1 and 120 characters".to_owned(),
        ))
    } else {
        Ok(title.to_owned())
    }
}

fn new_join_code() -> String {
    const ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
    let mut rng = rand::rng();
    let raw = (0..6)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect::<String>();
    format!("{}-{}", &raw[..3], &raw[3..])
}

fn starter_markdown(title: &str) -> String {
    let yaml_title = serde_json::to_string(title)
        .unwrap_or_else(|_| format!("\"{}\"", title.replace('"', "\\\"")));
    format!(
        r#"---
theme: default
title: {yaml_title}
drawings:
  persist: false
transition: slide-left
---
<!-- interdeck-slide: welcome -->

# {title}

Scan to join the conversation.

::audience-qr{{size="180"}}

---
layout: center
---
<!-- interdeck-slide: interdeck-guide -->

# Create. Involve. Present.

- **Create** in `slides.md`, style in `style.css`, or ask the Gemini Slide Assistant.
- **Involve** your audience with **+ Interaction** for polls, Q&A, word clouds, voting, and more.
- **Preview** changes on the right, then press **Present** when you are ready.
- **Invite** the room with the QR code or join code. Only the current slide's interactions appear.

<small>Replace this guide when you no longer need it.</small>
"#
    )
}

fn starter_css() -> &'static str {
    r#"/* Interdeck house style — edit freely or replace with your own. */
:root {
  --interdeck-accent: var(--slidev-theme-primary, #6252cc);
  --interdeck-ink: #172033;
  --interdeck-muted: #647084;
  --interdeck-canvas: #fbfaf7;
  --interdeck-surface: rgba(255, 255, 255, 0.72);
  --interdeck-line: #dfe3ea;
}

.slidev-layout {
  color: var(--interdeck-ink);
  background:
    radial-gradient(circle at 92% 8%, color-mix(in srgb, var(--interdeck-accent) 11%, transparent), transparent 28%),
    var(--interdeck-canvas);
  font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  font-size: 1.05rem;
  line-height: 1.55;
  padding: 3rem 3.5rem;
  box-shadow: inset 0 0.32rem 0 var(--interdeck-accent);
}

.slidev-layout h1,
.slidev-layout h2,
.slidev-layout h3 {
  color: var(--interdeck-ink);
  font-weight: 750;
  letter-spacing: -0.035em;
  text-wrap: balance;
}

.slidev-layout h1 {
  font-size: 2.75rem;
  line-height: 1.08;
  margin-bottom: 1.25rem;
}

.slidev-layout h2 {
  font-size: 1.75rem;
  line-height: 1.15;
}

.slidev-layout h3 {
  color: var(--interdeck-muted);
  font-size: 1.25rem;
  letter-spacing: -0.02em;
}

.slidev-layout p,
.slidev-layout li {
  line-height: 1.55;
}

.slidev-layout li {
  margin-block: 0.3rem;
  padding-left: 0.2rem;
}

.slidev-layout li::marker {
  color: var(--interdeck-accent);
}

.slidev-layout strong {
  color: var(--interdeck-ink);
  font-weight: 720;
}

.slidev-layout a {
  color: var(--interdeck-accent);
  text-decoration-color: color-mix(in srgb, var(--interdeck-accent) 45%, transparent);
  text-underline-offset: 0.18em;
}

.slidev-layout blockquote {
  margin: 1.25rem 0;
  padding: 0.9rem 1.15rem;
  border-left: 0.28rem solid var(--interdeck-accent);
  border-radius: 0 0.75rem 0.75rem 0;
  background: var(--interdeck-surface);
  color: var(--interdeck-muted);
}

.slidev-layout :not(pre) > code {
  padding: 0.12em 0.38em;
  border: 1px solid var(--interdeck-line);
  border-radius: 0.35rem;
  background: var(--interdeck-surface);
  color: var(--interdeck-ink);
}

.slidev-layout pre {
  border: 1px solid var(--interdeck-line);
  border-radius: 0.85rem;
  box-shadow: 0 0.7rem 1.8rem rgba(23, 32, 51, 0.08);
}

.slidev-layout table {
  width: 100%;
  border-collapse: separate;
  border-spacing: 0;
  overflow: hidden;
  border: 1px solid var(--interdeck-line);
  border-radius: 0.75rem;
  background: var(--interdeck-surface);
}

.slidev-layout th {
  background: color-mix(in srgb, var(--interdeck-accent) 9%, white);
  color: var(--interdeck-ink);
  font-weight: 700;
}

.slidev-layout th,
.slidev-layout td {
  padding: 0.55rem 0.75rem;
  border-color: var(--interdeck-line);
}

.slidev-layout hr {
  border-color: var(--interdeck-line);
}
"#
}

fn sign_slidev_token(
    state: &AppState,
    deck_id: Uuid,
    version: i64,
    mode: &str,
) -> AppResult<String> {
    let payload = SlidevTokenPayload {
        deck_id,
        version,
        mode: mode.to_owned(),
        exp: Utc::now().timestamp() + 4 * 60 * 60,
    };
    let encoded = URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&payload).map_err(|error| AppError::Internal(error.into()))?);
    let mut mac = Hmac::<Sha256>::new_from_slice(state.config.slidev_token_secret.as_bytes())
        .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?;
    mac.update(encoded.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    Ok(format!("{encoded}.{signature}"))
}

fn audience_cookie(token: &str, secure: bool) -> String {
    format!(
        "{AUDIENCE_COOKIE}={token}; Path=/api/join; HttpOnly; SameSite=Lax; Max-Age={}{}",
        24 * 60 * 60,
        if secure { "; Secure" } else { "" }
    )
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

fn require_internal(state: &AppState, headers: &HeaderMap) -> AppResult<()> {
    let supplied = headers
        .get("x-interdeck-internal-token")
        .and_then(|value| value.to_str().ok())
        .ok_or(AppError::Unauthorized)?;
    if !constant_time_eq(
        supplied.as_bytes(),
        state.config.internal_service_token.as_bytes(),
    ) {
        return Err(AppError::Unauthorized);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_summary_turns_poll_aggregates_into_a_readable_leader() {
        let options = vec![
            InteractionOption {
                id: "safe".to_owned(),
                label: "Play it safe".to_owned(),
                image_url: None,
            },
            InteractionOption {
                id: "reinvent".to_owned(),
                label: "Reinvent".to_owned(),
                image_url: None,
            },
        ];
        let result = json!({ "count": 5, "counts": { "safe": 1, "reinvent": 4 } });
        let (headline, highlights) = summarize_interaction("poll", &options, &json!({}), &result);
        assert_eq!(headline, "Reinvent led with 4 selections");
        assert_eq!(highlights[0], "Reinvent: 4");
    }

    #[test]
    fn live_summary_does_not_invent_results_for_empty_interactions() {
        let (headline, highlights) =
            summarize_interaction("rating", &[], &json!({}), &json!({ "count": 0 }));
        assert_eq!(headline, "No responses yet");
        assert!(highlights.is_empty());
    }

    fn contract_definition(
        kind: &str,
        options: &[(&str, &str)],
        config: Value,
    ) -> InteractionDefinition {
        InteractionDefinition {
            id: "room-priority".to_owned(),
            slide_key: "closing".to_owned(),
            slide_number: 9,
            kind: kind.to_owned(),
            title: "Updated wording".to_owned(),
            options: options
                .iter()
                .map(|(id, label)| InteractionOption {
                    id: (*id).to_owned(),
                    label: (*label).to_owned(),
                    image_url: None,
                })
                .collect(),
            config,
            element: false,
        }
    }

    #[test]
    fn response_contract_allows_presentation_edits_and_additive_poll_options() {
        let old_options = json!([
            { "id": "delivery", "label": "Delivery" },
            { "id": "quality", "label": "Quality" }
        ]);
        let proposed = contract_definition(
            "poll",
            &[
                ("delivery", "Delivery excellence"),
                ("quality", "Product quality"),
                ("people", "People"),
            ],
            json!({ "results": "manual", "timer": "60" }),
        );

        assert_eq!(
            incompatible_response_contract(
                "poll",
                &json!({ "element": false, "results": "after-vote", "timer": "30" }),
                &old_options,
                &proposed,
            ),
            None
        );
    }

    #[test]
    fn response_contract_rejects_type_shape_and_stable_id_changes() {
        let old_options = json!([
            { "id": "delivery", "label": "Delivery" },
            { "id": "quality", "label": "Quality" }
        ]);

        let changed_type = contract_definition(
            "quiz",
            &[("delivery", "Delivery"), ("quality", "Quality")],
            json!({}),
        );
        assert!(
            incompatible_response_contract("poll", &json!({}), &old_options, &changed_type)
                .unwrap()
                .contains("type changed")
        );

        let changed_shape = contract_definition(
            "poll",
            &[("delivery", "Delivery"), ("quality", "Quality")],
            json!({ "multiple": "true" }),
        );
        assert!(
            incompatible_response_contract("poll", &json!({}), &old_options, &changed_shape)
                .unwrap()
                .contains("validation")
        );

        let removed_option = contract_definition("poll", &[("delivery", "Delivery")], json!({}));
        assert!(
            incompatible_response_contract("poll", &json!({}), &old_options, &removed_option)
                .unwrap()
                .contains("quality")
        );
    }

    #[test]
    fn complete_option_responses_reject_additive_options() {
        let old_options = json!([
            { "id": "delivery", "label": "Delivery" },
            { "id": "quality", "label": "Quality" }
        ]);
        for kind in ["allocation", "ranking"] {
            let proposed = contract_definition(
                kind,
                &[
                    ("delivery", "Delivery"),
                    ("quality", "Quality"),
                    ("people", "People"),
                ],
                json!({}),
            );
            assert!(
                incompatible_response_contract(kind, &json!({}), &old_options, &proposed)
                    .unwrap()
                    .contains("every option")
            );
        }
    }

    #[test]
    fn response_contract_rejects_scoring_ranges_and_element_scope_changes() {
        let options = json!([
            { "id": "yes", "label": "Yes" },
            { "id": "no", "label": "No" }
        ]);
        let quiz = contract_definition(
            "quiz",
            &[("yes", "Yes"), ("no", "No")],
            json!({ "correct": "no" }),
        );
        assert!(
            incompatible_response_contract("quiz", &json!({ "correct": "yes" }), &options, &quiz)
                .is_some()
        );

        let rating = contract_definition("rating", &[], json!({ "min": "0", "max": "5" }));
        assert!(
            incompatible_response_contract(
                "rating",
                &json!({ "min": "1", "max": "5" }),
                &json!([]),
                &rating
            )
            .is_some()
        );

        let mut element = contract_definition("rating", &[], json!({}));
        element.element = true;
        assert!(
            incompatible_response_contract(
                "rating",
                &json!({ "element": false }),
                &json!([]),
                &element
            )
            .unwrap()
            .contains("element-level")
        );
    }

    #[test]
    fn survey_contract_allows_wording_and_additions_but_protects_nested_ids() {
        let old_config = json!({
            "questions": [
                {
                    "id": "confidence",
                    "label": "How confident are you?",
                    "type": "rating",
                    "required": true,
                    "min": 1,
                    "max": 5
                },
                {
                    "id": "priority",
                    "label": "What matters?",
                    "type": "choice",
                    "required": true,
                    "options": [
                        { "id": "speed", "label": "Speed" },
                        { "id": "quality", "label": "Quality" }
                    ]
                }
            ]
        });
        let additive_config = json!({
            "questions": [
                {
                    "id": "confidence",
                    "label": "Confidence today?",
                    "type": "rating",
                    "required": true,
                    "min": 1,
                    "max": 5
                },
                {
                    "id": "priority",
                    "label": "Choose a priority",
                    "type": "choice",
                    "required": true,
                    "options": [
                        { "id": "speed", "label": "Move faster" },
                        { "id": "quality", "label": "Raise quality" },
                        { "id": "people", "label": "Support people" }
                    ]
                },
                {
                    "id": "context",
                    "label": "Anything else?",
                    "type": "text",
                    "required": false,
                    "max": 500
                }
            ]
        });
        let survey_options = json!([
            { "id": "confidence", "label": "How confident are you?" },
            { "id": "priority", "label": "What matters?" }
        ]);
        let additive = contract_definition(
            "survey",
            &[
                ("confidence", "Confidence today?"),
                ("priority", "Choose a priority"),
                ("context", "Anything else?"),
            ],
            additive_config.clone(),
        );
        assert_eq!(
            incompatible_response_contract("survey", &old_config, &survey_options, &additive),
            None
        );

        let mut removed_nested = additive_config;
        removed_nested["questions"][1]["options"] =
            json!([{ "id": "speed", "label": "Move faster" }]);
        let removed = contract_definition(
            "survey",
            &[("confidence", "Confidence"), ("priority", "Priority")],
            removed_nested,
        );
        assert!(
            incompatible_response_contract("survey", &old_config, &survey_options, &removed)
                .unwrap()
                .contains("quality")
        );
    }

    fn resolve_test_control(
        kind: &str,
        action: &str,
        phase: &str,
        requires_click_reveal: bool,
        click_step: i32,
        option_count: i32,
    ) -> AppResult<InteractionControl> {
        resolve_interaction_control(
            kind,
            action,
            phase,
            true,
            false,
            false,
            None,
            None,
            requires_click_reveal,
            click_step,
            option_count,
        )
    }

    #[test]
    fn audience_result_visibility_defaults_to_after_vote() {
        assert!(!audience_can_see_results(&json!({}), false, false));
        assert!(audience_can_see_results(&json!({}), true, false));
        assert!(audience_can_see_results(
            &json!({ "results": "always" }),
            false,
            false,
        ));
        assert!(!audience_can_see_results(
            &json!({ "results": "presenter" }),
            true,
            true,
        ));
        assert!(!audience_can_see_results(
            &json!({ "results": "manual" }),
            true,
            false,
        ));
        assert!(audience_can_see_results(
            &json!({ "results": "manual" }),
            false,
            true,
        ));
    }

    #[test]
    fn pilot_choice_types_enforce_their_response_contracts() {
        let options = json!([
            { "id": "one", "label": "One" },
            { "id": "two", "label": "Two" },
            { "id": "three", "label": "Three" }
        ]);
        assert!(
            validate_response_payload(
                "poll",
                &options,
                &json!({ "multiple": "true", "max": "2" }),
                &json!({ "option_ids": ["one", "two"] }),
            )
            .is_ok()
        );
        assert!(
            validate_response_payload(
                "poll",
                &options,
                &json!({ "multiple": "true", "max": "2" }),
                &json!({ "option_ids": ["one", "two", "three"] }),
            )
            .is_err()
        );
        assert!(
            validate_response_payload(
                "poll",
                &options,
                &json!({}),
                &json!({ "option_ids": ["one"] }),
            )
            .is_err()
        );
        assert!(
            validate_response_payload(
                "reaction",
                &options,
                &json!({}),
                &json!({ "option_id": "two" }),
            )
            .is_ok()
        );
    }

    #[test]
    fn quiz_answers_are_scored_without_leaking_the_key_early() {
        let options = vec![
            InteractionOption {
                id: "one".to_owned(),
                label: "One".to_owned(),
                image_url: None,
            },
            InteractionOption {
                id: "two".to_owned(),
                label: "Two".to_owned(),
                image_url: None,
            },
        ];
        let responses = vec![
            (Uuid::now_v7(), json!({ "option_id": "two" })),
            (Uuid::now_v7(), json!({ "option_id": "one" })),
        ];
        let hidden = aggregate(
            "quiz",
            &options,
            &json!({ "correct": "two" }),
            &responses,
            false,
        );
        assert!(hidden["correct_count"].is_null());
        assert!(hidden["correct_option_id"].is_null());

        let revealed = aggregate(
            "quiz",
            &options,
            &json!({ "correct": "two" }),
            &responses,
            true,
        );
        assert_eq!(revealed["correct_count"], 1);
        assert_eq!(revealed["correct_option_id"], "two");
    }

    #[test]
    fn structured_inputs_validate_and_aggregate() {
        let options = json!([
            { "id": "quality", "label": "Quality" },
            { "id": "speed", "label": "Speed" }
        ]);
        assert!(
            validate_response_payload(
                "number",
                &json!([]),
                &json!({ "min": "10", "max": "20" }),
                &json!({ "value": 15 }),
            )
            .is_ok()
        );
        assert!(
            validate_response_payload(
                "number",
                &json!([]),
                &json!({ "min": "10", "max": "20" }),
                &json!({ "value": 25 }),
            )
            .is_err()
        );
        assert!(
            validate_response_payload(
                "allocation",
                &options,
                &json!({ "total": "100" }),
                &json!({ "allocations": { "quality": 60, "speed": 40 } }),
            )
            .is_ok()
        );
        assert!(
            validate_response_payload(
                "allocation",
                &options,
                &json!({ "total": "100" }),
                &json!({ "allocations": { "quality": 60, "speed": 30 } }),
            )
            .is_err()
        );
        assert!(
            validate_response_payload(
                "matrix",
                &json!([]),
                &json!({ "x-min": "0", "x-max": "10", "y-min": "-5", "y-max": "5" }),
                &json!({ "x": 8, "y": -2 }),
            )
            .is_ok()
        );

        let parsed_options: Vec<InteractionOption> = serde_json::from_value(options).unwrap();
        let allocation = aggregate(
            "allocation",
            &parsed_options,
            &json!({}),
            &[
                (
                    Uuid::now_v7(),
                    json!({ "allocations": { "quality": 60, "speed": 40 } }),
                ),
                (
                    Uuid::now_v7(),
                    json!({ "allocations": { "quality": 80, "speed": 20 } }),
                ),
            ],
            true,
        );
        assert_eq!(allocation["averages"]["quality"], 70.0);

        let estimates = aggregate(
            "number",
            &[],
            &json!({}),
            &[
                (Uuid::now_v7(), json!({ "value": 10 })),
                (Uuid::now_v7(), json!({ "value": 20 })),
            ],
            true,
        );
        assert_eq!(estimates["average"], 15.0);
        assert_eq!(estimates["median"], 15.0);
    }

    #[test]
    fn word_cloud_accepts_multiple_entries_and_counts_people_not_duplicates() {
        assert!(
            validate_response_payload(
                "word-cloud",
                &json!([]),
                &json!({}),
                &json!({ "texts": ["Amazing", "Perfection"] }),
            )
            .is_ok()
        );
        assert!(
            validate_response_payload(
                "word-cloud",
                &json!([]),
                &json!({}),
                &json!({ "texts": [] }),
            )
            .is_ok()
        );
        assert!(
            validate_response_payload(
                "word-cloud",
                &json!([]),
                &json!({}),
                &json!({ "texts": vec!["word"; 51] }),
            )
            .is_err()
        );
        assert!(
            validate_response_payload(
                "word-cloud",
                &json!([]),
                &json!({ "entries": "one" }),
                &json!({ "texts": ["Amazing"] }),
            )
            .is_ok()
        );
        assert!(
            validate_response_payload(
                "word-cloud",
                &json!([]),
                &json!({ "entries": "one" }),
                &json!({ "texts": ["Amazing", "Perfection"] }),
            )
            .is_err()
        );
        assert!(
            validate_response_payload(
                "word-cloud",
                &json!([]),
                &json!({}),
                &json!({ "texts": ["Amazing", "amazing"] }),
            )
            .is_err()
        );

        let result = aggregate(
            "word-cloud",
            &[],
            &json!({}),
            &[
                (
                    Uuid::now_v7(),
                    json!({ "texts": ["Amazing", "Perfection", "amazing"] }),
                ),
                (Uuid::now_v7(), json!({ "texts": ["Amazing", "Curious"] })),
                (Uuid::now_v7(), json!({ "text": "Perfection" })),
            ],
            true,
        );
        assert_eq!(result["count"], 3);
        assert_eq!(result["submission_count"], 5);
        let words = result["words"].as_array().unwrap();
        let count_for = |needle: &str| {
            words
                .iter()
                .find(|word| word["text"] == needle)
                .and_then(|word| word["count"].as_i64())
        };
        assert_eq!(count_for("amazing"), Some(2));
        assert_eq!(count_for("perfection"), Some(2));
        assert_eq!(count_for("curious"), Some(1));
        assert_eq!(words[0]["text"], "amazing");
        assert_eq!(words[1]["text"], "perfection");

        let single_entry_result = aggregate(
            "word-cloud",
            &[],
            &json!({ "entries": "one" }),
            &[(Uuid::now_v7(), json!({ "texts": ["Earlier", "Latest"] }))],
            true,
        );
        assert_eq!(single_entry_result["submission_count"], 1);
        assert_eq!(single_entry_result["words"][0]["text"], "latest");
    }

    #[test]
    fn ranking_and_hotspot_validate_and_aggregate() {
        let option_value = json!([
            { "id": "quality", "label": "Quality" },
            { "id": "speed", "label": "Speed" },
            { "id": "learning", "label": "Learning" }
        ]);
        assert!(
            validate_response_payload(
                "ranking",
                &option_value,
                &json!({}),
                &json!({ "ranking": ["speed", "quality", "learning"] }),
            )
            .is_ok()
        );
        assert!(
            validate_response_payload(
                "ranking",
                &option_value,
                &json!({}),
                &json!({ "ranking": ["speed", "speed", "learning"] }),
            )
            .is_err()
        );
        assert!(
            validate_response_payload(
                "image-hotspot",
                &json!([]),
                &json!({}),
                &json!({ "x": 0.25, "y": 0.75 }),
            )
            .is_ok()
        );
        assert!(
            validate_response_payload(
                "image-hotspot",
                &json!([]),
                &json!({}),
                &json!({ "x": 1.25, "y": 0.75 }),
            )
            .is_err()
        );

        let options: Vec<InteractionOption> = serde_json::from_value(option_value).unwrap();
        let ranking = aggregate(
            "ranking",
            &options,
            &json!({}),
            &[
                (
                    Uuid::now_v7(),
                    json!({ "ranking": ["quality", "speed", "learning"] }),
                ),
                (
                    Uuid::now_v7(),
                    json!({ "ranking": ["speed", "quality", "learning"] }),
                ),
                (
                    Uuid::now_v7(),
                    json!({ "ranking": ["speed", "quality", "learning"] }),
                ),
            ],
            true,
        );
        assert_eq!(ranking["ranking"][0], "speed");
        assert_eq!(ranking["count"], 3);

        let hotspot = aggregate(
            "image-hotspot",
            &[],
            &json!({}),
            &[
                (Uuid::now_v7(), json!({ "x": 0.25, "y": 0.5 })),
                (Uuid::now_v7(), json!({ "x": 0.75, "y": 1.0 })),
            ],
            true,
        );
        assert_eq!(hotspot["average_x"], 0.5);
        assert_eq!(hotspot["average_y"], 0.75);
        assert_eq!(hotspot["points"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn mixed_surveys_are_atomic_and_aggregate_each_question() {
        let config = json!({ "questions": [
            { "id": "confidence", "label": "Confidence", "type": "rating", "required": true, "min": 1, "max": 5 },
            { "id": "priority", "label": "Priority", "type": "choice", "required": true, "options": [
                { "id": "quality", "label": "Quality" }, { "id": "speed", "label": "Speed" }
            ]},
            { "id": "comment", "label": "Comment", "type": "text", "required": false, "max": 100 }
        ]});
        assert!(
            validate_response_payload(
                "survey",
                &json!([]),
                &config,
                &json!({ "answers": { "confidence": 4, "priority": "quality" } }),
            )
            .is_ok()
        );
        assert!(
            validate_response_payload(
                "survey",
                &json!([]),
                &config,
                &json!({ "answers": { "confidence": 4 } }),
            )
            .is_err()
        );
        assert!(
            validate_response_payload(
                "survey",
                &json!([]),
                &config,
                &json!({ "answers": { "confidence": 7, "priority": "quality" } }),
            )
            .is_err()
        );

        let result = aggregate(
            "survey",
            &[],
            &config,
            &[
                (
                    Uuid::now_v7(),
                    json!({ "answers": { "confidence": 4, "priority": "quality", "comment": "More focus" } }),
                ),
                (
                    Uuid::now_v7(),
                    json!({ "answers": { "confidence": 2, "priority": "speed" } }),
                ),
            ],
            true,
        );
        assert_eq!(result["count"], 2);
        assert_eq!(result["questions"]["confidence"]["average"], 3.0);
        assert_eq!(result["questions"]["priority"]["counts"]["quality"], 1);
        assert_eq!(result["questions"]["comment"]["count"], 1);
    }

    #[test]
    fn rewrites_only_well_formed_uploaded_asset_urls_for_the_audience() {
        let deck = Uuid::now_v7();
        let asset = Uuid::now_v7();
        assert_eq!(
            audience_asset_url(
                "ABC-123",
                &format!("/api/decks/{deck}/assets/{asset}/content")
            ),
            Some(format!("/api/join/ABC-123/assets/{asset}"))
        );
        assert_eq!(
            audience_asset_url("ABC-123", "https://example.com/image.png"),
            None
        );
        assert_eq!(
            audience_asset_url("ABC-123", "/api/decks/nope/assets/nope/content"),
            None
        );
    }

    #[test]
    fn sea_creatures_sample_is_a_valid_interactive_deck() {
        let parsed = parse_deck(SEA_CREATURES_SAMPLE).expect("sample deck should parse");

        assert_eq!(parsed.slides.len(), 14);
        assert_eq!(parsed.interactions.len(), 10);
        assert!(
            parsed
                .interactions
                .iter()
                .any(|interaction| interaction.id == "ocean-superpowers")
        );
        assert!(parsed.interactions.iter().any(|interaction| {
            interaction.id == "adaptation-rank" && interaction.kind == "ranked-list"
        }));
        assert!(parsed.interactions.iter().any(|interaction| {
            interaction.id == "ocean-pulse" && interaction.kind == "survey"
        }));
        assert_eq!(
            parsed
                .interactions
                .iter()
                .filter(|interaction| !interaction.element)
                .filter(|interaction| interaction.config["show-title"] == "true")
                .count(),
            7
        );
    }

    #[test]
    fn ranked_list_aggregates_scores_and_keeps_source_order_for_ties() {
        let options = vec![
            InteractionOption {
                id: "quality".to_owned(),
                label: "Quality".to_owned(),
                image_url: None,
            },
            InteractionOption {
                id: "speed".to_owned(),
                label: "Speed".to_owned(),
                image_url: None,
            },
            InteractionOption {
                id: "learning".to_owned(),
                label: "Learning".to_owned(),
                image_url: None,
            },
        ];
        let responses = vec![
            (
                Uuid::now_v7(),
                json!({ "votes": { "quality": 1, "speed": -1 } }),
            ),
            (
                Uuid::now_v7(),
                json!({ "votes": { "quality": 1, "learning": 1 } }),
            ),
        ];

        let result = aggregate("ranked-list", &options, &json!({}), &responses, true);
        assert_eq!(result["count"], 2);
        assert_eq!(result["scores"]["quality"]["score"], 2);
        assert_eq!(result["ranking"], json!(["quality", "learning", "speed"]));
    }

    #[test]
    fn ranked_list_rejects_unknown_options() {
        let options = json!([
            { "id": "quality", "label": "Quality" },
            { "id": "speed", "label": "Speed" }
        ]);
        let result = validate_response_payload(
            "ranked-list",
            &options,
            &json!({}),
            &json!({ "votes": { "unknown": 1 } }),
        );
        assert!(result.is_err());
    }

    #[test]
    fn ranked_list_accepts_only_revealed_options() {
        let options = json!([
            { "id": "quality", "label": "Quality" },
            { "id": "speed", "label": "Speed" },
            { "id": "learning", "label": "Learning" }
        ]);
        let visible = validate_ranked_revealed_votes(
            &options,
            &json!({ "votes": { "quality": 1, "speed": -1 } }),
            2,
        );
        let hidden =
            validate_ranked_revealed_votes(&options, &json!({ "votes": { "learning": 1 } }), 2);
        assert!(visible.is_ok());
        assert!(hidden.is_err());
    }

    #[test]
    fn closing_an_ordinary_interaction_keeps_its_phase_and_slide_reveal() {
        let closed = resolve_test_control("poll", "close", "voting", false, 0, 2)
            .expect("a poll should close");
        assert_eq!(closed.phase, "voting");
        assert!(!closed.accepting_responses);
        assert!(!closed.replays_reveal);

        let reopened = resolve_test_control("word-cloud", "open", "voting", false, 0, 0)
            .expect("a word cloud should reopen");
        assert!(reopened.accepting_responses);

        // Resetting a non-ranked interaction must not rewind the slide's click step.
        let reset = resolve_test_control("rating", "reset", "voting", false, 3, 0)
            .expect("a rating should reset");
        assert!(reset.accepting_responses);
        assert!(!reset.replays_reveal);
    }

    #[test]
    fn result_reveal_is_independent_except_for_on_close_policy() {
        let revealed = resolve_interaction_control(
            "poll", "reveal", "voting", true, false, false, None, None, false, 0, 2,
        )
        .expect("manual results should reveal");
        assert!(revealed.accepting_responses);
        assert!(revealed.results_revealed);

        let hidden = resolve_interaction_control(
            "poll", "hide", "voting", false, true, false, None, None, false, 0, 2,
        )
        .expect("results should hide without reopening input");
        assert!(!hidden.accepting_responses);
        assert!(!hidden.results_revealed);

        let closed = resolve_interaction_control(
            "poll", "close", "voting", true, false, true, None, None, false, 0, 2,
        )
        .expect("on-close results should reveal");
        assert!(!closed.accepting_responses);
        assert!(closed.results_revealed);
    }

    #[test]
    fn ranked_lists_keep_their_reveal_guarded_phase_transitions() {
        assert!(resolve_test_control("ranked-list", "close", "voting", true, 2, 3).is_err());
        assert!(resolve_test_control("ranked-list", "close", "ranked", false, 0, 3).is_err());

        let closed = resolve_test_control("ranked-list", "close", "voting", true, 3, 3)
            .expect("a fully revealed ranked list should close");
        assert_eq!(closed.phase, "ranked");
        assert!(!closed.accepting_responses);

        let reset = resolve_test_control("ranked-list", "reset", "ranked", true, 3, 3)
            .expect("a ranked list should reset");
        assert_eq!(reset.phase, "voting");
        assert!(reset.accepting_responses);
        assert!(reset.replays_reveal);
    }

    #[test]
    fn unknown_control_actions_are_rejected_for_every_kind() {
        assert!(resolve_test_control("poll", "lock", "voting", false, 0, 2).is_err());
        assert!(resolve_test_control("ranked-list", "lock", "voting", false, 0, 2).is_err());
    }

    #[test]
    fn asset_detection_uses_file_signatures_not_browser_claims() {
        let png = detect_asset(b"\x89PNG\r\n\x1a\nrest").expect("PNG should be accepted");
        assert_eq!(png.media_type, "image/png");
        let woff2 = detect_asset(b"wOF2font-data").expect("WOFF2 should be accepted");
        assert_eq!(woff2.media_type, "font/woff2");
        assert!(detect_asset(b"<svg onload=alert(1)>").is_err());
        assert!(detect_asset(b"not really a png").is_err());
    }

    #[test]
    fn static_self_contained_svg_assets_are_supported() {
        let svg = br##"<?xml version="1.0" encoding="UTF-8"?>
            <!-- exported illustration -->
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 80">
              <defs>
                <linearGradient id="paint"><stop stop-color="#6252cc"/></linearGradient>
                <symbol id="mark"><path d="M0 0h8v8H0z"/></symbol>
              </defs>
              <rect width="120" height="80" fill="url('#paint')"/>
              <use href="#mark" x="12" y="12"/>
              <text x="10" y="70">https://slides.example.org</text>
            </svg>"##;
        let detected = detect_asset(svg).expect("static SVG should be accepted");
        assert_eq!(detected.media_type, "image/svg+xml");
        assert_eq!(detected.extension, "svg");
        assert_eq!(detected.max_bytes, 2_000_000);
    }

    #[test]
    fn active_or_externally_referencing_svg_assets_are_rejected() {
        let unsafe_svgs: &[&[u8]] = &[
            br#"<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"/>"#,
            br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#,
            br#"<svg xmlns="http://www.w3.org/2000/svg"><foreignObject><div>HTML</div></foreignObject></svg>"#,
            br#"<svg xmlns="http://www.w3.org/2000/svg"><use href="https://example.com/a.svg#mark"/></svg>"#,
            br#"<svg xmlns="http://www.w3.org/2000/svg"><image href="data:image/png;base64,AAAA"/></svg>"#,
            br#"<svg xmlns="http://www.w3.org/2000/svg"><animate attributeName="x"/></svg>"#,
            br#"<svg xmlns="http://www.w3.org/2000/svg"><rect fill="url(https://example.com/paint.svg#x)"/></svg>"#,
            br#"<!DOCTYPE svg [<!ENTITY xxe SYSTEM "file:///etc/passwd">]><svg>&xxe;</svg>"#,
            br#"<?xml version="1.0"?><?xml-stylesheet href="https://example.com/x.css"?><svg/>"#,
            br##"<svg xmlns="http://www.w3.org/2000/svg" xml:base="https://example.com/"><use href="#mark"/></svg>"##,
        ];
        for svg in unsafe_svgs {
            assert!(detect_asset(svg).is_err(), "unsafe SVG was accepted");
        }
    }

    #[test]
    fn asset_storage_filenames_are_safe_and_keep_detected_extensions() {
        assert_eq!(
            storage_filename("../../Quarterly Results.PNG", "png"),
            "quarterly-results.png"
        );
        assert_eq!(storage_filename("🔥.exe", "woff2"), "asset.woff2");
    }

    #[test]
    fn csv_cells_are_quoted_and_neutralize_spreadsheet_formulas() {
        assert_eq!(csv_cell("ordinary"), "\"ordinary\"");
        assert_eq!(csv_cell("a, \"quote\""), "\"a, \"\"quote\"\"\"");
        assert_eq!(
            csv_cell("=HYPERLINK(\"bad\")"),
            "\"'=HYPERLINK(\"\"bad\"\")\""
        );
        assert_eq!(csv_cell("  +1"), "\"'  +1\"");
    }
}
