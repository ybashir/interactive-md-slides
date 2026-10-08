-- Runtime authoring now uses one mutable deck version. Historical revision
-- rows remain intact for a later, deliberately destructive cleanup migration.
UPDATE decks
SET draft_version = GREATEST(draft_version, revision);

ALTER TABLE decks
    RENAME COLUMN draft_version TO version;

ALTER TABLE decks
    ADD COLUMN verified_version BIGINT,
    ADD COLUMN last_verified_markdown TEXT;

UPDATE decks d
SET last_verified_markdown = r.markdown
FROM deck_revisions r
WHERE r.deck_id = d.id
  AND r.revision = d.revision;

UPDATE decks d
SET verified_version = d.version
WHERE d.markdown = d.last_verified_markdown
  AND EXISTS (
    SELECT 1
    FROM deck_builds b
    WHERE b.deck_id = d.id
      AND b.source_revision = d.revision
      AND b.status = 'succeeded'
  );

ALTER TABLE deck_builds
    DROP CONSTRAINT IF EXISTS deck_builds_deck_id_source_revision_fkey;
