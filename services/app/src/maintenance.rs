use std::{sync::Arc, time::Duration};

use anyhow::Result;
use sqlx::Row;

use crate::state::AppState;

pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(60));
        let mut shutdown = state.shutdown_receiver();
        loop {
            tokio::select! {
                _ = timer.tick() => {
                    if let Err(error) = run_once(&state).await {
                        tracing::warn!(error = %error, "maintenance will retry");
                    }
                }
                _ = shutdown.changed() => break,
            }
        }
    });
}

pub(crate) async fn run_once(state: &Arc<AppState>) -> Result<()> {
    sqlx::query("DELETE FROM oauth_states WHERE expires_at <= now()")
        .execute(&state.db)
        .await?;
    sqlx::query("DELETE FROM auth_sessions WHERE expires_at <= now()")
        .execute(&state.db)
        .await?;
    if state.config.audience_retention_days > 0 {
        let mut tx = state.db.begin().await?;
        // Lock decks before pruning so a presentation cannot start during purge.
        let decks: Vec<uuid::Uuid> = sqlx::query_scalar(
            "SELECT id FROM decks d WHERE NOT EXISTS (SELECT 1 FROM presentation_runs r WHERE r.deck_id = d.id AND r.status = 'live') FOR UPDATE OF d SKIP LOCKED",
        ).fetch_all(&mut *tx).await?;
        let cutoff = chrono::Utc::now()
            - chrono::Duration::days(i64::from(state.config.audience_retention_days));
        let mut changed = Vec::new();
        for deck_id in decks {
            let removed = sqlx::query(
                "DELETE FROM participant_sessions WHERE deck_id = $1 AND last_seen_at < $2",
            )
            .bind(deck_id)
            .bind(cutoff)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            sqlx::query("DELETE FROM deck_insight_versions WHERE deck_id = $1 AND created_at < $2")
                .bind(deck_id)
                .bind(cutoff)
                .execute(&mut *tx)
                .await?;
            if removed > 0 {
                changed.push(deck_id);
            }
        }
        tx.commit().await?;
        for deck_id in changed {
            state.notify(deck_id, 0);
        }
    }
    // Serialize storage deletion across replicas. A referenced object is never
    // deleted, including uploads whose SQL transaction committed just in time.
    for _ in 0..50 {
        let mut tx = state.db.begin().await?;
        let row = sqlx::query("SELECT storage_key FROM asset_gc WHERE available_at <= now() ORDER BY available_at FOR UPDATE SKIP LOCKED LIMIT 1")
            .fetch_optional(&mut *tx).await?;
        let Some(row) = row else { break };
        let key: String = row.try_get("storage_key")?;
        let referenced: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM deck_assets WHERE storage_key = $1)")
                .bind(&key)
                .fetch_one(&mut *tx)
                .await?;
        if referenced || state.assets.delete(&key).await.is_ok() {
            sqlx::query("DELETE FROM asset_gc WHERE storage_key = $1")
                .bind(&key)
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("UPDATE asset_gc SET attempts = attempts + 1, available_at = now() + interval '5 minutes' WHERE storage_key = $1")
                .bind(&key).execute(&mut *tx).await?;
            tracing::warn!("asset deletion deferred until storage recovers");
        }
        tx.commit().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use axum::http::{HeaderMap, HeaderValue};
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    #[ignore = "requires an explicitly configured disposable INTERDECK_TEST_DATABASE_URL"]
    async fn database_cleanup_retries_storage_and_preserves_live_data() -> Result<()> {
        let database_url = std::env::var("INTERDECK_TEST_DATABASE_URL")?;
        let parsed_url = url::Url::parse(&database_url)?;
        assert!(matches!(
            parsed_url.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]")
        ));
        let control = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await?;
        let schema = format!("readiness_{}", Uuid::now_v7().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&control)
            .await?;
        let options =
            PgConnectOptions::from_str(&database_url)?.options([("search_path", schema.as_str())]);
        let db = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        // Upgrade real pre-release fixtures, including old audience cookies.
        let mut prior = sqlx::migrate!("./migrations");
        prior.migrations = std::borrow::Cow::Owned(
            prior
                .iter()
                .filter(|migration| migration.version < 19)
                .cloned()
                .collect(),
        );
        prior.run(&db).await?;
        let root = std::env::temp_dir().join(&schema);
        tokio::fs::create_dir_all(&root).await?;
        let mut config = crate::config::test_config();
        config.asset_local_dir = root.to_string_lossy().into_owned();
        config.audience_retention_days = 7;
        let state = AppState::new(config, db.clone());
        let user = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO users(id, google_sub, email, display_name) VALUES($1,$2,$2,'Fixture')",
        )
        .bind(user)
        .bind(format!("{user}@example.invalid"))
        .execute(&db)
        .await?;
        let inactive = Uuid::now_v7();
        let live = Uuid::now_v7();
        for deck in [inactive, live] {
            sqlx::query("INSERT INTO decks(id,owner_id,title,join_code,markdown) VALUES($1,$2,'Fixture',$3,'# Fixture')")
                .bind(deck).bind(user).bind(deck.to_string()).execute(&db).await?;
            let participant = Uuid::now_v7();
            sqlx::query("INSERT INTO participant_sessions(id,token_hash,deck_id,created_at,last_seen_at) VALUES($1,$2,$3,now()-interval '8 days',now()-interval '8 days')")
                .bind(participant).bind(participant.to_string()).bind(deck).execute(&db).await?;
            sqlx::query("INSERT INTO current_responses(deck_id,interaction_key,participant_id,result_epoch,payload) VALUES($1,'poll',$2,1,'{}')")
                .bind(deck).bind(participant).execute(&db).await?;
        }
        sqlx::migrate!("./migrations").run(&db).await?;
        let expired_cookies: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM participant_sessions WHERE expires_at <= now()",
        )
        .fetch_one(&db)
        .await?;
        assert_eq!(
            expired_cookies, 2,
            "upgrading must not revive old audience cookies"
        );
        sqlx::query("INSERT INTO presentation_runs(id,deck_id,source_revision,result_epoch,status,started_by) VALUES($1,$2,1,1,'live',$3)")
            .bind(Uuid::now_v7()).bind(live).bind(user).execute(&db).await?;
        sqlx::query("INSERT INTO oauth_states(state_hash,pkce_verifier,expires_at) VALUES('expired','fixture',now()-interval '1 hour')")
            .execute(&db).await?;
        sqlx::query("INSERT INTO auth_sessions(token_hash,user_id,expires_at) VALUES('expired',$1,now()-interval '1 hour')")
            .bind(user).execute(&db).await?;

        // A queued referenced object is preserved; an orphan is deleted. A
        // directory at an object key simulates an unavailable deletion backend.
        for key in ["referenced", "orphan", "failed"] {
            sqlx::query("INSERT INTO asset_gc(storage_key) VALUES($1)")
                .bind(key)
                .execute(&db)
                .await?;
        }
        tokio::fs::write(root.join("referenced"), "keep").await?;
        tokio::fs::write(root.join("orphan"), "remove").await?;
        tokio::fs::create_dir(root.join("failed")).await?;
        sqlx::query("INSERT INTO deck_assets(id,deck_id,original_filename,media_type,byte_size,sha256,storage_key,created_by) VALUES($1,$2,'fixture.png','image/png',4,'fixture-hash','referenced',$3)")
            .bind(Uuid::now_v7()).bind(inactive).bind(user).execute(&db).await?;
        run_once(&state).await?;
        assert!(root.join("referenced").exists());
        assert!(!root.join("orphan").exists());
        let attempts: i32 =
            sqlx::query_scalar("SELECT attempts FROM asset_gc WHERE storage_key='failed'")
                .fetch_one(&db)
                .await?;
        assert_eq!(attempts, 1);
        let retained: Vec<Uuid> = sqlx::query_scalar("SELECT deck_id FROM current_responses")
            .fetch_all(&db)
            .await?;
        assert_eq!(retained, vec![live]);
        let expired: i64 = sqlx::query_scalar(
            "SELECT (SELECT count(*) FROM auth_sessions) + (SELECT count(*) FROM oauth_states)",
        )
        .fetch_one(&db)
        .await?;
        assert_eq!(expired, 0);
        tokio::fs::remove_dir(root.join("failed")).await?;
        tokio::fs::write(root.join("failed"), "retry").await?;
        sqlx::query("UPDATE asset_gc SET available_at=now() WHERE storage_key='failed'")
            .execute(&db)
            .await?;
        run_once(&state).await?;
        assert!(!root.join("failed").exists());

        // A failed SQL transaction cannot queue deletion of a committed asset.
        let mut tx = db.begin().await?;
        sqlx::query("INSERT INTO asset_gc(storage_key) VALUES('referenced')")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM deck_assets WHERE storage_key='referenced'")
            .execute(&mut *tx)
            .await?;
        tx.rollback().await?;
        run_once(&state).await?;
        assert!(root.join("referenced").exists());

        // The actual deck deletion route commits metadata and the outbox
        // together, even if the storage backend cannot delete immediately.
        let token = "maintenance-creator-fixture";
        sqlx::query("INSERT INTO auth_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 hour')")
            .bind(crate::auth::hash_token(token)).bind(user).execute(&db).await?;
        let mut headers = HeaderMap::new();
        headers.insert(
            "cookie",
            HeaderValue::from_str(&format!("interdeck_session={token}"))?,
        );
        crate::routes::delete_deck(
            axum::extract::State(Arc::clone(&state)),
            axum::extract::Path(inactive),
            headers,
        )
        .await?;
        let queued: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM asset_gc WHERE storage_key='referenced')",
        )
        .fetch_one(&db)
        .await?;
        assert!(queued);
        assert!(root.join("referenced").exists());
        run_once(&state).await?;
        assert!(!root.join("referenced").exists());

        state.begin_shutdown();
        tokio::time::sleep(Duration::from_millis(200)).await;
        db.close().await;
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&control)
            .await?;
        control.close().await;
        tokio::fs::remove_dir_all(root).await?;
        Ok(())
    }
}
