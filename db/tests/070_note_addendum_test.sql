\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/070 — "Add to this note": the addendum draft's doors and db/064's rule 6a (sign-in slice 3c-1).
--
-- WHY: an addendum draft remembers the signed note it adds to (note_draft.addendum_to, db/065),
-- set once when its header is made and never changed. These cases pin the door's answer words
-- (no-such-note, elsewhere, the spent words, a blank first save that makes nothing), that a
-- refusal writes nothing, that db/066's plain save keeps the target, and the floor's rule 6a:
-- at the authoring door an addendum-to target is a clinical.note.asserted of the event's own
-- chart; at the remote door only its existence is checked (#948). The Rust suite
-- (crates/cairn-node/tests/note_addendum_doors.rs) drives the same through signed events.
-- Nothing here commits (ROLLBACK).

\o /dev/null
BEGIN;

-- Fixtures: charts P and Q. On P: a non-note event, and a note N (event_log + its projection
-- row, inserted directly in replica role since the fixture body is not a real one). On Q: a note
-- M. One clinical member and one admin, written as db/tests/066 writes them.
SET LOCAL session_replication_role = replica;
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
                       node_origin, signed_bytes, content_address, body, contributors,
                       signer_key_id, plaintext_twin)
SELECT v.id, v.pid, v.t, '1', v.w, 1, 'n1', v.b,
       '\x1220'::bytea || digest(v.b, 'sha256'), '{}'::jsonb, '[]'::jsonb, 'deadbeef', 'mirror fixture'
  FROM (VALUES ('aaaaaaaa-0070-7000-8000-000000000001'::uuid, 'bbbbbbbb-0070-7000-8000-000000000001'::uuid,
                'note.added', 1, '\x7001'::bytea),
               ('aaaaaaaa-0070-7000-8000-0000000000a1'::uuid, 'bbbbbbbb-0070-7000-8000-000000000001'::uuid,
                'clinical.note.asserted', 2, '\x70a1'::bytea),
               ('aaaaaaaa-0070-7000-8000-0000000000b1'::uuid, 'bbbbbbbb-0070-7000-8000-000000000002'::uuid,
                'clinical.note.asserted', 3, '\x70b1'::bytea)) v(id, pid, t, w, b);
SET LOCAL session_replication_role = origin;
INSERT INTO clinical_note (event_id, patient_id, narrative, refs, t_effective, effective_basis,
                           recording_started, recording_ended, content_address)
SELECT e.event_id, e.patient_id, 'mirror note', '[]', '2026-11-05 01:00+00', 'recording-started',
       '2026-11-05 01:00+00', '2026-11-05 01:05+00', e.content_address
  FROM event_log e WHERE e.event_type = 'clinical.note.asserted' AND e.event_id::text LIKE 'aaaaaaaa-0070-%';

INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, sealed,
                          contact_email, enrolled_by)
VALUES ('nurse-a', 'Nurse A', 'nurse', '\x01', repeat('a1', 32), '\x00', 'a@x', 'console'),
       ('admin-x', 'Admin X', 'admin', '\x04', repeat('d4', 32), '\x00', 'x@x', 'console');

-- The draft's state as one string, to show a refusal wrote nothing: header, parts, revisions.
CREATE FUNCTION pg_temp.state(d uuid) RETURNS text LANGUAGE sql AS $$
    SELECT (SELECT count(*) FROM note_draft WHERE draft_id = d) || '/'
        || (SELECT count(*) FROM note_draft_part WHERE draft_id = d) || '/'
        || (SELECT count(*) FROM note_draft_revision WHERE draft_id = d);
$$;

DO $$
DECLARE
    p   uuid := 'bbbbbbbb-0070-7000-8000-000000000001';
    n   uuid := 'aaaaaaaa-0070-7000-8000-0000000000a1';  -- P's note
    m   uuid := 'aaaaaaaa-0070-7000-8000-0000000000b1';  -- Q's note
    x   uuid := 'aaaaaaaa-0070-7000-8000-000000000001';  -- P's non-note
    d   uuid;
    w   text;
    h   note_draft%ROWTYPE;
BEGIN
    -- an_addendum_draft_names_its_note: the header carries the target, and db/066's door wrote
    -- part 1 and its revision (and moved the header's last_keystroke to its own instant).
    d := 'dddddddd-0070-7000-8000-000000000001';
    w := cairn_note_draft_addendum_part_save(d, p, 'term-1', 'nurse-a', 'plan: review', NULL, n);
    SELECT * INTO STRICT h FROM note_draft WHERE draft_id = d;
    IF w <> 'saved' OR h.addendum_to IS DISTINCT FROM n OR h.recording_started > h.last_keystroke
       OR cairn_note_draft_addendum_to(d) IS DISTINCT FROM n OR pg_temp.state(d) <> '1/1/1' THEN
        RAISE EXCEPTION 'an_addendum_draft_names_its_note: % % %', w, h, pg_temp.state(d);
    END IF;
    -- a_later_save_of_the_same_target_appends_through_db_066: a colleague's part 2.
    w := cairn_note_draft_addendum_part_save(d, p, 'term-1', NULL, 'seen', NULL, n);
    IF w <> 'saved' OR pg_temp.state(d) <> '1/2/2' THEN
        RAISE EXCEPTION 'a_later_save_appends: % %', w, pg_temp.state(d);
    END IF;
    -- a_later_plain_save_keeps_the_target: db/066's door never touches addendum_to.
    w := cairn_note_draft_part_save(d, p, 'term-1', NULL, 'seen, more', NULL);
    IF w <> 'saved' OR cairn_note_draft_addendum_to(d) IS DISTINCT FROM n THEN
        RAISE EXCEPTION 'a_later_plain_save_keeps_the_target: % %', w, cairn_note_draft_addendum_to(d);
    END IF;
    -- another_target_is_elsewhere: one draft adds to one note.
    w := cairn_note_draft_addendum_part_save(d, p, 'term-1', 'nurse-a', 'x', NULL, m);
    IF w <> 'elsewhere' OR pg_temp.state(d) <> '1/2/3' THEN
        RAISE EXCEPTION 'another_target_is_elsewhere: % %', w, pg_temp.state(d);
    END IF;
    -- the_part_rules_are_db_066s: another terminal is 'moved', an admin is 'no-such-writer'.
    IF cairn_note_draft_addendum_part_save(d, p, 'term-2', 'nurse-a', 'x', NULL, n) <> 'moved'
       OR cairn_note_draft_addendum_part_save(d, p, 'term-1', 'admin-x', 'x', NULL, n) <> 'no-such-writer'
       OR pg_temp.state(d) <> '1/2/3' THEN
        RAISE EXCEPTION 'the_part_rules_are_db_066s: %', pg_temp.state(d);
    END IF;

    -- a_blank_first_save_makes_no_header: NBSP and a newline are blank by the floor's own test.
    d := 'dddddddd-0070-7000-8000-000000000002';
    w := cairn_note_draft_addendum_part_save(d, p, 'term-1', 'nurse-a', E' \u00a0\n', NULL, n);
    IF w <> 'saved' OR pg_temp.state(d) <> '0/0/0' OR cairn_note_draft_addendum_to(d) IS NOT NULL THEN
        RAISE EXCEPTION 'a_blank_first_save_makes_no_header: % %', w, pg_temp.state(d);
    END IF;

    -- no_such_note: another chart's note, a non-note, an unknown id. Nothing is written.
    d := 'dddddddd-0070-7000-8000-000000000003';
    IF cairn_note_draft_addendum_part_save(d, p, 'term-1', 'nurse-a', 'x', NULL, m) <> 'no-such-note'
       OR cairn_note_draft_addendum_part_save(d, p, 'term-1', 'nurse-a', 'x', NULL, x) <> 'no-such-note'
       OR cairn_note_draft_addendum_part_save(d, p, 'term-1', 'nurse-a', 'x', NULL, gen_random_uuid()) <> 'no-such-note'
       OR pg_temp.state(d) <> '0/0/0' THEN
        RAISE EXCEPTION 'no_such_note: %', pg_temp.state(d);
    END IF;

    -- a_refused_writer_leaves_no_header: the header this call made goes with the refusal.
    d := 'dddddddd-0070-7000-8000-000000000004';
    w := cairn_note_draft_addendum_part_save(d, p, 'term-1', 'admin-x', 'x', NULL, n);
    IF w <> 'no-such-writer' OR pg_temp.state(d) <> '0/0/0' THEN
        RAISE EXCEPTION 'a_refused_writer_leaves_no_header: % %', w, pg_temp.state(d);
    END IF;

    -- a_new_notes_draft_refuses_a_target_later: a header made by db/066 says "a new note".
    d := 'dddddddd-0070-7000-8000-000000000005';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'a new note', NULL);
    w := cairn_note_draft_addendum_part_save(d, p, 'term-1', 'nurse-a', 'a new note, more', NULL, n);
    IF w <> 'elsewhere' OR pg_temp.state(d) <> '1/1/1' OR cairn_note_draft_addendum_to(d) IS NOT NULL THEN
        RAISE EXCEPTION 'a_new_notes_draft_refuses_a_target_later: % %', w, pg_temp.state(d);
    END IF;

    -- spent_words: db/066's three, before any header or target check (even a bad target).
    INSERT INTO note_draft_spent (draft_id, patient_id, event_id, by_timer, discarded_by) VALUES
        ('dddddddd-0070-7000-8000-000000000011', p, gen_random_uuid(), false, NULL),
        ('dddddddd-0070-7000-8000-000000000012', p, gen_random_uuid(), true, NULL),
        ('dddddddd-0070-7000-8000-000000000013', p, gen_random_uuid(), false, repeat('a1', 32));
    SELECT string_agg(cairn_note_draft_addendum_part_save(s.id, p, 'term-1', 'nurse-a', 'x', NULL, m), ',' ORDER BY s.id)
      INTO w
      FROM unnest(ARRAY['dddddddd-0070-7000-8000-000000000011', 'dddddddd-0070-7000-8000-000000000012',
                        'dddddddd-0070-7000-8000-000000000013']::uuid[]) s(id);
    IF w <> 'spent,committed,discarded' THEN
        RAISE EXCEPTION 'spent_words: %', w;
    END IF;

    -- no_such_draft_reads_null
    IF cairn_note_draft_addendum_to(gen_random_uuid()) IS NOT NULL THEN
        RAISE EXCEPTION 'no_such_draft_reads_null';
    END IF;
END $$;

-- a_null_argument_raises: every argument but the writer and the stated time.
DO $$
DECLARE
    p uuid := 'bbbbbbbb-0070-7000-8000-000000000001';
    n uuid := 'aaaaaaaa-0070-7000-8000-0000000000a1';
    d uuid := 'dddddddd-0070-7000-8000-000000000021';
    k int;
BEGIN
    FOR k IN 1..5 LOOP
        BEGIN
            PERFORM cairn_note_draft_addendum_part_save(
                CASE WHEN k = 1 THEN NULL ELSE d END, CASE WHEN k = 2 THEN NULL ELSE p END,
                CASE WHEN k = 3 THEN NULL ELSE 'term-1' END, 'nurse-a',
                CASE WHEN k = 4 THEN NULL ELSE 'x' END, NULL, CASE WHEN k = 5 THEN NULL ELSE n END);
            RAISE EXCEPTION 'a_null_argument_raises: argument % was accepted', k;
        EXCEPTION WHEN raise_exception THEN
            IF SQLERRM NOT LIKE 'cairn_note_draft_addendum_part_save needs %' THEN
                RAISE;
            END IF;
        END;
    END LOOP;
END $$;

-- Rule 6a, called directly as db/tests/064 calls the floor. A body for chart P adding to `t`.
CREATE FUNCTION pg_temp.addendum(pid text, t text) RETURNS jsonb LANGUAGE sql AS $$
    SELECT jsonb_build_object(
        'patient_id',     pid,
        'schema_version', 'clinical.note/1',
        't_effective',    '2026-11-05T02:00:00Z',
        'plaintext_twin', 'seen',
        'payload', jsonb_build_object(
            'narrative', 'seen', 'media', '[]'::jsonb, 'encounter', NULL,
            'refs', jsonb_build_array(jsonb_build_object('rel', 'addendum-to', 'event', t)),
            'recording_started', '2026-11-05T02:00:00Z', 'recording_ended', '2026-11-05T02:01:00Z',
            'effective_basis', 'recording-started'));
$$;
-- The outcome of the floor on b: 'ok', or the refusal's SQLSTATE and message.
CREATE FUNCTION pg_temp.floor(b jsonb) RETURNS text LANGUAGE plpgsql AS $$
BEGIN
    PERFORM cairn_check_note_assertion('clinical.note.asserted', b);
    RETURN 'ok';
EXCEPTION WHEN others THEN
    RETURN SQLSTATE || ' ' || SQLERRM;
END $$;

DO $$
DECLARE
    p text := 'bbbbbbbb-0070-7000-8000-000000000001';
    w text;
BEGIN
    -- the_authoring_door_admits_this_charts_note (the passing control), refuses the rest.
    w := pg_temp.floor(pg_temp.addendum(p, 'aaaaaaaa-0070-7000-8000-0000000000a1'));
    IF w <> 'ok' THEN RAISE EXCEPTION 'this_charts_note: %', w; END IF;
    w := pg_temp.floor(pg_temp.addendum(p, 'aaaaaaaa-0070-7000-8000-0000000000b1'));
    IF w <> 'P0001 note assertion: an addendum-to target must be a note on this chart, got aaaaaaaa-0070-7000-8000-0000000000b1' THEN
        RAISE EXCEPTION 'another_charts_note: %', w;
    END IF;
    w := pg_temp.floor(pg_temp.addendum(p, 'aaaaaaaa-0070-7000-8000-000000000001'));
    IF w <> 'P0001 note assertion: an addendum-to target must be a note on this chart, got aaaaaaaa-0070-7000-8000-000000000001' THEN
        RAISE EXCEPTION 'a_non_note: %', w;
    END IF;
    w := pg_temp.floor(pg_temp.addendum(p, '01900000-0000-7000-8000-00000000dead'));
    IF w NOT LIKE 'P0001 note assertion: refs event % is not held on this node%' THEN
        RAISE EXCEPTION 'an_unknown_id: %', w;
    END IF;

    -- the_remote_door_checks_existence_only (#948): another chart's note and a non-note are
    -- carried; an unknown id is still the bare P0001 the puller pens.
    PERFORM set_config('cairn.remote_apply', 'on', true);
    w := pg_temp.floor(pg_temp.addendum(p, 'aaaaaaaa-0070-7000-8000-0000000000b1'))
      || ',' || pg_temp.floor(pg_temp.addendum(p, 'aaaaaaaa-0070-7000-8000-000000000001'));
    IF w <> 'ok,ok' THEN RAISE EXCEPTION 'remote_held_targets: %', w; END IF;
    w := pg_temp.floor(pg_temp.addendum(p, '01900000-0000-7000-8000-00000000dead'));
    IF w NOT LIKE 'P0001 note assertion: refs event % is not held on this node%' THEN
        RAISE EXCEPTION 'remote_unknown_id: %', w;
    END IF;
    PERFORM set_config('cairn.remote_apply', '', true);
END $$;

-- the_doors_are_the_windows_only: revoked from PUBLIC, granted to cairn_window.
DO $$
BEGIN
    ASSERT NOT has_function_privilege('public',
        'cairn_note_draft_addendum_part_save(uuid,uuid,text,text,text,timestamptz,uuid)', 'EXECUTE'),
        'PUBLIC must not execute the addendum save door';
    ASSERT NOT has_function_privilege('public', 'cairn_note_draft_addendum_to(uuid)', 'EXECUTE'),
        'PUBLIC must not execute the addendum reader';
    ASSERT has_function_privilege('cairn_window',
        'cairn_note_draft_addendum_part_save(uuid,uuid,text,text,text,timestamptz,uuid)', 'EXECUTE'),
        'cairn_window executes the addendum save door';
    ASSERT has_function_privilege('cairn_window', 'cairn_note_draft_addendum_to(uuid)', 'EXECUTE'),
        'cairn_window executes the addendum reader';
END $$;

ROLLBACK;
