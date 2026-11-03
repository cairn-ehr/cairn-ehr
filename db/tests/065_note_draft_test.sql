\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/065 — the progress-note draft store and its five doors (sign-in slice 3a, Task 3).
--
-- WHY: a draft is the one MUTABLE thing next to an append-only record, so the guarantees that
-- keep it honest are asserted here: one draft = one chart + one terminal, a spent draft can
-- never be resurrected, a spend must be backed by a real note event at COMMIT, and the window
-- role reaches the tables only through the doors. Nothing here commits (ROLLBACK at the end);
-- the deferred trigger is forced with SET CONSTRAINTS ... IMMEDIATE instead of a COMMIT.

BEGIN;

-- Fixtures: a chart with one event (note.added: its projection reads nothing from the body,
-- see db/tests/051), and a second chart with none.
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
                       node_origin, signed_bytes, content_address, body, contributors,
                       signer_key_id, plaintext_twin)
VALUES ('aaaaaaaa-0000-7000-8000-000000000001', 'bbbbbbbb-0000-7000-8000-000000000001',
        'note.added', '1', 1, 1, 'n1', '\xcc'::bytea,
        '\x1220'::bytea || digest('\xcc'::bytea, 'sha256'),
        '{}'::jsonb, '[]'::jsonb, 'deadbeef', 'mirror fixture');

-- A real clinical.note.asserted event for the spends below. The fixture insert bypasses the note projection (which needs a full body) via replica role.
SET LOCAL session_replication_role = replica;
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
                       node_origin, signed_bytes, content_address, body, contributors,
                       signer_key_id, plaintext_twin)
VALUES ('aaaaaaaa-0000-7000-8000-000000000002', 'bbbbbbbb-0000-7000-8000-000000000001',
        'clinical.note.asserted', '1', 2, 1, 'n1', '\xdd'::bytea,
        '\x1220'::bytea || digest('\xdd'::bytea, 'sha256'),
        '{}'::jsonb, '[]'::jsonb, 'deadbeef', 'mirror fixture');
SET LOCAL session_replication_role = origin;

DO $$
DECLARE
    d  uuid := 'dddddddd-0000-7000-8000-000000000001';
    p  uuid := 'bbbbbbbb-0000-7000-8000-000000000001';
    r  record;
    t1 timestamptz;
    t2 timestamptz;
BEGIN
    -- Save, and the R1 rule: only a real change moves last_keystroke.
    ASSERT cairn_note_draft_save(d, p, 'term-1', 'first', NULL) = 'saved', 'first save';
    SELECT recording_started, last_keystroke INTO r FROM note_draft WHERE draft_id = d;
    ASSERT r.recording_started = r.last_keystroke, 'insert stamps both times';
    t1 := r.last_keystroke;
    PERFORM pg_sleep(0.02);
    ASSERT cairn_note_draft_save(d, p, 'term-1', 'first', NULL) = 'saved', 'identical re-save answers saved';
    ASSERT (SELECT last_keystroke FROM note_draft WHERE draft_id = d) = t1,
        'an identical re-save is not a keystroke';
    ASSERT cairn_note_draft_save(d, p, 'term-1', 'first, more', NULL) = 'saved', 'changed save';
    SELECT recording_started, last_keystroke INTO r FROM note_draft WHERE draft_id = d;
    ASSERT r.last_keystroke > t1, 'a changed narrative moves last_keystroke';
    ASSERT r.recording_started = t1, 'recording_started never moves';
    t2 := r.last_keystroke;
    PERFORM pg_sleep(0.02);
    ASSERT cairn_note_draft_save(d, p, 'term-1', 'first, more', '2026-01-01 10:00+00') = 'saved', 'stated time save';
    ASSERT (SELECT last_keystroke FROM note_draft WHERE draft_id = d) > t2, 'a changed stated_time moves it too';

    -- Raw text round-trips exactly.
    PERFORM cairn_note_draft_save(d, p, 'term-1', E'*raw* [x]\n  y ', NULL);
    ASSERT (SELECT narrative FROM cairn_note_drafts(p) WHERE draft_id = d) = E'*raw* [x]\n  y ', 'stored as typed';

    -- Elsewhere / no-chart.
    ASSERT cairn_note_draft_save(d, p, 'term-2', 'x', NULL) = 'elsewhere', 'another terminal';
    ASSERT cairn_note_draft_save(d, 'bbbbbbbb-0000-7000-8000-000000000002', 'term-1', 'x', NULL) = 'elsewhere',
        'another patient';
    ASSERT cairn_note_draft_save(gen_random_uuid(), 'bbbbbbbb-0000-7000-8000-000000000002', 'term-1', 'x', NULL)
        = 'no-chart', 'a patient with no event has no chart';
    ASSERT NOT EXISTS (SELECT 1 FROM note_draft WHERE patient_id = 'bbbbbbbb-0000-7000-8000-000000000002'),
        'no-chart writes nothing';

    -- Signer name wraps the key-holder lookup.
    ASSERT cairn_note_signer_name('console') = 'the node''s console', 'console has a display name';
    ASSERT cairn_note_signer_name('no-such-key') IS NULL, 'unknown key -> NULL';

    -- Spend: returns the row, removes the draft, and the id can never be saved again.
    SELECT * INTO r FROM cairn_note_draft_spend(d, 'aaaaaaaa-0000-7000-8000-000000000002');
    ASSERT r.patient_id = p AND r.narrative = E'*raw* [x]\n  y ', 'spend returns the draft';
    ASSERT NOT EXISTS (SELECT 1 FROM note_draft WHERE draft_id = d), 'spent draft is gone';
    ASSERT cairn_note_draft_save(d, p, 'term-1', 'again', NULL) = 'spent', 'save after spend';
    ASSERT NOT EXISTS (SELECT 1 FROM note_draft WHERE draft_id = d), 'spent save writes nothing';
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_note_draft_spend(d, 'aaaaaaaa-0000-7000-8000-000000000002')),
        'a second spend returns zero rows';
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_note_draft_spend(gen_random_uuid(), gen_random_uuid())),
        'spending an absent draft returns zero rows';
END $$;

-- The commit-time check. A spend naming a note.added event (not clinical.note.asserted) is
-- refused when the deferred trigger fires; the sub-block's rollback discards the queued check.
DO $$
DECLARE d uuid := 'dddddddd-0000-7000-8000-000000000003'; refused boolean := false;
BEGIN
    BEGIN
        PERFORM cairn_note_draft_save(d, 'bbbbbbbb-0000-7000-8000-000000000001', 'term-1', 'x', NULL);
        PERFORM cairn_note_draft_spend(d, 'aaaaaaaa-0000-7000-8000-000000000001');
        SET CONSTRAINTS note_draft_spent_names_its_note IMMEDIATE;
    EXCEPTION WHEN raise_exception THEN refused := true;
    END;
    ASSERT refused, 'a spend with no note event must fail at COMMIT';
END $$;

-- The backed spend from the first block passes the same check.
SET CONSTRAINTS note_draft_spent_names_its_note IMMEDIATE;

-- Privileges: tables closed to the window, doors closed to PUBLIC and open to the window.
DO $$
DECLARE f text;
BEGIN
    FOREACH f IN ARRAY ARRAY[
        'cairn_note_draft_save(uuid, uuid, text, text, timestamptz)',
        'cairn_note_drafts(uuid)', 'cairn_note_draft_spend(uuid, uuid)',
        'cairn_note_signer_name(text)'] LOOP
        ASSERT NOT has_function_privilege('public', f, 'EXECUTE'), f || ' must not be PUBLIC';
        ASSERT has_function_privilege('cairn_window', f, 'EXECUTE'), f || ' is the window''s';
    END LOOP;
    ASSERT NOT has_any_column_privilege('cairn_window', 'note_draft', 'SELECT'), 'window cannot read note_draft';
    ASSERT NOT has_any_column_privilege('cairn_window', 'note_draft_spent', 'SELECT'), 'window cannot read note_draft_spent';
    ASSERT NOT has_table_privilege('cairn_window', 'note_draft', 'INSERT'), 'window cannot write note_draft';
END $$;

CREATE ROLE note_draft_probe NOLOGIN;
GRANT cairn_window TO note_draft_probe;
SET LOCAL ROLE note_draft_probe;
DO $$
BEGIN
    PERFORM 1 FROM note_draft;
    RAISE EXCEPTION 'probe read note_draft';
EXCEPTION WHEN insufficient_privilege THEN NULL;
END $$;
DO $$
BEGIN
    PERFORM 1 FROM note_draft_spent;
    RAISE EXCEPTION 'probe read note_draft_spent';
EXCEPTION WHEN insufficient_privilege THEN NULL;
END $$;
RESET ROLE;

-- The new timing kind inserts, and a replay of db/044's constraint pair converges with the row in place.
INSERT INTO ui_gesture_timing (gesture_kind, size_bucket, n) VALUES ('note-sign', '1-3', 1);
-- (db/044 itself opens its own transaction, so its ALTER pair is replayed verbatim here.)
ALTER TABLE ui_gesture_timing DROP CONSTRAINT IF EXISTS ui_gesture_timing_kind_ck;
ALTER TABLE ui_gesture_timing ADD CONSTRAINT ui_gesture_timing_kind_ck
    CHECK (gesture_kind IN ('signoff', 'cease', 'withhold', 'release', 'note-sign'));
DO $$
BEGIN
    ASSERT (SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conname = 'ui_gesture_timing_kind_ck')
        LIKE '%note-sign%', 'the live constraint names note-sign';
END $$;

ROLLBACK;
