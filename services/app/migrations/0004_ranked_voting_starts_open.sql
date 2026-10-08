ALTER TABLE interaction_live_states
    ALTER COLUMN phase SET DEFAULT 'voting';

UPDATE interaction_live_states
SET phase = 'voting', updated_at = now()
WHERE phase = 'revealing';
