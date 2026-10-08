-- Expiry revokes audience cookies without deleting durable results.
ALTER TABLE participant_sessions ADD COLUMN expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '1 day';
UPDATE participant_sessions SET expires_at = created_at + interval '1 day';
CREATE INDEX participant_sessions_expiry_idx ON participant_sessions(expires_at);
CREATE INDEX oauth_states_expiry_idx ON oauth_states(expires_at);
CREATE INDEX auth_sessions_expiry_idx ON auth_sessions(expires_at);

-- Survives deck deletion and a crash between object upload and SQL commit.
CREATE TABLE asset_gc (
    storage_key TEXT PRIMARY KEY,
    available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX asset_gc_ready_idx ON asset_gc(available_at);
