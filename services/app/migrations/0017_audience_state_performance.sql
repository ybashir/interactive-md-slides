CREATE INDEX participant_sessions_deck_last_seen_idx
    ON participant_sessions(deck_id, last_seen_at DESC);
