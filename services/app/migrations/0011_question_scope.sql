ALTER TABLE questions
    ADD COLUMN slide_key TEXT,
    ADD COLUMN slide_number INTEGER,
    ADD CONSTRAINT questions_slide_scope_consistent CHECK (
        (slide_key IS NULL AND slide_number IS NULL)
        OR (slide_key IS NOT NULL AND slide_number IS NOT NULL AND slide_number > 0)
    );

CREATE INDEX questions_slide_scope_idx
    ON questions(deck_id, result_epoch, slide_key, created_at DESC)
    WHERE lifecycle_status <> 'archived';
