mod ai;
mod auth;
mod config;
mod error;
mod maintenance;
mod markdown;
mod models;
mod routes;
mod slide_assistant;
mod state;
mod storage;

use std::{sync::Arc, time::Duration};

use anyhow::Context;
use axum::{
    Router,
    body::Body,
    extract::{DefaultBodyLimit, MatchedPath},
    http::Request,
    routing::{delete, get, patch, post, put},
};
use config::Config;
use sqlx::postgres::PgPoolOptions;
use state::AppState;
use tower_http::{
    catch_panic::CatchPanicLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let config = Config::from_env()?;
    let bind_addr = config.bind_addr.clone();
    let db = PgPoolOptions::new()
        .max_connections(30)
        .min_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&config.database_url)
        .await
        .context("connect to PostgreSQL")?;
    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .context("run database migrations")?;

    let state = AppState::new(config, db);
    state.start_event_listener();
    ai::spawn_worker(Arc::clone(&state));
    maintenance::spawn(Arc::clone(&state));
    let app = Router::new()
        .route("/health/live", get(routes::health_live))
        .route("/health/ready", get(routes::health_ready))
        .route("/api/auth/google/start", get(auth::login))
        .route("/api/auth/google/callback", get(auth::callback))
        .route("/api/auth/me", get(auth::me))
        .route("/api/config", get(routes::public_config))
        .route("/api/auth/logout", post(auth::logout))
        .route(
            "/api/decks",
            get(routes::list_decks).post(routes::create_deck),
        )
        .route(
            "/api/decks/{deck_id}",
            get(routes::get_deck)
                .put(routes::update_deck)
                .delete(routes::delete_deck),
        )
        .route(
            "/api/decks/{deck_id}/restore-last-verified",
            post(routes::restore_last_verified),
        )
        .route(
            "/api/decks/{deck_id}/assistant/propose",
            post(slide_assistant::propose),
        )
        .route(
            "/api/decks/{deck_id}/results.csv",
            get(routes::export_results_csv),
        )
        .route(
            "/api/decks/{deck_id}/join-code/rotate",
            post(routes::rotate_join_code),
        )
        .route(
            "/api/decks/{deck_id}/assets",
            get(routes::list_deck_assets).post(routes::upload_deck_asset),
        )
        .route(
            "/api/decks/{deck_id}/assets/{asset_id}/content",
            get(routes::deck_asset_content),
        )
        .route(
            "/api/decks/{deck_id}/assets/{asset_id}",
            delete(routes::delete_deck_asset),
        )
        .route(
            "/api/decks/{deck_id}/slidev-access",
            get(routes::slidev_access),
        )
        .route(
            "/api/decks/{deck_id}/present",
            post(routes::start_presentation),
        )
        .route(
            "/api/decks/{deck_id}/rehearse",
            post(routes::start_rehearsal),
        )
        .route("/api/decks/{deck_id}/stop", post(routes::stop_presentation))
        .route(
            "/api/decks/{deck_id}/input-control",
            post(routes::control_input),
        )
        .route("/api/decks/{deck_id}/navigate", post(routes::navigate))
        .route(
            "/api/decks/{deck_id}/interactions/{interaction_id}/control",
            post(routes::control_interaction),
        )
        .route("/api/decks/{deck_id}/reset", post(routes::reset_results))
        .route(
            "/api/decks/{deck_id}/qa-settings",
            patch(routes::update_qa_settings),
        )
        .route(
            "/api/decks/{deck_id}/qa-insights",
            post(routes::generate_qa_insights),
        )
        .route(
            "/api/decks/{deck_id}/live-state",
            get(routes::presenter_state),
        )
        .route(
            "/api/decks/{deck_id}/live-summary",
            get(routes::live_summary),
        )
        .route(
            "/api/decks/{deck_id}/questions/{question_id}",
            patch(routes::moderate_question),
        )
        .route(
            "/api/join/{join_code}",
            post(routes::join).layer(DefaultBodyLimit::max(2_048)),
        )
        .route(
            "/api/join/{join_code}/assets/{asset_id}",
            get(routes::audience_asset_content),
        )
        .route("/api/join/{join_code}/state", get(routes::audience_state))
        .route("/api/join/{join_code}/events", get(routes::audience_events))
        .route(
            "/api/join/{join_code}/interactions/{interaction_id}",
            put(routes::submit_response).layer(DefaultBodyLimit::max(16_384)),
        )
        .route(
            "/api/join/{join_code}/questions",
            post(routes::submit_question).layer(DefaultBodyLimit::max(4_096)),
        )
        .route(
            "/api/join/{join_code}/questions/{question_id}/vote",
            post(routes::vote_question),
        )
        .route(
            "/internal/decks/{deck_id}/source",
            get(routes::internal_deck_source),
        )
        .route(
            "/internal/decks/{deck_id}/builds",
            post(routes::internal_record_deck_build),
        )
        .route("/internal/metrics", get(routes::internal_metrics))
        .layer(DefaultBodyLimit::max(11_000_000))
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<Body>| {
                let route = request
                    .extensions()
                    .get::<MatchedPath>()
                    .map(MatchedPath::as_str)
                    .unwrap_or("unmatched");
                let request_id = request
                    .headers()
                    .get("x-request-id")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("");
                tracing::info_span!(
                    "http.request",
                    method = %request.method(),
                    route = route,
                    request_id = request_id,
                )
            }),
        )
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::new(
            axum::http::HeaderName::from_static("x-request-id"),
            MakeRequestUuid,
        ))
        .layer(CatchPanicLayer::new())
        .with_state(Arc::clone(&state));

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("bind API to {bind_addr}"))?;
    tracing::info!(address = %bind_addr, "Interdeck API listening");
    let shutdown_state = Arc::clone(&state);
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            shutdown_state.begin_shutdown();
        })
        .await
        .context("serve API")?;
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("interdeck=debug,tower_http=info"));
    if std::env::var("APP_ENV").as_deref() == Ok("production") {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .compact()
            .init();
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
