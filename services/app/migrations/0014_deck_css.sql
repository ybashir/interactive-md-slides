ALTER TABLE decks
    ADD COLUMN deck_css TEXT NOT NULL DEFAULT '',
    ADD COLUMN last_verified_css TEXT;

UPDATE decks
SET last_verified_css = ''
WHERE last_verified_markdown IS NOT NULL;
