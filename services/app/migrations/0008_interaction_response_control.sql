ALTER TABLE interaction_live_states
    ADD COLUMN accepting_responses BOOLEAN NOT NULL DEFAULT true;

-- A ranked list that closed before this migration is not accepting answers,
-- so the new column must not claim otherwise.
UPDATE interaction_live_states
SET accepting_responses = false
WHERE phase = 'ranked';

INSERT INTO interaction_live_states (run_id, interaction_key, phase, accepting_responses)
SELECT r.id, i.interaction_key, 'voting', true
FROM presentation_runs r
JOIN interaction_definitions i
  ON i.deck_id = r.deck_id AND i.source_revision = r.source_revision AND i.is_archived = false
WHERE r.status = 'live'
ON CONFLICT (run_id, interaction_key) DO NOTHING;
