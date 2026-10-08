ALTER TABLE interaction_live_states
    ADD COLUMN results_revealed BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN timer_ends_at TIMESTAMPTZ,
    ADD COLUMN timer_remaining_seconds INTEGER;

ALTER TABLE interaction_live_states
    ADD CONSTRAINT interaction_timer_remaining_valid
    CHECK (timer_remaining_seconds IS NULL OR timer_remaining_seconds BETWEEN 0 AND 7200);
