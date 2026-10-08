CREATE INDEX current_responses_deck_epoch_idx
    ON current_responses(deck_id, result_epoch, interaction_key);
