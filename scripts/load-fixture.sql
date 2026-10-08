BEGIN;

DELETE FROM decks WHERE id = '00000000-0000-4000-8000-000000002001';
DELETE FROM users WHERE id = '00000000-0000-4000-8000-000000001001';

INSERT INTO users (id, google_sub, email, display_name)
VALUES (
  '00000000-0000-4000-8000-000000001001',
  'interdeck-load-test',
  'load-test@localhost',
  'Interdeck Load Test'
);

INSERT INTO decks (id, owner_id, title, join_code, markdown, revision, result_epoch)
VALUES (
  '00000000-0000-4000-8000-000000002001',
  '00000000-0000-4000-8000-000000001001',
  'Audience load fixture',
  'LOD-TST',
  $deck$---
theme: default
---

<!-- interdeck-slide: load-poll -->

:::interact{type="poll" id="load-poll" results="after-vote"}
# Choose an option

- [option-a] Option A
- [option-b] Option B
:::
$deck$,
  1,
  1
);

INSERT INTO deck_revisions (id, deck_id, revision, markdown, created_by)
SELECT
  '00000000-0000-4000-8000-000000003001',
  id,
  revision,
  markdown,
  owner_id
FROM decks
WHERE id = '00000000-0000-4000-8000-000000002001';

INSERT INTO interaction_definitions (
  id,
  deck_id,
  interaction_key,
  slide_key,
  slide_number,
  source_order,
  kind,
  title,
  config,
  options,
  source_revision
)
VALUES (
  '00000000-0000-4000-8000-000000004001',
  '00000000-0000-4000-8000-000000002001',
  'load-poll',
  'load-poll',
  1,
  0,
  'poll',
  'Choose an option',
  '{"results":"after-vote"}',
  '[{"id":"option-a","label":"Option A"},{"id":"option-b","label":"Option B"}]',
  1
);

INSERT INTO result_epochs (deck_id, epoch, created_by, reason)
VALUES (
  '00000000-0000-4000-8000-000000002001',
  1,
  '00000000-0000-4000-8000-000000001001',
  'load-test'
);

INSERT INTO presentation_runs (
  id,
  deck_id,
  source_revision,
  result_epoch,
  status,
  started_by
)
VALUES (
  '00000000-0000-4000-8000-000000005001',
  '00000000-0000-4000-8000-000000002001',
  1,
  1,
  'live',
  '00000000-0000-4000-8000-000000001001'
);

INSERT INTO live_deck_states (
  deck_id,
  run_id,
  slide_key,
  slide_number,
  click_step,
  sequence
)
VALUES (
  '00000000-0000-4000-8000-000000002001',
  '00000000-0000-4000-8000-000000005001',
  'load-poll',
  1,
  0,
  1
);

COMMIT;
