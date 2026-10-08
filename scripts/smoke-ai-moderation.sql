\set ON_ERROR_STOP on
BEGIN;

INSERT INTO users (id, google_sub, email, display_name)
VALUES ('01993100-0000-7000-8000-000000000001', 'ai-smoke-user', 'ai-smoke@example.invalid', 'AI Smoke');

INSERT INTO decks (id, owner_id, title, join_code, markdown)
VALUES (
    '01993100-0000-7000-8000-000000000002',
    '01993100-0000-7000-8000-000000000001',
    'AI smoke deck', 'AISMK01', E'---\ntitle: AI smoke\n---\n# Test'
);

INSERT INTO participant_sessions (id, token_hash, deck_id, display_name)
VALUES (
    '01993100-0000-7000-8000-000000000003', 'ai-smoke-token',
    '01993100-0000-7000-8000-000000000002', NULL
);

INSERT INTO questions (
    id, deck_id, participant_id, result_epoch, body, analysis_status
) VALUES (
    '01993100-0000-7000-8000-000000000004',
    '01993100-0000-7000-8000-000000000002',
    '01993100-0000-7000-8000-000000000003',
    1, 'How will we improve delivery predictability?', 'queued'
);

INSERT INTO background_jobs (id, deck_id, kind, payload)
VALUES (
    '01993100-0000-7000-8000-000000000005',
    '01993100-0000-7000-8000-000000000002',
    'question_moderation',
    '{"question_id":"01993100-0000-7000-8000-000000000004"}'::jsonb
);

INSERT INTO background_jobs (id, deck_id, kind, payload, available_at)
VALUES (
    '01993100-0000-7000-8000-000000000006',
    '01993100-0000-7000-8000-000000000002',
    'deck_insights', '{"result_epoch":1}'::jsonb, now() + interval '8 seconds'
)
ON CONFLICT (deck_id, kind)
    WHERE kind = 'deck_insights' AND status = 'queued'
DO UPDATE SET available_at = EXCLUDED.available_at;

INSERT INTO background_jobs (id, deck_id, kind, payload, available_at)
VALUES (
    '01993100-0000-7000-8000-000000000007',
    '01993100-0000-7000-8000-000000000002',
    'deck_insights', '{"result_epoch":1}'::jsonb, now() + interval '8 seconds'
)
ON CONFLICT (deck_id, kind)
    WHERE kind = 'deck_insights' AND status = 'queued'
DO UPDATE SET available_at = EXCLUDED.available_at;

-- A presenter's decision must survive a later AI completion.
UPDATE questions
SET moderation_status = 'approved', moderation_source = 'human'
WHERE id = '01993100-0000-7000-8000-000000000004';

UPDATE questions SET
    analysis_status = 'complete',
    moderation_labels = '{"spam":false,"safety_flagged":false}'::jsonb,
    moderation_reason = 'Relevant delivery question',
    topic = 'Delivery',
    moderation_status = CASE
        WHEN moderation_status = 'pending' THEN 'rejected' ELSE moderation_status END,
    moderation_source = CASE
        WHEN moderation_status = 'pending' THEN 'ai' ELSE moderation_source END
WHERE id = '01993100-0000-7000-8000-000000000004';

INSERT INTO question_analysis_versions (
    id, question_id, provider, model, prompt_version, suggested_action,
    confidence, labels, reason, topic
) VALUES (
    '01993100-0000-7000-8000-000000000008',
    '01993100-0000-7000-8000-000000000004',
    'gemini', 'fixture', 'question-moderation-v2-gemini', 'approve',
    0.98, '{"spam":false}'::jsonb, 'Relevant delivery question', 'Delivery'
);

INSERT INTO deck_insight_versions (
    id, deck_id, result_epoch, provider, model, prompt_version,
    question_count, summary, themes, suggested_answers
) VALUES (
    '01993100-0000-7000-8000-000000000009',
    '01993100-0000-7000-8000-000000000002', 1,
    'gemini', 'fixture', 'deck-insights-v2-attributed', 1,
    'Delivery predictability is the current theme.',
    '[{"label":"Delivery","summary":"Predictability","question_ids":["01993100-0000-7000-8000-000000000004"]}]'::jsonb,
    '["Clarify the current delivery baseline."]'::jsonb
);

DO $$
DECLARE
    decision_status TEXT;
    decision_source TEXT;
    active_insight_jobs INTEGER;
BEGIN
    SELECT moderation_status, moderation_source
      INTO decision_status, decision_source
    FROM questions WHERE id = '01993100-0000-7000-8000-000000000004';
    IF decision_status <> 'approved' OR decision_source <> 'human' THEN
        RAISE EXCEPTION 'human moderation was overwritten';
    END IF;

    SELECT COUNT(*) INTO active_insight_jobs
    FROM background_jobs
    WHERE deck_id = '01993100-0000-7000-8000-000000000002'
      AND kind = 'deck_insights' AND status = 'queued';
    IF active_insight_jobs <> 1 THEN
        RAISE EXCEPTION 'insight debounce invariant failed';
    END IF;
END $$;

ROLLBACK;
