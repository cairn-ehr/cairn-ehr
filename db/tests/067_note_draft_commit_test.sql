\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/067 — the quarantine policy and the overdue-commit door (sign-in slice 3b-2a, Task 1).
--
-- WHY: a draft nobody has typed into for the site's quarantine time goes into the record by
-- itself, unsigned. "Overdue" is decided on the database's own clock, by ONE door, under the
-- draft's lock, in the same statement that consumes it, so a draft can never go in twice and
-- never early. These cases pin the policy (default, bounds, console-only), the door (not yet
-- overdue, overdue, all-blank, spent, a shortened policy, the chart filter) and the `committed`
-- answer the save and take doors now give for a timer's commit. Nothing here commits (ROLLBACK).
--
-- A draft is made overdue by setting its header's times back as the owner, never by sleeping.

BEGIN;

-- Start from "no policy row": the default is what a fresh site and a restored one both get.
DELETE FROM note_policy;

-- Fixtures: charts P and Q with one event each, and two clinical vault members (A, B), as
-- db/tests/066 writes them (the `sealed` blob is a placeholder nobody releases).
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
                       node_origin, signed_bytes, content_address, body, contributors,
                       signer_key_id, plaintext_twin)
SELECT v.id, v.pid, 'note.added', '1', v.w, 1, 'n1', v.b,
       '\x1220'::bytea || digest(v.b, 'sha256'), '{}'::jsonb, '[]'::jsonb, 'deadbeef', 'mirror fixture'
  FROM (VALUES ('aaaaaaaa-0067-7000-8000-000000000001'::uuid, 'bbbbbbbb-0067-7000-8000-000000000001'::uuid, 1, '\x6701'::bytea),
               ('aaaaaaaa-0067-7000-8000-000000000002'::uuid, 'bbbbbbbb-0067-7000-8000-000000000002'::uuid, 2, '\x6702'::bytea)) v(id, pid, w, b);

INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, sealed,
                          contact_email, enrolled_by)
VALUES ('nurse-a',  'Nurse A', 'nurse',     '\x01', repeat('a1', 32), '\x00', 'a@x', 'console'),
       ('doctor-b', 'Dr B',    'clinician', '\x02', repeat('b2', 32), '\x00', 'b@x', 'console');

-- A clinical.note.asserted event e on chart pid, inserted in THIS transaction (so db/065's
-- deferred trigger accepts the spend that names it). Replica role, because the projection needs a
-- full body this fixture does not have; back to origin so the spend's own trigger still fires.
CREATE FUNCTION pg_temp.note(e uuid, pid uuid, w int) RETURNS void LANGUAGE plpgsql AS $$
DECLARE b bytea := convert_to(e::text, 'UTF8');
BEGIN
    PERFORM set_config('session_replication_role', 'replica', true);
    INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
                           node_origin, signed_bytes, content_address, body, contributors,
                           signer_key_id, plaintext_twin)
    VALUES (e, pid, 'clinical.note.asserted', '1', w, 1, 'n1', b,
            '\x1220'::bytea || digest(b, 'sha256'), '{}'::jsonb, '[]'::jsonb, 'deadbeef', 'mirror fixture');
    PERFORM set_config('session_replication_role', 'origin', true);
END $$;

-- Pretend nobody has typed into draft d for `mins` minutes: the header and its parts are set back.
CREATE FUNCTION pg_temp.idle(d uuid, mins int) RETURNS void LANGUAGE sql AS $$
    UPDATE note_draft SET recording_started = clock_timestamp() - make_interval(mins => mins + 5),
                          last_keystroke = clock_timestamp() - make_interval(mins => mins)
     WHERE draft_id = d;
    UPDATE note_draft_part SET recording_started = clock_timestamp() - make_interval(mins => mins + 5),
                               last_keystroke = clock_timestamp() - make_interval(mins => mins)
     WHERE draft_id = d;
$$;

-- default_quarantine_is_180, set_quarantine_bounds
DO $$
BEGIN
    IF cairn_note_quarantine_minutes() <> 180 THEN
        RAISE EXCEPTION 'default_quarantine_is_180: no row gave %', cairn_note_quarantine_minutes();
    END IF;
    PERFORM cairn_note_set_quarantine(60);
    IF cairn_note_quarantine_minutes() <> 60 THEN
        RAISE EXCEPTION 'default_quarantine_is_180: 60 read back as %', cairn_note_quarantine_minutes();
    END IF;
    PERFORM cairn_note_set_quarantine(NULL);
    IF cairn_note_quarantine_minutes() <> 180 OR (SELECT quarantine_minutes FROM note_policy) IS NOT NULL THEN
        RAISE EXCEPTION 'default_quarantine_is_180: NULL must store NULL and read 180';
    END IF;
    BEGIN
        PERFORM cairn_note_set_quarantine(0);
        RAISE EXCEPTION 'set_quarantine_bounds: 0 was stored' USING ERRCODE = 'P0002';
    EXCEPTION WHEN raise_exception THEN NULL;
    END;
    BEGIN
        PERFORM cairn_note_set_quarantine(43201);
        RAISE EXCEPTION 'set_quarantine_bounds: 43201 was stored' USING ERRCODE = 'P0002';
    EXCEPTION WHEN raise_exception THEN NULL;
    END;
    PERFORM cairn_note_set_quarantine(1);
    IF cairn_note_quarantine_minutes() <> 1 THEN RAISE EXCEPTION 'set_quarantine_bounds: 1 not stored'; END IF;
    PERFORM cairn_note_set_quarantine(43200);
    IF cairn_note_quarantine_minutes() <> 43200 THEN RAISE EXCEPTION 'set_quarantine_bounds: 43200 not stored'; END IF;
    PERFORM cairn_note_set_quarantine(NULL);
END $$;

DO $$
DECLARE
    p  uuid := 'bbbbbbbb-0067-7000-8000-000000000001';
    q  uuid := 'bbbbbbbb-0067-7000-8000-000000000002';
    d  uuid;
    w  text;
    n  int;
BEGIN
    -- not_overdue_commits_nothing: 179 minutes idle under the default 180.
    d := 'dddddddd-0067-7000-8000-000000000001';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'obs stable', NULL);
    PERFORM pg_temp.idle(d, 179);
    IF EXISTS (SELECT 1 FROM cairn_note_draft_commit_overdue(d, gen_random_uuid())) THEN
        RAISE EXCEPTION 'not_overdue_commits_nothing: returned parts';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM note_draft_part WHERE draft_id = d AND narrative = 'obs stable')
       OR NOT EXISTS (SELECT 1 FROM note_draft_revision WHERE draft_id = d)
       OR EXISTS (SELECT 1 FROM note_draft_spent WHERE draft_id = d) THEN
        RAISE EXCEPTION 'not_overdue_commits_nothing: the draft changed';
    END IF;
    IF EXISTS (SELECT 1 FROM cairn_note_drafts_overdue(NULL) o WHERE o.draft_id = d) THEN
        RAISE EXCEPTION 'not_overdue_commits_nothing: listed as overdue';
    END IF;

    -- overdue_returns_parts_and_spends_by_timer: a two-writer draft, 181 minutes idle.
    d := 'dddddddd-0067-7000-8000-000000000002';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'obs', NULL);
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'doctor-b', 'plan', NULL);
    PERFORM pg_temp.idle(d, 181);
    IF NOT EXISTS (SELECT 1 FROM cairn_note_drafts_overdue(p) o WHERE o.draft_id = d) THEN
        RAISE EXCEPTION 'overdue_returns_parts_and_spends_by_timer: not listed as overdue';
    END IF;
    PERFORM pg_temp.note('aaaaaaaa-0067-7000-8000-0000000000a1', p, 11);
    SELECT string_agg(c.seq || ':' || c.writer || ':' || c.narrative, ',' ORDER BY c.seq), count(*) INTO w, n
      FROM cairn_note_draft_commit_overdue(d, 'aaaaaaaa-0067-7000-8000-0000000000a1') c
     WHERE c.patient_id = p;
    IF w IS DISTINCT FROM '1:nurse-a:obs,2:doctor-b:plan' OR n <> 2 THEN
        RAISE EXCEPTION 'overdue_returns_parts_and_spends_by_timer: %', w;
    END IF;
    IF EXISTS (SELECT 1 FROM note_draft WHERE draft_id = d) OR EXISTS (SELECT 1 FROM note_draft_part WHERE draft_id = d)
       OR EXISTS (SELECT 1 FROM note_draft_revision WHERE draft_id = d) THEN
        RAISE EXCEPTION 'overdue_returns_parts_and_spends_by_timer: the header, a part or a revision is left';
    END IF;
    SELECT count(*) INTO n FROM note_draft_spent
     WHERE draft_id = d AND patient_id = p AND event_id = 'aaaaaaaa-0067-7000-8000-0000000000a1' AND by_timer;
    IF n <> 1 THEN RAISE EXCEPTION 'overdue_returns_parts_and_spends_by_timer: % by-timer spend rows', n; END IF;

    -- a_spent_draft_commits_nothing (the timer's own spend, and a Sign's below)
    IF EXISTS (SELECT 1 FROM cairn_note_draft_commit_overdue(d, gen_random_uuid())) THEN
        RAISE EXCEPTION 'a_spent_draft_commits_nothing: a second commit returned parts';
    END IF;
    IF EXISTS (SELECT 1 FROM cairn_note_draft_commit_overdue(gen_random_uuid(), gen_random_uuid())) THEN
        RAISE EXCEPTION 'an absent draft commits nothing';
    END IF;

    -- save_after_a_timer_commit_answers_committed, take_after_a_timer_commit_answers_committed
    w := cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'one more line', NULL);
    IF w IS DISTINCT FROM 'committed' OR EXISTS (SELECT 1 FROM note_draft WHERE draft_id = d) THEN
        RAISE EXCEPTION 'save_after_a_timer_commit_answers_committed: % (or wrote)', w;
    END IF;
    w := cairn_note_draft_take(d, p, 'term-2');
    IF w IS DISTINCT FROM 'committed' THEN
        RAISE EXCEPTION 'take_after_a_timer_commit_answers_committed: %', w;
    END IF;

    -- save_after_a_sign_still_answers_spent (and a Sign-spent draft commits nothing)
    d := 'dddddddd-0067-7000-8000-000000000003';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'signed text', NULL);
    PERFORM pg_temp.note('aaaaaaaa-0067-7000-8000-0000000000a2', p, 12);
    PERFORM cairn_note_draft_spend_parts(d, 'aaaaaaaa-0067-7000-8000-0000000000a2');
    IF (SELECT by_timer FROM note_draft_spent WHERE draft_id = d) IS DISTINCT FROM false THEN
        RAISE EXCEPTION 'a Sign''s spend must not be by_timer';
    END IF;
    w := cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'again', NULL);
    IF w IS DISTINCT FROM 'spent' THEN RAISE EXCEPTION 'save_after_a_sign_still_answers_spent: %', w; END IF;
    w := cairn_note_draft_take(d, p, 'term-2');
    IF w IS DISTINCT FROM 'spent' THEN RAISE EXCEPTION 'take after a sign still answers spent: %', w; END IF;
    IF EXISTS (SELECT 1 FROM cairn_note_draft_commit_overdue(d, gen_random_uuid())) THEN
        RAISE EXCEPTION 'a_spent_draft_commits_nothing: a signed draft returned parts';
    END IF;

    -- an_all_blank_overdue_draft_is_deleted_without_a_spend (NBSP and IDEOGRAPHIC SPACE are blank)
    d := 'dddddddd-0067-7000-8000-000000000004';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'typed', NULL);
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', U&'\00A0\3000 ', NULL);
    PERFORM pg_temp.idle(d, 181);
    IF EXISTS (SELECT 1 FROM cairn_note_draft_commit_overdue(d, gen_random_uuid())) THEN
        RAISE EXCEPTION 'an_all_blank_overdue_draft_is_deleted_without_a_spend: returned parts';
    END IF;
    IF EXISTS (SELECT 1 FROM note_draft WHERE draft_id = d) OR EXISTS (SELECT 1 FROM note_draft_part WHERE draft_id = d)
       OR EXISTS (SELECT 1 FROM note_draft_revision WHERE draft_id = d)
       OR EXISTS (SELECT 1 FROM note_draft_spent WHERE draft_id = d) THEN
        RAISE EXCEPTION 'an_all_blank_overdue_draft_is_deleted_without_a_spend: something is left';
    END IF;

    -- a_shorter_policy_applies_to_waiting_drafts (Review Focus 3): 90 minutes idle is not overdue
    -- at 180; after the site sets 60 it is listed and committed — nothing is grandfathered.
    d := 'dddddddd-0067-7000-8000-000000000005';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'doctor-b', 'waiting', NULL);
    PERFORM pg_temp.idle(d, 90);
    IF EXISTS (SELECT 1 FROM cairn_note_drafts_overdue(NULL) o WHERE o.draft_id = d)
       OR EXISTS (SELECT 1 FROM cairn_note_draft_commit_overdue(d, gen_random_uuid())) THEN
        RAISE EXCEPTION 'a_shorter_policy_applies_to_waiting_drafts: overdue at 180';
    END IF;
    PERFORM cairn_note_set_quarantine(60);
    IF NOT EXISTS (SELECT 1 FROM cairn_note_drafts_overdue(NULL) o WHERE o.draft_id = d) THEN
        RAISE EXCEPTION 'a_shorter_policy_applies_to_waiting_drafts: not listed at 60';
    END IF;
    PERFORM pg_temp.note('aaaaaaaa-0067-7000-8000-0000000000a3', p, 13);
    SELECT string_agg(c.narrative, ',') INTO w
      FROM cairn_note_draft_commit_overdue(d, 'aaaaaaaa-0067-7000-8000-0000000000a3') c;
    IF w IS DISTINCT FROM 'waiting' THEN
        RAISE EXCEPTION 'a_shorter_policy_applies_to_waiting_drafts: committed %', w;
    END IF;
    PERFORM cairn_note_set_quarantine(NULL);

    -- overdue_list_filters_by_chart: an overdue draft on each chart; A's is the older.
    PERFORM cairn_note_draft_part_save('dddddddd-0067-7000-8000-000000000006', p, 'term-1', 'nurse-a', 'on P', NULL);
    PERFORM cairn_note_draft_part_save('dddddddd-0067-7000-8000-000000000007', q, 'term-1', 'nurse-a', 'on Q', NULL);
    PERFORM pg_temp.idle('dddddddd-0067-7000-8000-000000000006', 300);
    PERFORM pg_temp.idle('dddddddd-0067-7000-8000-000000000007', 200);
    SELECT string_agg(o.draft_id::text, ',' ORDER BY o.ord) INTO w
      FROM (SELECT draft_id, row_number() OVER () AS ord FROM cairn_note_drafts_overdue(p)) o;
    IF w IS DISTINCT FROM 'dddddddd-0067-7000-8000-000000000006' THEN
        RAISE EXCEPTION 'overdue_list_filters_by_chart: chart P listed %', w;
    END IF;
    -- NULL = every chart, oldest last keystroke first (draft 1 of chart P is not overdue).
    SELECT string_agg(o.draft_id::text, ',' ORDER BY o.ord) INTO w
      FROM (SELECT draft_id, row_number() OVER () AS ord FROM cairn_note_drafts_overdue(NULL)) o;
    IF w IS DISTINCT FROM 'dddddddd-0067-7000-8000-000000000006,dddddddd-0067-7000-8000-000000000007' THEN
        RAISE EXCEPTION 'overdue_list_filters_by_chart: every chart listed %', w;
    END IF;
END $$;

-- Every spend above names its note, inserted in this transaction: the commit-time check passes.
SET CONSTRAINTS note_draft_spent_names_its_note IMMEDIATE;

-- a_timer_commit_still_needs_its_note: the door's spend is checked like a Sign's, so a commit
-- naming no note of this chart is refused at COMMIT (here: when the constraint is made immediate).
DO $$
DECLARE refused boolean := false;
BEGIN
    BEGIN
        PERFORM cairn_note_draft_part_save('dddddddd-0067-7000-8000-000000000008', 'bbbbbbbb-0067-7000-8000-000000000001',
                                           'term-1', 'nurse-a', 'x', NULL);
        PERFORM pg_temp.idle('dddddddd-0067-7000-8000-000000000008', 181);
        PERFORM cairn_note_draft_commit_overdue('dddddddd-0067-7000-8000-000000000008', gen_random_uuid());
        SET CONSTRAINTS note_draft_spent_names_its_note IMMEDIATE;
    EXCEPTION WHEN raise_exception THEN refused := true;
    END;
    IF NOT refused THEN RAISE EXCEPTION 'a_timer_commit_still_needs_its_note: a commit with no note passed'; END IF;
END $$;

-- NULLs are caller bugs, refused.
DO $$
DECLARE c text;
BEGIN
    FOREACH c IN ARRAY ARRAY[
        'SELECT * FROM cairn_note_draft_commit_overdue(NULL, gen_random_uuid())',
        'SELECT * FROM cairn_note_draft_commit_overdue(''dddddddd-0067-7000-8000-000000000006'', NULL)'] LOOP
        BEGIN
            EXECUTE c;
            RAISE EXCEPTION 'a NULL was accepted: %', c USING ERRCODE = 'P0002';
        EXCEPTION WHEN raise_exception THEN NULL;
        END;
    END LOOP;
END $$;

-- One definition of blank: the commit door asks cairn_twin_is_present, never a regex class
-- (ADR-0088 decision 10; db/tests/066 pins the other two doors the same way).
DO $$
DECLARE f text := pg_get_functiondef('cairn_note_draft_commit_overdue(uuid, uuid)'::regprocedure);
BEGIN
    IF f NOT LIKE '%cairn_twin_is_present(%' OR f LIKE '%[:space:]%' THEN
        RAISE EXCEPTION 'cairn_note_draft_commit_overdue must test blankness with cairn_twin_is_present only';
    END IF;
END $$;

-- a_take_does_not_restart_the_clock (ADR-0089 decision 1): carrying a draft to another terminal
-- is not typing in it, so an overdue draft taken stays overdue and still goes in.
DO $$
DECLARE
    p uuid := 'bbbbbbbb-0067-7000-8000-000000000001';
    d uuid := 'dddddddd-0067-7000-8000-0000000000a1';
    w text;
BEGIN
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'obs stable', NULL);
    PERFORM pg_temp.idle(d, 181);
    w := cairn_note_draft_take(d, p, 'term-2');
    IF w IS DISTINCT FROM 'taken' THEN RAISE EXCEPTION 'a_take_does_not_restart_the_clock: take answered %', w; END IF;
    IF NOT EXISTS (SELECT 1 FROM cairn_note_drafts_overdue(p) o WHERE o.draft_id = d) THEN
        RAISE EXCEPTION 'a_take_does_not_restart_the_clock: the taken draft is no longer overdue';
    END IF;
    PERFORM pg_temp.note('eeeeeeee-0067-7000-8000-0000000000a1', p, 67);
    IF NOT EXISTS (SELECT 1 FROM cairn_note_draft_commit_overdue(d, 'eeeeeeee-0067-7000-8000-0000000000a1')) THEN
        RAISE EXCEPTION 'a_take_does_not_restart_the_clock: the taken draft did not go in';
    END IF;
END $$;

-- a_rotated_human_key_is_human: the commit names a writer's CURRENT key, which after a re-key is
-- on a rotate-key row; it is human only because cairn_actor_append copies the enrol row's kind
-- (db/004). If that copy regressed, a re-keyed nurse's unsigned note would grade "writer not
-- known" under her own name. A device key is never human.
DO $$
DECLARE a bytea;
BEGIN
    a := enroll_actor('human', '{"role":"clinician","handle":"k067-rot"}', 'k067-human-1');
    PERFORM cairn_actor_append(a, 'rotate-key', 'k067-human-2', NULL, NULL);
    PERFORM enroll_actor('device', '{"role":"registration-desk","node_key":"k067"}', 'k067-device');
    IF NOT cairn_note_key_is_human('k067-human-1') OR NOT cairn_note_key_is_human('k067-human-2') THEN
        RAISE EXCEPTION 'a_rotated_human_key_is_human: a human key (enrolled or rotated) is not human';
    END IF;
    IF cairn_note_key_is_human('k067-device') OR cairn_note_key_is_human('k067-nobody') THEN
        RAISE EXCEPTION 'a_rotated_human_key_is_human: a device or unknown key is human';
    END IF;
END $$;

-- the_console_door_has_no_window_grant; the other five are the window's; all six definer-run
-- with a pinned search_path; the policy table is nobody's.
DO $$
DECLARE f text;
BEGIN
    IF has_function_privilege('cairn_window', 'cairn_note_set_quarantine(int)', 'EXECUTE')
       OR has_function_privilege('cairn_agent', 'cairn_note_set_quarantine(int)', 'EXECUTE')
       OR has_function_privilege('cairn_node', 'cairn_note_set_quarantine(int)', 'EXECUTE') THEN
        RAISE EXCEPTION 'the_console_door_has_no_window_grant: a runtime role may set the quarantine';
    END IF;
    IF has_table_privilege('cairn_window', 'note_policy', 'SELECT')
       OR has_table_privilege('cairn_window', 'note_policy', 'UPDATE')
       OR has_table_privilege('cairn_agent', 'note_policy', 'SELECT')
       OR has_table_privilege('cairn_node', 'note_policy', 'SELECT') THEN
        RAISE EXCEPTION 'the_console_door_has_no_window_grant: a runtime role can reach note_policy';
    END IF;
    FOREACH f IN ARRAY ARRAY['cairn_note_quarantine_minutes()', 'cairn_note_drafts_overdue(uuid)',
                             'cairn_note_draft_commit_overdue(uuid, uuid)', 'cairn_note_set_quarantine(int)',
                             'cairn_note_key_is_human(text)', 'cairn_note_drafts_due(uuid)'] LOOP
        IF has_function_privilege('public', f, 'EXECUTE') THEN RAISE EXCEPTION '% must not be PUBLIC', f; END IF;
        IF f <> 'cairn_note_set_quarantine(int)' AND NOT has_function_privilege('cairn_window', f, 'EXECUTE') THEN
            RAISE EXCEPTION '% is the window''s', f;
        END IF;
        IF NOT (SELECT prosecdef AND proconfig @> ARRAY['search_path=public, pg_temp'] FROM pg_proc
                 WHERE oid = f::regprocedure) THEN
            RAISE EXCEPTION '% must be definer-run with a pinned search_path', f;
        END IF;
    END LOOP;
END $$;

ROLLBACK;
