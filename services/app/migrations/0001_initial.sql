CREATE TABLE users (
    id UUID PRIMARY KEY,
    google_sub TEXT NOT NULL UNIQUE,
    email TEXT NOT NULL,
    display_name TEXT NOT NULL,
    picture_url TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX users_email_lower_idx ON users ((lower(email)));

CREATE TABLE oauth_states (
    state_hash TEXT PRIMARY KEY,
    pkce_verifier TEXT NOT NULL,
    return_to TEXT NOT NULL DEFAULT '/',
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE auth_sessions (
    token_hash TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX auth_sessions_user_idx ON auth_sessions(user_id);

CREATE TABLE decks (
    id UUID PRIMARY KEY,
    owner_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    join_code TEXT NOT NULL UNIQUE,
    markdown TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 1,
    result_epoch INTEGER NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX decks_owner_updated_idx ON decks(owner_id, updated_at DESC);

CREATE TABLE deck_revisions (
    id UUID PRIMARY KEY,
    deck_id UUID NOT NULL REFERENCES decks(id) ON DELETE CASCADE,
    revision BIGINT NOT NULL,
    markdown TEXT NOT NULL,
    created_by UUID NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(deck_id, revision)
);

CREATE TABLE interaction_definitions (
    id UUID PRIMARY KEY,
    deck_id UUID NOT NULL REFERENCES decks(id) ON DELETE CASCADE,
    interaction_key TEXT NOT NULL,
    slide_key TEXT NOT NULL,
    slide_number INTEGER NOT NULL,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    options JSONB NOT NULL DEFAULT '[]'::jsonb,
    source_revision BIGINT NOT NULL,
    is_archived BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(deck_id, interaction_key)
);

CREATE INDEX interaction_definitions_slide_idx
    ON interaction_definitions(deck_id, slide_key)
    WHERE is_archived = false;

CREATE TABLE result_epochs (
    deck_id UUID NOT NULL REFERENCES decks(id) ON DELETE CASCADE,
    epoch INTEGER NOT NULL,
    created_by UUID NOT NULL REFERENCES users(id),
    reason TEXT NOT NULL DEFAULT 'reset',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(deck_id, epoch)
);

CREATE TABLE presentation_runs (
    id UUID PRIMARY KEY,
    deck_id UUID NOT NULL REFERENCES decks(id) ON DELETE CASCADE,
    source_revision BIGINT NOT NULL,
    result_epoch INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('live', 'ended')),
    started_by UUID NOT NULL REFERENCES users(id),
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX one_live_run_per_deck_idx
    ON presentation_runs(deck_id)
    WHERE status = 'live';

CREATE TABLE live_deck_states (
    deck_id UUID PRIMARY KEY REFERENCES decks(id) ON DELETE CASCADE,
    run_id UUID NOT NULL REFERENCES presentation_runs(id) ON DELETE CASCADE,
    slide_key TEXT NOT NULL,
    slide_number INTEGER NOT NULL,
    click_step INTEGER NOT NULL DEFAULT 0,
    sequence BIGINT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE participant_sessions (
    id UUID PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    deck_id UUID NOT NULL REFERENCES decks(id) ON DELETE CASCADE,
    display_name TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX participant_sessions_deck_idx ON participant_sessions(deck_id);

CREATE TABLE response_events (
    id UUID PRIMARY KEY,
    deck_id UUID NOT NULL REFERENCES decks(id) ON DELETE CASCADE,
    interaction_key TEXT NOT NULL,
    participant_id UUID NOT NULL REFERENCES participant_sessions(id) ON DELETE CASCADE,
    result_epoch INTEGER NOT NULL,
    idempotency_key TEXT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(participant_id, idempotency_key)
);

CREATE INDEX response_events_aggregate_idx
    ON response_events(deck_id, interaction_key, result_epoch, created_at);

CREATE TABLE current_responses (
    deck_id UUID NOT NULL REFERENCES decks(id) ON DELETE CASCADE,
    interaction_key TEXT NOT NULL,
    participant_id UUID NOT NULL REFERENCES participant_sessions(id) ON DELETE CASCADE,
    result_epoch INTEGER NOT NULL,
    payload JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(deck_id, interaction_key, participant_id, result_epoch)
);

CREATE TABLE questions (
    id UUID PRIMARY KEY,
    deck_id UUID NOT NULL REFERENCES decks(id) ON DELETE CASCADE,
    participant_id UUID NOT NULL REFERENCES participant_sessions(id) ON DELETE CASCADE,
    result_epoch INTEGER NOT NULL,
    body TEXT NOT NULL,
    moderation_status TEXT NOT NULL DEFAULT 'pending'
        CHECK (moderation_status IN ('pending', 'approved', 'rejected')),
    lifecycle_status TEXT NOT NULL DEFAULT 'open'
        CHECK (lifecycle_status IN ('open', 'answered', 'archived')),
    is_pinned BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX questions_deck_epoch_idx
    ON questions(deck_id, result_epoch, moderation_status, created_at DESC);

CREATE TABLE question_votes (
    question_id UUID NOT NULL REFERENCES questions(id) ON DELETE CASCADE,
    participant_id UUID NOT NULL REFERENCES participant_sessions(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(question_id, participant_id)
);

CREATE TABLE audit_events (
    id UUID PRIMARY KEY,
    deck_id UUID REFERENCES decks(id) ON DELETE CASCADE,
    actor_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    action TEXT NOT NULL,
    data JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
