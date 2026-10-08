ALTER TABLE decks
ADD COLUMN qa_display_mode TEXT NOT NULL DEFAULT 'verbatim'
CHECK (qa_display_mode IN ('verbatim', 'ai_grouped'));
