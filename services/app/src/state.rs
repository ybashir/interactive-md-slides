use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, postgres::PgListener};
use tokio::sync::{Mutex as AsyncMutex, broadcast, watch};
use uuid::Uuid;

use crate::{config::Config, storage::AssetStore};

pub struct AppState {
    pub config: Config,
    pub db: PgPool,
    pub http: reqwest::Client,
    pub assets: AssetStore,
    channels: DashMap<Uuid, broadcast::Sender<DeckEvent>>,
    audience_snapshots: DashMap<Uuid, (i64, Arc<str>)>,
    audience_snapshot_locks: DashMap<Uuid, Arc<AsyncMutex<()>>>,
    event_clocks: DashMap<Uuid, Arc<EventClock>>,
    rate_limits: DashMap<String, Arc<Mutex<RateWindow>>>,
    assistant_inflight: DashMap<String, ()>,
    instance_id: Uuid,
    event_listener_ready: AtomicBool,
    active_sse_connections: AtomicI64,
    audience_snapshot_builds: AtomicU64,
    audience_snapshot_failures: AtomicU64,
    ai_jobs_succeeded: AtomicU64,
    ai_job_attempt_failures: AtomicU64,
    ai_jobs_recovered: AtomicU64,
    shutdown: watch::Sender<bool>,
}

pub struct SseConnectionGuard {
    state: Arc<AppState>,
}

pub struct AssistantRequestGuard {
    state: Arc<AppState>,
    key: String,
}

impl Drop for SseConnectionGuard {
    fn drop(&mut self) {
        self.state
            .active_sse_connections
            .fetch_sub(1, Ordering::Relaxed);
    }
}

impl Drop for AssistantRequestGuard {
    fn drop(&mut self) {
        self.state.assistant_inflight.remove(&self.key);
    }
}

struct EventClock {
    sequence: AtomicI64,
    response_notification_scheduled: AtomicBool,
}

#[derive(Clone, Debug)]
pub struct DeckEvent {
    pub sequence: i64,
    pub audience_snapshot: Option<Arc<str>>,
}

struct RateWindow {
    started_at: Instant,
    count: u32,
}

#[derive(Debug, Deserialize, Serialize)]
struct CrossReplicaEvent {
    deck_id: Uuid,
    sequence: i64,
    instance_id: Uuid,
}

const EVENT_CHANNEL: &str = "interdeck_events";

impl AppState {
    pub fn new(config: Config, db: PgPool) -> Arc<Self> {
        let assets = AssetStore::from_config(&config);
        let (shutdown, _) = watch::channel(false);
        Arc::new(Self {
            config,
            db,
            http: reqwest::Client::builder()
                .user_agent("Interdeck/0.1")
                .build()
                .expect("HTTP client should build"),
            assets,
            channels: DashMap::new(),
            audience_snapshots: DashMap::new(),
            audience_snapshot_locks: DashMap::new(),
            event_clocks: DashMap::new(),
            rate_limits: DashMap::new(),
            assistant_inflight: DashMap::new(),
            instance_id: Uuid::now_v7(),
            event_listener_ready: AtomicBool::new(false),
            active_sse_connections: AtomicI64::new(0),
            audience_snapshot_builds: AtomicU64::new(0),
            audience_snapshot_failures: AtomicU64::new(0),
            ai_jobs_succeeded: AtomicU64::new(0),
            ai_job_attempt_failures: AtomicU64::new(0),
            ai_jobs_recovered: AtomicU64::new(0),
            shutdown,
        })
    }

    pub fn start_event_listener(self: &Arc<Self>) {
        let state = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                if let Err(error) = state.listen_once().await {
                    state.event_listener_ready.store(false, Ordering::Release);
                    tracing::warn!(error = %error, "PostgreSQL event listener disconnected");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        });
    }

    pub fn subscribe(&self, deck_id: Uuid) -> broadcast::Receiver<DeckEvent> {
        self.sender(deck_id).subscribe()
    }

    pub fn latest_audience_snapshot(&self, deck_id: Uuid) -> Option<(i64, Arc<str>)> {
        self.audience_snapshots
            .get(&deck_id)
            .map(|entry| (entry.value().0, Arc::clone(&entry.value().1)))
    }

    /// Warm the per-deck audience snapshot once after an instance starts.
    /// A reconnect storm waits on one build instead of issuing one full-state
    /// query per audience member.
    pub async fn ensure_audience_snapshot(
        self: &Arc<Self>,
        deck_id: Uuid,
    ) -> Option<(i64, Arc<str>)> {
        if let Some(snapshot) = self.latest_audience_snapshot(deck_id) {
            return Some(snapshot);
        }
        let lock = self
            .audience_snapshot_locks
            .entry(deck_id)
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let _guard = lock.lock().await;
        if let Some(snapshot) = self.latest_audience_snapshot(deck_id) {
            return Some(snapshot);
        }
        self.publish_local_snapshot(deck_id, 0).await;
        self.latest_audience_snapshot(deck_id)
    }

    pub fn track_sse(self: &Arc<Self>) -> SseConnectionGuard {
        self.active_sse_connections.fetch_add(1, Ordering::Relaxed);
        SseConnectionGuard {
            state: Arc::clone(self),
        }
    }

    pub fn shutdown_receiver(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    pub fn begin_shutdown(&self) {
        let _ = self.shutdown.send(true);
    }

    pub fn operational_metrics(&self) -> serde_json::Value {
        serde_json::json!({
            "instance_id": self.instance_id,
            "event_listener_ready": self.event_listener_ready.load(Ordering::Acquire),
            "active_sse_connections": self.active_sse_connections.load(Ordering::Relaxed),
            "active_deck_channels": self.channels.len(),
            "audience_snapshot_builds": self.audience_snapshot_builds.load(Ordering::Relaxed),
            "audience_snapshot_failures": self.audience_snapshot_failures.load(Ordering::Relaxed),
            "cached_audience_snapshots": self.audience_snapshots.len(),
            "audience_snapshot_locks": self.audience_snapshot_locks.len(),
            "ai_jobs_succeeded": self.ai_jobs_succeeded.load(Ordering::Relaxed),
            "ai_job_attempt_failures": self.ai_job_attempt_failures.load(Ordering::Relaxed),
            "ai_jobs_recovered": self.ai_jobs_recovered.load(Ordering::Relaxed),
            "rate_limit_keys": self.rate_limits.len(),
            "database_pool_size": self.db.size(),
            "database_pool_idle": self.db.num_idle(),
        })
    }

    pub fn record_ai_job_succeeded(&self) {
        self.ai_jobs_succeeded.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_ai_job_attempt_failure(&self) {
        self.ai_job_attempt_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_ai_jobs_recovered(&self, count: u64) {
        self.ai_jobs_recovered.fetch_add(count, Ordering::Relaxed);
    }

    pub fn notify(self: &Arc<Self>, deck_id: Uuid, durable_floor: i64) -> i64 {
        let sequence = self.next_event_sequence(deck_id, durable_floor);
        self.schedule_event(deck_id);
        sequence
    }

    pub fn notify_response(self: &Arc<Self>, deck_id: Uuid, durable_floor: i64) -> i64 {
        let clock = self.clock(deck_id, durable_floor);
        let sequence = clock
            .sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.max(durable_floor) + 1)
            })
            .unwrap_or(durable_floor)
            .max(durable_floor)
            + 1;
        if clock
            .response_notification_scheduled
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            let state = Arc::clone(self);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(125)).await;
                // Clear before reading. A response racing after this store either
                // appears in this snapshot or schedules the next invalidation.
                clock
                    .response_notification_scheduled
                    .store(false, Ordering::Release);
                let latest = clock.sequence.load(Ordering::Acquire);
                state.publish_snapshot_event(deck_id, latest).await;
            });
        }
        sequence
    }

    pub fn check_rate_limit(
        &self,
        namespace: &str,
        subject: impl std::fmt::Display,
        limit: u32,
        window: Duration,
    ) -> Result<(), u64> {
        if self.rate_limits.len() > 100_000 {
            self.rate_limits.retain(|_, value| {
                value
                    .lock()
                    .is_ok_and(|entry| entry.started_at.elapsed() < Duration::from_secs(600))
            });
        }
        let key = format!("{namespace}:{subject}");
        let entry = self
            .rate_limits
            .entry(key)
            .or_insert_with(|| {
                Arc::new(Mutex::new(RateWindow {
                    started_at: Instant::now(),
                    count: 0,
                }))
            })
            .clone();
        let mut entry = entry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if entry.started_at.elapsed() >= window {
            entry.started_at = Instant::now();
            entry.count = 0;
        }
        if entry.count >= limit {
            let remaining = window.saturating_sub(entry.started_at.elapsed());
            return Err(remaining.as_secs().max(1));
        }
        entry.count += 1;
        Ok(())
    }

    pub fn begin_assistant_request(
        self: &Arc<Self>,
        user_id: Uuid,
        deck_id: Uuid,
    ) -> Option<AssistantRequestGuard> {
        let key = format!("{user_id}:{deck_id}");
        self.assistant_inflight
            .insert(key.clone(), ())
            .is_none()
            .then(|| AssistantRequestGuard {
                state: Arc::clone(self),
                key,
            })
    }

    async fn listen_once(self: &Arc<Self>) -> anyhow::Result<()> {
        let mut listener = PgListener::connect(&self.config.database_url).await?;
        listener.listen(EVENT_CHANNEL).await?;
        self.event_listener_ready.store(true, Ordering::Release);
        tracing::info!(instance_id = %self.instance_id, "PostgreSQL event listener ready");
        loop {
            let notification = listener.recv().await?;
            let Ok(event) = serde_json::from_str::<CrossReplicaEvent>(notification.payload())
            else {
                tracing::warn!("ignored malformed cross-replica event");
                continue;
            };
            if event.instance_id == self.instance_id {
                continue;
            }
            let clock = self.clock(event.deck_id, event.sequence);
            let previous = clock.sequence.fetch_max(event.sequence, Ordering::AcqRel);
            self.publish_local_snapshot(event.deck_id, previous.max(event.sequence))
                .await;
        }
    }

    fn schedule_event(self: &Arc<Self>, deck_id: Uuid) {
        let clock = self.clock(deck_id, 0);
        if clock
            .response_notification_scheduled
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return;
        }
        let state = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(125)).await;
            clock
                .response_notification_scheduled
                .store(false, Ordering::Release);
            let latest = clock.sequence.load(Ordering::Acquire);
            state.publish_snapshot_event(deck_id, latest).await;
        });
    }

    async fn publish_snapshot_event(self: &Arc<Self>, deck_id: Uuid, sequence: i64) {
        self.publish_local_snapshot(deck_id, sequence).await;
        let Ok(payload) = serde_json::to_string(&CrossReplicaEvent {
            deck_id,
            sequence,
            instance_id: self.instance_id,
        }) else {
            return;
        };
        let db = self.db.clone();
        tokio::spawn(async move {
            if let Err(error) = sqlx::query("SELECT pg_notify($1, $2)")
                .bind(EVENT_CHANNEL)
                .bind(payload)
                .execute(&db)
                .await
            {
                tracing::warn!(error = %error, "could not publish cross-replica event");
            }
        });
    }

    async fn publish_local_snapshot(&self, deck_id: Uuid, sequence: i64) {
        self.audience_snapshot_builds
            .fetch_add(1, Ordering::Relaxed);
        let (effective_sequence, snapshot) = match crate::routes::build_shared_audience_state(
            &self.db, deck_id,
        )
        .await
        {
            Ok(state) => {
                let effective_sequence = sequence.max(state.sequence);
                let snapshot = serde_json::to_string(&serde_json::json!({
                    "type": "snapshot",
                    "sequence": effective_sequence,
                    "state": state,
                }))
                .ok()
                .map(Arc::<str>::from);
                (effective_sequence, snapshot)
            }
            Err(error) => {
                self.audience_snapshot_failures
                    .fetch_add(1, Ordering::Relaxed);
                tracing::warn!(%deck_id, %sequence, error = %error, "could not build audience SSE snapshot");
                (sequence, None)
            }
        };
        if let Some(snapshot) = &snapshot {
            if self
                .audience_snapshots
                .get(&deck_id)
                .is_some_and(|current| current.value().0 > effective_sequence)
            {
                return;
            }
            self.audience_snapshots
                .insert(deck_id, (effective_sequence, Arc::clone(snapshot)));
        }
        let _ = self.sender(deck_id).send(DeckEvent {
            sequence: effective_sequence,
            audience_snapshot: snapshot,
        });
    }

    fn next_event_sequence(&self, deck_id: Uuid, durable_floor: i64) -> i64 {
        let clock = self.clock(deck_id, durable_floor);
        clock
            .sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.max(durable_floor) + 1)
            })
            .unwrap_or(durable_floor)
            .max(durable_floor)
            + 1
    }

    fn clock(&self, deck_id: Uuid, durable_floor: i64) -> Arc<EventClock> {
        self.event_clocks
            .entry(deck_id)
            .or_insert_with(|| {
                Arc::new(EventClock {
                    sequence: AtomicI64::new(durable_floor),
                    response_notification_scheduled: AtomicBool::new(false),
                })
            })
            .clone()
    }

    fn sender(&self, deck_id: Uuid) -> broadcast::Sender<DeckEvent> {
        self.channels
            .entry(deck_id)
            .or_insert_with(|| broadcast::channel(256).0)
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use sqlx::postgres::PgPoolOptions;

    use super::*;

    fn test_state() -> Arc<AppState> {
        let db = PgPoolOptions::new()
            .connect_lazy("postgresql://postgres:postgres@localhost/interdeck")
            .expect("test database URL should parse");
        AppState::new(crate::config::test_config(), db)
    }

    #[tokio::test]
    async fn response_notifications_are_coalesced_with_the_latest_sequence() {
        let state = test_state();
        // This unit test exercises coalescing and its invalidation fallback,
        // without depending on a local PostgreSQL connection or its timeout.
        state.db.close().await;
        let deck_id = Uuid::now_v7();
        let mut receiver = state.subscribe(deck_id);
        let mut expected = 0;
        for _ in 0..100 {
            expected = state.notify_response(deck_id, 1);
        }

        let event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("coalesced notification should arrive")
            .expect("notification channel should remain open");
        assert_eq!(event.sequence, expected);
        assert!(event.audience_snapshot.is_none());
        assert!(matches!(
            receiver.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn fixed_window_rate_limit_reports_retry_time_and_resets() {
        let state = test_state();
        assert!(
            state
                .check_rate_limit("question", "participant", 2, Duration::from_secs(60))
                .is_ok()
        );
        assert!(
            state
                .check_rate_limit("question", "participant", 2, Duration::from_secs(60))
                .is_ok()
        );
        assert!(
            state
                .check_rate_limit("question", "participant", 2, Duration::from_secs(60))
                .is_err()
        );
        assert!(
            state
                .check_rate_limit("question", "someone-else", 2, Duration::from_secs(60))
                .is_ok()
        );
    }

    #[test]
    fn cross_replica_payload_does_not_expose_state() {
        let event = CrossReplicaEvent {
            deck_id: Uuid::now_v7(),
            sequence: 42,
            instance_id: Uuid::now_v7(),
        };
        let payload = serde_json::to_string(&event).unwrap();
        let decoded: CrossReplicaEvent = serde_json::from_str(&payload).unwrap();
        assert_eq!(decoded.deck_id, event.deck_id);
        assert_eq!(decoded.sequence, 42);
        assert!(!payload.contains("question"));
    }

    #[tokio::test]
    async fn sse_tracking_and_shutdown_are_observable() {
        let state = test_state();
        let mut shutdown = state.shutdown_receiver();
        assert_eq!(state.operational_metrics()["active_sse_connections"], 0);
        let connection = state.track_sse();
        assert_eq!(state.operational_metrics()["active_sse_connections"], 1);

        state.begin_shutdown();
        shutdown.changed().await.unwrap();
        assert!(*shutdown.borrow());
        drop(connection);
        assert_eq!(state.operational_metrics()["active_sse_connections"], 0);
    }
}
