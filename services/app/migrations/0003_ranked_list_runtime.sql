CREATE TABLE interaction_live_states (
    run_id UUID NOT NULL REFERENCES presentation_runs(id) ON DELETE CASCADE,
    interaction_key TEXT NOT NULL,
    phase TEXT NOT NULL DEFAULT 'revealing'
        CHECK (phase IN ('revealing', 'voting', 'ranked')),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, interaction_key)
);

CREATE INDEX interaction_live_states_run_idx
    ON interaction_live_states(run_id);
