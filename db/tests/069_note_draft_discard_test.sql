\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/069 — the audited discard of a draft note (sign-in slice 3b-2b, Task 3; ADR-0089 decision 15).
--
-- WHY: a draft may be thrown away only by someone whose own writing it is (every non-blank part
-- by the discarder's KEY, or by a writer not known), and only with a signed record that it
-- existed. These cases pin the door's rule and outcome words, the `discarded` answer the save and
-- take doors now give, and db/065's commit-time check that a discard's spend names a
-- clinical.note-discard.asserted SIGNED BY THE DISCARDER'S KEY in this same transaction — the one
-- thing a raw-SQL client holding a colleague's key id (but not their private key) cannot make.
-- Nothing here commits (ROLLBACK); the deferred trigger is forced with SET CONSTRAINTS IMMEDIATE.

BEGIN;

-- Fixtures: charts P and Q with one event each; clinical members A and B and an admin, as
-- db/tests/066 writes them (the `sealed` blob is a placeholder nobody releases).
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
                       node_origin, signed_bytes, content_address, body, contributors,
                       signer_key_id, plaintext_twin)
SELECT v.id, v.pid, 'note.added', '1', v.w, 1, 'n1', v.b,
       '\x1220'::bytea || digest(v.b, 'sha256'), '{}'::jsonb, '[]'::jsonb, 'deadbeef', 'mirror fixture'
  FROM (VALUES ('aaaaaaaa-0069-7000-8000-000000000001'::uuid, 'bbbbbbbb-0069-7000-8000-000000000001'::uuid, 1, '\x6901'::bytea),
               ('aaaaaaaa-0069-7000-8000-000000000002'::uuid, 'bbbbbbbb-0069-7000-8000-000000000002'::uuid, 2, '\x6902'::bytea)) v(id, pid, w, b);

INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, sealed,
                          contact_email, enrolled_by)
VALUES ('nurse-a',  'Nurse A', 'nurse',     '\x01', repeat('a1', 32), '\x00', 'a@x', 'console'),
       ('doctor-b', 'Dr B',    'clinician', '\x02', repeat('b2', 32), '\x00', 'b@x', 'console'),
       ('admin-x',  'Admin X', 'admin',     '\x04', repeat('d4', 32), '\x00', 'x@x', 'console');

-- An event e of type t on chart pid, signed by key k, inserted as the owner in the CALLER's
-- (sub)transaction: a stand-in for what the strict door writes after verifying k's signature.
-- Replica role skips the projections (this fixture has no real body); back to origin after.
CREATE FUNCTION pg_temp.ev(e uuid, pid uuid, t text, k text) RETURNS void LANGUAGE plpgsql AS $$
DECLARE b bytea := convert_to(e::text, 'UTF8');
BEGIN
    PERFORM set_config('session_replication_role', 'replica', true);
    INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
                           node_origin, signed_bytes, content_address, body, contributors,
                           signer_key_id, plaintext_twin)
    VALUES (e, pid, t, '1', 100, 1, 'n1', b,
            '\x1220'::bytea || digest(b, 'sha256'), '{}'::jsonb, '[]'::jsonb, k, 'mirror fixture');
    PERFORM set_config('session_replication_role', 'origin', true);
END $$;

-- The draft's state as one string, to show a refusal wrote nothing: parts, revisions, spend.
CREATE FUNCTION pg_temp.state(d uuid) RETURNS text LANGUAGE sql AS $$
    SELECT (SELECT count(*) FROM note_draft WHERE draft_id = d) || '/'
        || (SELECT count(*) FROM note_draft_part WHERE draft_id = d) || '/'
        || (SELECT count(*) FROM note_draft_revision WHERE draft_id = d) || '/'
        || (SELECT count(*) FROM note_draft_spent WHERE draft_id = d);
$$;

-- The one refusal row's outcome, raising if the door returned anything but one bare row.
CREATE FUNCTION pg_temp.refusal(d uuid, pid uuid, term text, k text) RETURNS text LANGUAGE plpgsql AS $$
DECLARE r record; n int := 0; o text;
BEGIN
    FOR r IN SELECT * FROM cairn_note_draft_discard(d, pid, term, gen_random_uuid(), k) LOOP
        n := n + 1;
        o := r.outcome;
        IF r.seq IS NOT NULL OR r.by_discarder IS NOT NULL OR r.narrative IS NOT NULL OR r.stated_time IS NOT NULL
           OR r.recording_started IS NOT NULL OR r.last_keystroke IS NOT NULL OR r.discarded_at IS NOT NULL THEN
            RAISE EXCEPTION 'a % row carries columns', o;
        END IF;
    END LOOP;
    IF n <> 1 THEN RAISE EXCEPTION 'a refusal must be one row, got %', n; END IF;
    RETURN o;
END $$;

DO $$
DECLARE
    p  uuid := 'bbbbbbbb-0069-7000-8000-000000000001';
    q  uuid := 'bbbbbbbb-0069-7000-8000-000000000002';
    ka text := repeat('a1', 32);
    kb text := repeat('b2', 32);
    d  uuid;
    e  uuid;
    w  text;
    m  int;
    n  int;
BEGIN
    -- own_draft_is_discarded_and_spent_with_the_key: A's part, then a not-known writer's.
    d := 'dddddddd-0069-7000-8000-000000000001';
    e := 'eeeeeeee-0069-7000-8000-000000000001';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'obs stable', '2026-11-04 08:00+00');
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', NULL, 'more', NULL);
    PERFORM pg_temp.ev(e, p, 'clinical.note-discard.asserted', ka);
    SELECT string_agg(r.outcome || ':' || r.seq || ':' || r.by_discarder || ':' || r.narrative || ':'
                      || COALESCE((r.stated_time = '2026-11-04 08:00+00')::text, '-'), ',' ORDER BY r.ord),
           count(DISTINCT r.discarded_at), count(*) FILTER (WHERE r.discarded_at IS NULL
                                                               OR r.recording_started IS NULL
                                                               OR r.last_keystroke < r.recording_started
                                                               OR r.discarded_at > clock_timestamp())
      INTO w, m, n
      FROM (SELECT x.*, row_number() OVER () AS ord FROM cairn_note_draft_discard(d, p, 'term-1', e, ka) x) r;
    IF w IS DISTINCT FROM 'ok:1:true:obs stable:true,ok:2:false:more:-' THEN
        RAISE EXCEPTION 'own_draft_is_discarded_and_spent_with_the_key: %', w;
    END IF;
    IF m <> 1 OR n <> 0 THEN
        RAISE EXCEPTION 'own_draft_is_discarded_and_spent_with_the_key: % instants, % rows with a bad time', m, n;
    END IF;
    IF pg_temp.state(d) <> '0/0/0/1' THEN
        RAISE EXCEPTION 'own_draft_is_discarded_and_spent_with_the_key: left %', pg_temp.state(d);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM note_draft_spent WHERE draft_id = d AND patient_id = p AND event_id = e
                      AND discarded_by = ka AND NOT by_timer) THEN
        RAISE EXCEPTION 'own_draft_is_discarded_and_spent_with_the_key: the spend does not name the key';
    END IF;

    -- every returned row shares ONE discard instant
    d := 'dddddddd-0069-7000-8000-000000000011';
    e := 'eeeeeeee-0069-7000-8000-000000000011';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'one', NULL);
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', NULL, 'two', NULL);
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'three', NULL);
    PERFORM pg_temp.ev(e, p, 'clinical.note-discard.asserted', ka);
    SELECT count(*), count(DISTINCT x.discarded_at) INTO m, n FROM cairn_note_draft_discard(d, p, 'term-1', e, ka) x;
    IF m <> 3 OR n <> 1 THEN RAISE EXCEPTION 'one discard instant per call: % rows, % instants', m, n; END IF;

    -- save_after_discard_answers_discarded, take_after_discard_answers_discarded, and the door again
    d := 'dddddddd-0069-7000-8000-000000000001';
    w := cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'one more line', NULL);
    IF w IS DISTINCT FROM 'discarded' OR pg_temp.state(d) <> '0/0/0/1' THEN
        RAISE EXCEPTION 'save_after_discard_answers_discarded: % (left %)', w, pg_temp.state(d);
    END IF;
    w := cairn_note_draft_take(d, p, 'term-2');
    IF w IS DISTINCT FROM 'discarded' THEN RAISE EXCEPTION 'take_after_discard_answers_discarded: %', w; END IF;
    w := pg_temp.refusal(d, p, 'term-1', ka);
    IF w IS DISTINCT FROM 'discarded' THEN RAISE EXCEPTION 'a second discard answers discarded: %', w; END IF;

    -- unknown_writer_draft_any_clinical_signer_may_discard
    d := 'dddddddd-0069-7000-8000-000000000002';
    e := 'eeeeeeee-0069-7000-8000-000000000002';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', NULL, 'who wrote this', NULL);
    PERFORM pg_temp.ev(e, p, 'clinical.note-discard.asserted', kb);
    SELECT string_agg(x.outcome || ':' || x.by_discarder, ',') INTO w FROM cairn_note_draft_discard(d, p, 'term-1', e, kb) x;
    IF w IS DISTINCT FROM 'ok:false' OR pg_temp.state(d) <> '0/0/0/1' THEN
        RAISE EXCEPTION 'unknown_writer_draft_any_clinical_signer_may_discard: % (left %)', w, pg_temp.state(d);
    END IF;

    -- a_colleagues_part_is_not_yours: A's observations, B's plan; A may not discard B's writing.
    d := 'dddddddd-0069-7000-8000-000000000003';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'obs', NULL);
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'doctor-b', 'plan', NULL);
    w := pg_temp.refusal(d, p, 'term-1', ka);
    IF w IS DISTINCT FROM 'not-yours' OR pg_temp.state(d) <> '1/2/2/0' THEN
        RAISE EXCEPTION 'a_colleagues_part_is_not_yours: % (left %)', w, pg_temp.state(d);
    END IF;

    -- a_named_writer_without_a_key_is_not_yours: the writer left (no vault key). Never read as
    -- "not known": that would quietly lose who wrote it (principle 4). Nobody's key passes.
    d := 'dddddddd-0069-7000-8000-000000000004';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'obs', NULL);
    UPDATE note_draft_part SET writer = 'left-l' WHERE draft_id = d;
    IF pg_temp.refusal(d, p, 'term-1', ka) IS DISTINCT FROM 'not-yours'
       OR pg_temp.refusal(d, p, 'term-1', kb) IS DISTINCT FROM 'not-yours' OR pg_temp.state(d) <> '1/1/1/0' THEN
        RAISE EXCEPTION 'a_named_writer_without_a_key_is_not_yours';
    END IF;

    -- a_blank_colleague_part_does_not_count: B wrote, then blanked his own part (NBSP only).
    d := 'dddddddd-0069-7000-8000-000000000005';
    e := 'eeeeeeee-0069-7000-8000-000000000005';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'obs', NULL);
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'doctor-b', 'plan', NULL);
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'doctor-b', U&'\00A0 ', NULL);
    PERFORM pg_temp.ev(e, p, 'clinical.note-discard.asserted', ka);
    SELECT string_agg(x.outcome || ':' || x.seq || ':' || x.narrative, ',') INTO w
      FROM cairn_note_draft_discard(d, p, 'term-1', e, ka) x;
    IF w IS DISTINCT FROM 'ok:1:obs' OR pg_temp.state(d) <> '0/0/0/1' THEN
        RAISE EXCEPTION 'a_blank_colleague_part_does_not_count: % (left %)', w, pg_temp.state(d);
    END IF;

    -- an_admin_key_is_no_such_signer (and an unknown key): an admin confers no clinical authorship.
    d := 'dddddddd-0069-7000-8000-000000000006';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', NULL, 'unknown writer', NULL);
    IF pg_temp.refusal(d, p, 'term-1', repeat('d4', 32)) IS DISTINCT FROM 'no-such-signer'
       OR pg_temp.refusal(d, p, 'term-1', repeat('ff', 32)) IS DISTINCT FROM 'no-such-signer'
       OR pg_temp.state(d) <> '1/1/1/0' THEN
        RAISE EXCEPTION 'an_admin_key_is_no_such_signer';
    END IF;

    -- all_blank_is_deleted_empty_no_spend (design §3.5: nothing to audit, so no event is needed)
    d := 'dddddddd-0069-7000-8000-000000000007';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'typed', NULL);
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', U&'\00A0\3000 ', NULL);
    w := pg_temp.refusal(d, p, 'term-1', ka);
    IF w IS DISTINCT FROM 'empty' OR pg_temp.state(d) <> '0/0/0/0' THEN
        RAISE EXCEPTION 'all_blank_is_deleted_empty_no_spend: % (left %)', w, pg_temp.state(d);
    END IF;

    -- discard_after_timer_answers_committed
    d := 'dddddddd-0069-7000-8000-000000000008';
    e := 'eeeeeeee-0069-7000-8000-000000000008';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'left alone', NULL);
    UPDATE note_draft SET recording_started = clock_timestamp() - interval '190 minutes',
                          last_keystroke = clock_timestamp() - interval '181 minutes' WHERE draft_id = d;
    PERFORM pg_temp.ev(e, p, 'clinical.note.asserted', 'deadbeef');
    PERFORM cairn_note_draft_commit_overdue(d, e);
    w := pg_temp.refusal(d, p, 'term-1', ka);
    IF w IS DISTINCT FROM 'committed' THEN RAISE EXCEPTION 'discard_after_timer_answers_committed: %', w; END IF;

    -- discard_after_sign_answers_spent
    d := 'dddddddd-0069-7000-8000-000000000009';
    e := 'eeeeeeee-0069-7000-8000-000000000009';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'signed', NULL);
    PERFORM pg_temp.ev(e, p, 'clinical.note.asserted', ka);
    PERFORM cairn_note_draft_spend_parts(d, e);
    w := pg_temp.refusal(d, p, 'term-1', ka);
    IF w IS DISTINCT FROM 'spent' THEN RAISE EXCEPTION 'discard_after_sign_answers_spent: %', w; END IF;

    -- moved_and_elsewhere_and_gone: none of them writes.
    d := 'dddddddd-0069-7000-8000-000000000010';
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'here', NULL);
    IF pg_temp.refusal(d, p, 'term-2', ka) IS DISTINCT FROM 'moved'
       OR pg_temp.refusal(d, q, 'term-1', ka) IS DISTINCT FROM 'elsewhere'
       OR pg_temp.refusal(gen_random_uuid(), p, 'term-1', ka) IS DISTINCT FROM 'gone'
       OR pg_temp.state(d) <> '1/1/1/0' THEN
        RAISE EXCEPTION 'moved_and_elsewhere_and_gone';
    END IF;
END $$;

-- Every discard above named a discard event of its chart, signed by its discarder's key, in this
-- transaction (and every Sign or timer spend its note): the commit-time check passes them all.
SET CONSTRAINTS note_draft_spent_names_its_note IMMEDIATE;

-- A raw-SQL client holding only key IDS (public) and no private key: the door would consume the
-- draft, but the spend must name an event the strict door accepted under that key, and it has none.
-- the_commit_refuses_a_spend_without_its_signed_event (B discards A's draft with A's key id),
-- the_commit_refuses_an_event_signed_by_another_key (and the wrong chart, the wrong type, an
-- event from a subtransaction). The constraint is IMMEDIATE now, so the door's INSERT raises.
DO $$
DECLARE
    p   uuid := 'bbbbbbbb-0069-7000-8000-000000000001';
    q   uuid := 'bbbbbbbb-0069-7000-8000-000000000002';
    ka  text := repeat('a1', 32);
    kb  text := repeat('b2', 32);
    d   uuid := 'dddddddd-0069-7000-8000-000000000012';
    bad uuid;
    msg text;
BEGIN
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'mine', NULL);
    -- Top-level inserts (no exception block yet), so their xmin IS this transaction's id.
    PERFORM pg_temp.ev('eeeeeeee-0069-7000-8000-0000000000b1', p, 'clinical.note-discard.asserted', kb);
    PERFORM pg_temp.ev('eeeeeeee-0069-7000-8000-0000000000b2', q, 'clinical.note-discard.asserted', ka);
    PERFORM pg_temp.ev('eeeeeeee-0069-7000-8000-0000000000b3', p, 'clinical.note.asserted', ka);
    FOREACH bad IN ARRAY ARRAY[gen_random_uuid(), 'eeeeeeee-0069-7000-8000-0000000000b1',
                               'eeeeeeee-0069-7000-8000-0000000000b2', 'eeeeeeee-0069-7000-8000-0000000000b3',
                               'eeeeeeee-0069-7000-8000-0000000000b4']::uuid[] LOOP
        msg := NULL;
        BEGIN
            IF bad = 'eeeeeeee-0069-7000-8000-0000000000b4' THEN
                -- Inside this block's subtransaction: its xmin is a subxid, not the top-level id.
                PERFORM pg_temp.ev(bad, p, 'clinical.note-discard.asserted', ka);
            END IF;
            PERFORM * FROM cairn_note_draft_discard(d, p, 'term-1', bad, ka);
        EXCEPTION WHEN raise_exception THEN msg := SQLERRM;
        END;
        IF msg IS NULL OR msg NOT LIKE '%clinical.note-discard.asserted%' THEN
            RAISE EXCEPTION 'a discard spend naming % was not refused by the discard arm: %', bad, msg;
        END IF;
    END LOOP;
    IF pg_temp.state(d) <> '1/1/1/0' THEN RAISE EXCEPTION 'a refused discard left %', pg_temp.state(d); END IF;
    -- Control: the same draft, an event of this chart signed by A's key, passes.
    PERFORM pg_temp.ev('eeeeeeee-0069-7000-8000-0000000000b5', p, 'clinical.note-discard.asserted', ka);
    PERFORM * FROM cairn_note_draft_discard(d, p, 'term-1', 'eeeeeeee-0069-7000-8000-0000000000b5', ka);
    IF pg_temp.state(d) <> '0/0/0/1' THEN RAISE EXCEPTION 'the control discard left %', pg_temp.state(d); END IF;
END $$;

-- A Sign's spend keeps the note arm: a discard event never backs a Sign (discarded_by IS NULL).
DO $$
DECLARE
    p   uuid := 'bbbbbbbb-0069-7000-8000-000000000001';
    d   uuid := 'dddddddd-0069-7000-8000-000000000013';
    msg text;
BEGIN
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'sign me', NULL);
    PERFORM pg_temp.ev('eeeeeeee-0069-7000-8000-0000000000c1', p, 'clinical.note-discard.asserted', repeat('a1', 32));
    BEGIN
        PERFORM cairn_note_draft_spend_parts(d, 'eeeeeeee-0069-7000-8000-0000000000c1');
    EXCEPTION WHEN raise_exception THEN msg := SQLERRM;
    END;
    IF msg IS NULL OR msg NOT LIKE '%clinical.note.asserted%' THEN
        RAISE EXCEPTION 'a Sign spend backed by a discard event was not refused by the note arm: %', msg;
    END IF;
END $$;

-- NULLs are caller bugs, refused.
DO $$
DECLARE c text;
BEGIN
    FOREACH c IN ARRAY ARRAY[
        'SELECT * FROM cairn_note_draft_discard(NULL, gen_random_uuid(), ''t'', gen_random_uuid(), ''k'')',
        'SELECT * FROM cairn_note_draft_discard(gen_random_uuid(), NULL, ''t'', gen_random_uuid(), ''k'')',
        'SELECT * FROM cairn_note_draft_discard(gen_random_uuid(), gen_random_uuid(), NULL, gen_random_uuid(), ''k'')',
        'SELECT * FROM cairn_note_draft_discard(gen_random_uuid(), gen_random_uuid(), ''t'', NULL, ''k'')',
        'SELECT * FROM cairn_note_draft_discard(gen_random_uuid(), gen_random_uuid(), ''t'', gen_random_uuid(), NULL)'] LOOP
        BEGIN
            EXECUTE c;
            RAISE EXCEPTION 'a NULL was accepted: %', c USING ERRCODE = 'P0002';
        EXCEPTION WHEN raise_exception THEN NULL;
        END;
    END LOOP;
END $$;

-- note_discard_is_a_timing_kind (db/044, in place): the window times the Discard gesture.
INSERT INTO ui_gesture_timing (gesture_kind, size_bucket) VALUES ('note-discard', '1-3')
ON CONFLICT DO NOTHING;

-- One definition of blank, and the door inventory: not PUBLIC, the window's, definer-run with a
-- pinned search_path.
DO $$
DECLARE f text := 'cairn_note_draft_discard(uuid, uuid, text, uuid, text)';
BEGIN
    IF pg_get_functiondef(f::regprocedure) NOT LIKE '%cairn_twin_is_present(%'
       OR pg_get_functiondef(f::regprocedure) LIKE '%[:space:]%' THEN
        RAISE EXCEPTION '% must test blankness with cairn_twin_is_present only', f;
    END IF;
    IF has_function_privilege('public', f, 'EXECUTE') THEN RAISE EXCEPTION '% must not be PUBLIC', f; END IF;
    IF NOT has_function_privilege('cairn_window', f, 'EXECUTE') THEN RAISE EXCEPTION '% is the window''s', f; END IF;
    IF has_function_privilege('cairn_agent', f, 'EXECUTE') OR has_function_privilege('cairn_node', f, 'EXECUTE') THEN
        RAISE EXCEPTION '% is the window''s only', f;
    END IF;
    IF NOT (SELECT prosecdef AND proconfig @> ARRAY['search_path=public, pg_temp'] FROM pg_proc
             WHERE oid = f::regprocedure) THEN
        RAISE EXCEPTION '% must be definer-run with a pinned search_path', f;
    END IF;
END $$;

ROLLBACK;
