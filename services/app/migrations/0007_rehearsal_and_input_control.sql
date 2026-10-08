ALTER TABLE presentation_runs
    ADD COLUMN run_mode TEXT NOT NULL DEFAULT 'live'
        CHECK (run_mode IN ('live', 'rehearsal')),
    ADD COLUMN input_frozen BOOLEAN NOT NULL DEFAULT false;

CREATE INDEX presentation_runs_mode_idx
    ON presentation_runs(deck_id, run_mode, started_at DESC);
