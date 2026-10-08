ALTER TABLE live_deck_states
ADD COLUMN IF NOT EXISTS visible_element_interaction_ids jsonb NOT NULL DEFAULT '[]'::jsonb;
