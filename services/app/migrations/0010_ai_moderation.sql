ALTER TABLE questions
    ADD COLUMN analysis_status TEXT NOT NULL DEFAULT 'not_requested'
        CHECK (analysis_status IN ('not_requested', 'queued', 'complete', 'failed')),
    ADD COLUMN moderation_source TEXT NOT NULL DEFAULT 'human'
        CHECK (moderation_source IN ('human', 'ai')),
    ADD COLUMN moderation_labels JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN moderation_reason TEXT,
    ADD COLUMN topic TEXT,
    ADD COLUMN duplicate_of UUID REFERENCES questions(id) ON DELETE SET NULL,
    ADD COLUMN analyzed_at TIMESTAMPTZ;

CREATE TABLE background_jobs (
    id UUID PRIMARY KEY,
    deck_id UUID NOT NULL REFERENCES decks(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('question_moderation', 'deck_insights')),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'running', 'succeeded', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    locked_at TIMESTAMPTZ,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX background_jobs_ready_idx
    ON background_jobs(available_at, created_at)
    WHERE status = 'queued';

CREATE UNIQUE INDEX one_active_insights_job_per_deck_idx
    ON background_jobs(deck_id, kind)
    WHERE kind = 'deck_insights' AND status = 'queued';

CREATE TABLE question_analysis_versions (
    id UUID PRIMARY KEY,
    question_id UUID NOT NULL REFERENCES questions(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    suggested_action TEXT NOT NULL
        CHECK (suggested_action IN ('approve', 'review', 'reject')),
    confidence DOUBLE PRECISION NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
    labels JSONB NOT NULL DEFAULT '{}'::jsonb,
    reason TEXT NOT NULL,
    topic TEXT,
    duplicate_of UUID REFERENCES questions(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX question_analysis_history_idx
    ON question_analysis_versions(question_id, created_at DESC);

CREATE TABLE deck_insight_versions (
    id UUID PRIMARY KEY,
    deck_id UUID NOT NULL REFERENCES decks(id) ON DELETE CASCADE,
    result_epoch INTEGER NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    question_count INTEGER NOT NULL,
    summary TEXT NOT NULL,
    themes JSONB NOT NULL DEFAULT '[]'::jsonb,
    suggested_answers JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX deck_insight_latest_idx
    ON deck_insight_versions(deck_id, result_epoch, created_at DESC);
