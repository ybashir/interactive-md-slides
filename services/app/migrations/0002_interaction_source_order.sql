ALTER TABLE interaction_definitions
    ADD COLUMN source_order INTEGER NOT NULL DEFAULT 0;

WITH ordered AS (
    SELECT id,
           (row_number() OVER (
               PARTITION BY deck_id, slide_key
               ORDER BY created_at, interaction_key
           ) - 1)::INTEGER AS source_order
    FROM interaction_definitions
)
UPDATE interaction_definitions AS definition
SET source_order = ordered.source_order
FROM ordered
WHERE definition.id = ordered.id;
