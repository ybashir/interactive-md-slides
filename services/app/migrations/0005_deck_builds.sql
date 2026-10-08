CREATE TABLE deck_builds (
    id UUID PRIMARY KEY,
    deck_id UUID NOT NULL,
    source_revision BIGINT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('succeeded', 'failed')),
    source_sha256 TEXT NOT NULL,
    slidev_version TEXT,
    theme TEXT,
    startup_ms INTEGER,
    diagnostic_code TEXT,
    diagnostic_message TEXT,
    artifact_prefix TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (deck_id, source_revision),
    FOREIGN KEY (deck_id, source_revision)
        REFERENCES deck_revisions(deck_id, revision)
        ON DELETE CASCADE
);

CREATE INDEX deck_builds_successful_revision_idx
    ON deck_builds(deck_id, source_revision DESC)
    WHERE status = 'succeeded';
