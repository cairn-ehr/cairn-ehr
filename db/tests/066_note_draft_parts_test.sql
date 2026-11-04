\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/066 — a note draft is a sequence of writers' PARTS (sign-in slice 3b-1, Task 1).
--
-- WHY: on a ward several people write into one note before one of them signs it (a nurse's
-- observations, then the doctor's plan). Each person's words must stay theirs: a save only ever
-- rewrites the writer's OWN tail part, and anyone else's save appends a new part, so no door can
-- change or remove another writer's text. These cases pin that, plus the move door ('moved' for the
-- old terminal afterwards, #958's split from 'elsewhere'), the revision rows, the spend, the 3a
-- migration and the staff directory. Nothing here commits (ROLLBACK at the end).

BEGIN;

-- Fixtures: chart P with one event, chart Q with one event, and four vault members — two
-- clinical (A, B), a superuser and an admin. Members are written directly, as db/tests/063 does;
-- the `sealed` blob is a placeholder nobody releases.
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
                       node_origin, signed_bytes, content_address, body, contributors,
                       signer_key_id, plaintext_twin)
SELECT v.id, v.pid, 'note.added', '1', v.w, 1, 'n1', v.b,
       '\x1220'::bytea || digest(v.b, 'sha256'), '{}'::jsonb, '[]'::jsonb, 'deadbeef', 'mirror fixture'
  FROM (VALUES ('aaaaaaaa-0066-7000-8000-000000000001'::uuid, 'bbbbbbbb-0066-7000-8000-000000000001'::uuid, 1, '\x6601'::bytea),
               ('aaaaaaaa-0066-7000-8000-000000000002'::uuid, 'bbbbbbbb-0066-7000-8000-000000000002'::uuid, 2, '\x6602'::bytea)) v(id, pid, w, b);

-- The note event the spends below name (the projection needs a full body, so replica role).
SET LOCAL session_replication_role = replica;
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
                       node_origin, signed_bytes, content_address, body, contributors,
                       signer_key_id, plaintext_twin)
VALUES ('aaaaaaaa-0066-7000-8000-0000000000a1', 'bbbbbbbb-0066-7000-8000-000000000001',
        'clinical.note.asserted', '1', 3, 1, 'n1', '\x66a1'::bytea,
        '\x1220'::bytea || digest('\x66a1'::bytea, 'sha256'),
        '{}'::jsonb, '[]'::jsonb, 'deadbeef', 'mirror fixture');
SET LOCAL session_replication_role = origin;

INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, sealed,
                          contact_email, enrolled_by)
VALUES ('nurse-a',    'Nurse A',      'nurse',     '\x01', repeat('a1', 32), '\x00', 'a@x', 'console'),
       ('doctor-b',   'Dr B',         'clinician', '\x02', repeat('b2', 32), '\x00', 'b@x', 'console'),
       ('super-s',    'Super S',      'superuser', '\x03', repeat('c3', 32), '\x00', 's@x', 'console'),
       ('admin-x',    'Admin X',      'admin',     '\x04', repeat('d4', 32), '\x00', 'x@x', 'console');

-- migrated_3a_draft_is_part_1_unknown_writer: put back the two 3a columns, store a 3a-shaped
-- draft (a header with its text and no parts), and replay the REAL db/066. The draft must come
-- out as part 1 with an unknown (NULL) writer and its old text, and the columns must be gone.
ALTER TABLE note_draft ADD COLUMN narrative text, ADD COLUMN stated_time timestamptz;
INSERT INTO note_draft (draft_id, patient_id, terminal_id, narrative, stated_time, recording_started, last_keystroke)
VALUES ('dddddddd-0066-7000-8000-000000000301', 'bbbbbbbb-0066-7000-8000-000000000001', 'term-1',
        E'3a text *as typed*', '2026-01-01 10:00+00', '2026-11-01 01:00+00', '2026-11-01 01:05+00');
\set f_066 `cat db/066_note_draft_parts.sql`
SELECT :'f_066' AS q \gexec
DO $$
DECLARE r record;
BEGIN
    SELECT * INTO STRICT r FROM note_draft_part WHERE draft_id = 'dddddddd-0066-7000-8000-000000000301';
    IF r.seq <> 1 OR r.writer IS NOT NULL OR r.narrative <> E'3a text *as typed*'
       OR r.stated_time <> '2026-01-01 10:00+00' OR r.recording_started <> '2026-11-01 01:00+00'
       OR r.last_keystroke <> '2026-11-01 01:05+00' THEN
        RAISE EXCEPTION 'migrated_3a_draft_is_part_1_unknown_writer: got %', r;
    END IF;
    IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema = 'public'
                AND table_name = 'note_draft' AND column_name IN ('narrative', 'stated_time')) THEN
        RAISE EXCEPTION 'migrated_3a_draft_is_part_1_unknown_writer: the 3a columns are still there';
    END IF;
END $$;

-- The text of draft d's part s, or NULL.
CREATE FUNCTION pg_temp.part_text(d uuid, s int) RETURNS text LANGUAGE sql AS $$
    SELECT narrative FROM note_draft_part WHERE draft_id = d AND seq = s;
$$;
CREATE FUNCTION pg_temp.writers(d uuid) RETURNS text LANGUAGE sql AS $$
    SELECT string_agg(COALESCE(writer, '?'), ',' ORDER BY seq) FROM note_draft_part WHERE draft_id = d;
$$;

DO $$
DECLARE
    p  uuid := 'bbbbbbbb-0066-7000-8000-000000000001';
    q  uuid := 'bbbbbbbb-0066-7000-8000-000000000002';
    d  uuid;
    r  record;
    t  timestamptz;
    w  text;
    n  int;
BEGIN
    -- first_save_makes_part_1
    d := 'dddddddd-0066-7000-8000-000000000001';
    IF cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'obs stable', NULL) <> 'saved' THEN
        RAISE EXCEPTION 'first_save_makes_part_1: not saved';
    END IF;
    SELECT count(*) INTO n FROM note_draft WHERE draft_id = d;
    SELECT * INTO r FROM note_draft_part WHERE draft_id = d;
    IF n <> 1 OR r.seq <> 1 OR r.writer <> 'nurse-a' OR r.narrative <> 'obs stable'
       OR r.recording_started <> r.last_keystroke THEN
        RAISE EXCEPTION 'first_save_makes_part_1: got % headers, part %', n, r;
    END IF;

    -- same_writer_rewrites_the_tail
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'obs stable, afebrile', NULL);
    IF pg_temp.writers(d) <> 'nurse-a' OR pg_temp.part_text(d, 1) <> 'obs stable, afebrile' THEN
        RAISE EXCEPTION 'same_writer_rewrites_the_tail: % / %', pg_temp.writers(d), pg_temp.part_text(d, 1);
    END IF;

    -- another_writer_appends
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'doctor-b', 'plan: review', NULL);
    IF pg_temp.writers(d) <> 'nurse-a,doctor-b' OR pg_temp.part_text(d, 1) <> 'obs stable, afebrile'
       OR pg_temp.part_text(d, 2) <> 'plan: review' THEN
        RAISE EXCEPTION 'another_writer_appends: %', pg_temp.writers(d);
    END IF;

    -- b_cannot_reach_part_1: every argument combination a caller has leaves part 1 byte-equal to
    -- A's last save — B rewriting their own tail, B from another terminal or chart, a NULL
    -- writer, B in other case, an unknown writer.
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'doctor-b', 'obs stable, afebrile', NULL);
    PERFORM cairn_note_draft_part_save(d, p, 'term-2', 'doctor-b', 'clobber', NULL);
    PERFORM cairn_note_draft_part_save(d, q, 'term-1', 'doctor-b', 'clobber', NULL);
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', ' DOCTOR-B ', 'plan: review', '2026-01-01 10:00+00');
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nobody', 'clobber', NULL);
    IF pg_temp.part_text(d, 1) IS DISTINCT FROM 'obs stable, afebrile'
       OR (SELECT writer FROM note_draft_part WHERE draft_id = d AND seq = 1) <> 'nurse-a' THEN
        RAISE EXCEPTION 'b_cannot_reach_part_1: part 1 is now %', pg_temp.part_text(d, 1);
    END IF;

    -- a_b_a_is_three_parts (Review Focus 3): A types again after B wrote below — a NEW part,
    -- never A's text merged into part 1.
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'BP rechecked', NULL);
    IF pg_temp.writers(d) <> 'nurse-a,doctor-b,nurse-a' OR pg_temp.part_text(d, 3) <> 'BP rechecked'
       OR pg_temp.part_text(d, 1) <> 'obs stable, afebrile' THEN
        RAISE EXCEPTION 'a_b_a_is_three_parts: % / %', pg_temp.writers(d), pg_temp.part_text(d, 1);
    END IF;

    -- an_empty_save_by_a_new_writer_makes_no_part (whitespace is empty too)
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'doctor-b', E'  \n ', NULL);
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', NULL, '', NULL);
    IF pg_temp.writers(d) <> 'nurse-a,doctor-b,nurse-a' THEN
        RAISE EXCEPTION 'an_empty_save_by_a_new_writer_makes_no_part: %', pg_temp.writers(d);
    END IF;

    -- an_unknown_staff_id_answers_no_such_writer
    w := cairn_note_draft_part_save(d, p, 'term-1', 'nobody', 'x', NULL);
    IF w <> 'no-such-writer' THEN RAISE EXCEPTION 'an_unknown_staff_id_answers_no_such_writer: %', w; END IF;
    w := cairn_note_draft_part_save(gen_random_uuid(), p, 'term-1', '', 'x', NULL);
    IF w <> 'no-such-writer' THEN RAISE EXCEPTION 'an empty staff id is no writer: %', w; END IF;

    -- a_staff_id_in_other_case_is_normalised: ' Nurse-A ' is nurse-a, so it rewrites her tail.
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', ' Nurse-A ', 'BP rechecked 120/80', NULL);
    IF pg_temp.writers(d) <> 'nurse-a,doctor-b,nurse-a' OR pg_temp.part_text(d, 3) <> 'BP rechecked 120/80' THEN
        RAISE EXCEPTION 'a_staff_id_in_other_case_is_normalised: %', pg_temp.writers(d);
    END IF;

    -- moved_vs_elsewhere
    w := cairn_note_draft_part_save(d, p, 'term-2', 'nurse-a', 'x', NULL);
    IF w <> 'moved' THEN RAISE EXCEPTION 'moved_vs_elsewhere: another terminal answered %', w; END IF;
    w := cairn_note_draft_part_save(d, q, 'term-1', 'nurse-a', 'x', NULL);
    IF w <> 'elsewhere' THEN RAISE EXCEPTION 'moved_vs_elsewhere: another chart answered %', w; END IF;
    w := cairn_note_draft_part_save(gen_random_uuid(), 'bbbbbbbb-0066-7000-8000-0000000000ff', 'term-1', 'nurse-a', 'x', NULL);
    IF w <> 'no-chart' THEN RAISE EXCEPTION 'a patient with no event has no chart: %', w; END IF;

    -- take_moves_and_the_old_terminal_is_moved
    w := cairn_note_draft_take(d, p, 'term-2');
    IF w <> 'taken' THEN RAISE EXCEPTION 'take_moves_and_the_old_terminal_is_moved: take answered %', w; END IF;
    w := cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'late keystroke', NULL);
    IF w <> 'moved' THEN RAISE EXCEPTION 'take_moves_and_the_old_terminal_is_moved: T1 answered %', w; END IF;
    w := cairn_note_draft_part_save(d, p, 'term-2', 'nurse-a', 'BP rechecked 120/80, at T2', NULL);
    IF w <> 'saved' OR pg_temp.part_text(d, 3) <> 'BP rechecked 120/80, at T2' THEN
        RAISE EXCEPTION 'take_moves_and_the_old_terminal_is_moved: T2 answered %', w;
    END IF;
    IF pg_temp.part_text(d, 1) <> 'obs stable, afebrile' OR pg_temp.part_text(d, 2) <> 'plan: review' THEN
        RAISE EXCEPTION 'a take changes nothing but the terminal';
    END IF;

    -- take_of_unknown_is_gone, take_naming_another_chart_is_elsewhere
    w := cairn_note_draft_take(gen_random_uuid(), p, 'term-2');
    IF w <> 'gone' THEN RAISE EXCEPTION 'take_of_unknown_is_gone: %', w; END IF;
    w := cairn_note_draft_take(d, q, 'term-3');
    IF w <> 'elsewhere' OR (SELECT terminal_id FROM note_draft WHERE draft_id = d) <> 'term-2' THEN
        RAISE EXCEPTION 'take_naming_another_chart_is_elsewhere: %', w;
    END IF;

    -- The listing: one row per part, in order, with the writer's display name.
    SELECT string_agg(seq || ':' || COALESCE(writer_name, '?'), ',' ORDER BY seq) INTO w
      FROM cairn_note_draft_parts(p) WHERE draft_id = d;
    IF w <> '1:Nurse A,2:Dr B,3:Nurse A' THEN RAISE EXCEPTION 'listing: %', w; END IF;
    SELECT writer_name INTO w FROM cairn_note_draft_parts(p) WHERE draft_id = 'dddddddd-0066-7000-8000-000000000301';
    IF w IS NOT NULL THEN RAISE EXCEPTION 'an unknown writer has no name: %', w; END IF;
END $$;

-- every_real_change_is_a_revision: three changed saves and one unchanged save give 3 revision
-- rows, and the unchanged one moves neither last_keystroke (ruling R1) nor the count.
DO $$
DECLARE
    d  uuid := 'dddddddd-0066-7000-8000-000000000002';
    p  uuid := 'bbbbbbbb-0066-7000-8000-000000000001';
    t1 timestamptz; h1 timestamptz;
    n  int;
BEGIN
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'one', NULL);
    PERFORM pg_sleep(0.01);
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'one two', NULL);
    PERFORM pg_sleep(0.01);
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'one two', '2026-01-01 10:00+00');
    SELECT last_keystroke INTO t1 FROM note_draft_part WHERE draft_id = d AND seq = 1;
    SELECT last_keystroke INTO h1 FROM note_draft WHERE draft_id = d;
    PERFORM pg_sleep(0.01);
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'one two', '2026-01-01 10:00+00');
    SELECT count(*) INTO n FROM note_draft_revision WHERE draft_id = d;
    IF n <> 3 THEN RAISE EXCEPTION 'every_real_change_is_a_revision: % revisions', n; END IF;
    IF (SELECT last_keystroke FROM note_draft_part WHERE draft_id = d AND seq = 1) <> t1
       OR (SELECT last_keystroke FROM note_draft WHERE draft_id = d) <> h1 THEN
        RAISE EXCEPTION 'every_real_change_is_a_revision: an unchanged save moved last_keystroke';
    END IF;
    IF (SELECT array_agg(narrative ORDER BY rev) FROM note_draft_revision WHERE draft_id = d)
       <> ARRAY['one', 'one two', 'one two'] THEN
        RAISE EXCEPTION 'every_real_change_is_a_revision: the rows do not hold each saved text';
    END IF;
    IF EXISTS (SELECT 1 FROM note_draft_revision WHERE draft_id = d
                AND (writer <> 'nurse-a' OR terminal_id <> 'term-1' OR seq <> 1 OR saved_at IS NULL)) THEN
        RAISE EXCEPTION 'every_real_change_is_a_revision: a row lost its writer, terminal or part';
    END IF;
    -- A changed save moves BOTH the part's and the header's last keystroke.
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'one two three', '2026-01-01 10:00+00');
    IF (SELECT last_keystroke FROM note_draft_part WHERE draft_id = d AND seq = 1) <= t1
       OR (SELECT last_keystroke FROM note_draft WHERE draft_id = d) <= h1 THEN
        RAISE EXCEPTION 'a changed save must move last_keystroke';
    END IF;
END $$;

-- spend_returns_nonempty_parts_in_order_and_clears_revisions. Draft 3: A writes, B writes, A's
-- tail is emptied again by a later A save (an empty part is kept while typing, never signed).
DO $$
DECLARE
    d  uuid := 'dddddddd-0066-7000-8000-000000000003';
    p  uuid := 'bbbbbbbb-0066-7000-8000-000000000001';
    w  text;
BEGIN
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'obs', NULL);
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'doctor-b', 'plan', NULL);
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'more', NULL);
    PERFORM cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', '  ', NULL);
    SELECT string_agg(s.seq || ':' || s.writer || ':' || s.narrative, ',' ORDER BY s.seq) INTO w
      FROM cairn_note_draft_spend_parts(d, 'aaaaaaaa-0066-7000-8000-0000000000a1') s
     WHERE s.patient_id = p;
    IF w IS DISTINCT FROM '1:nurse-a:obs,2:doctor-b:plan' THEN
        RAISE EXCEPTION 'spend_returns_nonempty_parts_in_order_and_clears_revisions: %', w;
    END IF;
    IF EXISTS (SELECT 1 FROM note_draft WHERE draft_id = d) OR EXISTS (SELECT 1 FROM note_draft_part WHERE draft_id = d)
       OR EXISTS (SELECT 1 FROM note_draft_revision WHERE draft_id = d) THEN
        RAISE EXCEPTION 'spend must remove the header, its parts and its revisions';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM note_draft_spent WHERE draft_id = d AND event_id = 'aaaaaaaa-0066-7000-8000-0000000000a1') THEN
        RAISE EXCEPTION 'spend must record the spend';
    END IF;
    -- A spent draft never comes back, and is never taken or spent again.
    w := cairn_note_draft_part_save(d, p, 'term-1', 'nurse-a', 'again', NULL);
    IF w <> 'spent' OR EXISTS (SELECT 1 FROM note_draft WHERE draft_id = d) THEN
        RAISE EXCEPTION 'a save after a spend answered % (or wrote)', w;
    END IF;
    -- take_of_spent_is_spent
    w := cairn_note_draft_take(d, p, 'term-2');
    IF w <> 'spent' THEN RAISE EXCEPTION 'take_of_spent_is_spent: %', w; END IF;
    IF EXISTS (SELECT 1 FROM cairn_note_draft_spend_parts(d, gen_random_uuid())) THEN
        RAISE EXCEPTION 'a second spend must return zero rows';
    END IF;
    IF EXISTS (SELECT 1 FROM cairn_note_draft_spend_parts(gen_random_uuid(), gen_random_uuid())) THEN
        RAISE EXCEPTION 'spending an absent draft must return zero rows';
    END IF;
END $$;
-- The backed spend above passes the commit-time check.
SET CONSTRAINTS note_draft_spent_names_its_note IMMEDIATE;

-- A draft whose every part is empty is NOT spent: zero rows means nothing was spent.
DO $$
DECLARE d uuid := 'dddddddd-0066-7000-8000-000000000004';
BEGIN
    PERFORM cairn_note_draft_part_save(d, 'bbbbbbbb-0066-7000-8000-000000000001', 'term-1', 'nurse-a', ' ', NULL);
    IF EXISTS (SELECT 1 FROM cairn_note_draft_spend_parts(d, gen_random_uuid())) THEN
        RAISE EXCEPTION 'an all-empty draft returned parts';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM note_draft WHERE draft_id = d) OR EXISTS (SELECT 1 FROM note_draft_spent WHERE draft_id = d) THEN
        RAISE EXCEPTION 'an all-empty draft must be left unspent';
    END IF;
END $$;

-- spend_still_needs_its_note_in_this_transaction: 3a's committed-earlier case against the new
-- door. A note inserted in a subtransaction carries a foreign xid (standing in for "committed
-- earlier"), so the deferred trigger refuses the spend it backs.
SET LOCAL session_replication_role = replica;
DO $$
BEGIN
    BEGIN
        INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
                               node_origin, signed_bytes, content_address, body, contributors,
                               signer_key_id, plaintext_twin)
        VALUES ('aaaaaaaa-0066-7000-8000-0000000000a2', 'bbbbbbbb-0066-7000-8000-000000000001',
                'clinical.note.asserted', '1', 4, 1, 'n1', '\x66a2'::bytea,
                '\x1220'::bytea || digest('\x66a2'::bytea, 'sha256'), '{}'::jsonb, '[]'::jsonb, 'deadbeef', 'mirror fixture');
    EXCEPTION WHEN unique_violation THEN NULL;
    END;
END $$;
SET LOCAL session_replication_role = origin;
DO $$
DECLARE refused boolean := false;
BEGIN
    BEGIN
        PERFORM cairn_note_draft_part_save('dddddddd-0066-7000-8000-000000000005', 'bbbbbbbb-0066-7000-8000-000000000001',
                                           'term-1', 'nurse-a', 'x', NULL);
        PERFORM cairn_note_draft_spend_parts('dddddddd-0066-7000-8000-000000000005', 'aaaaaaaa-0066-7000-8000-0000000000a2');
        SET CONSTRAINTS note_draft_spent_names_its_note IMMEDIATE;
    EXCEPTION WHEN raise_exception THEN refused := true;
    END;
    IF NOT refused THEN
        RAISE EXCEPTION 'spend_still_needs_its_note_in_this_transaction: an older note backed a spend';
    END IF;
END $$;

-- NULLs are caller bugs, refused, on every door that touches a draft.
DO $$
DECLARE
    d uuid := 'dddddddd-0066-7000-8000-000000000001';
    p uuid := 'bbbbbbbb-0066-7000-8000-000000000001';
    calls text[] := ARRAY[
        format('SELECT cairn_note_draft_part_save(NULL, %L, ''term-2'', NULL, ''x'', NULL)', p),
        format('SELECT cairn_note_draft_part_save(%L, NULL, ''term-2'', NULL, ''x'', NULL)', d),
        format('SELECT cairn_note_draft_part_save(%L, %L, NULL, NULL, ''x'', NULL)', d, p),
        format('SELECT cairn_note_draft_take(NULL, %L, ''term-2'')', p),
        format('SELECT cairn_note_draft_take(%L, NULL, ''term-2'')', d),
        format('SELECT cairn_note_draft_take(%L, %L, NULL)', d, p)];
    c text;
BEGIN
    FOREACH c IN ARRAY calls LOOP
        BEGIN
            EXECUTE c;
            RAISE EXCEPTION 'a NULL was accepted: %', c USING ERRCODE = 'P0002';
        EXCEPTION WHEN raise_exception THEN NULL;
        END;
    END LOOP;
    IF (SELECT terminal_id FROM note_draft WHERE draft_id = d) <> 'term-2' THEN
        RAISE EXCEPTION 'a NULL call moved the draft';
    END IF;
END $$;

-- A backward clock step never stamps a part's last keystroke before its start (3a's final review
-- I1, now per part). Simulated by pushing the part's times an hour into the future, as the owner.
DO $$
DECLARE d uuid := 'dddddddd-0066-7000-8000-000000000006'; r record;
BEGIN
    PERFORM cairn_note_draft_part_save(d, 'bbbbbbbb-0066-7000-8000-000000000001', 'term-1', 'nurse-a', 'before', NULL);
    UPDATE note_draft SET recording_started = clock_timestamp() + interval '1 hour',
                          last_keystroke = clock_timestamp() + interval '1 hour' WHERE draft_id = d;
    UPDATE note_draft_part SET recording_started = clock_timestamp() + interval '1 hour',
                               last_keystroke = clock_timestamp() + interval '1 hour' WHERE draft_id = d;
    PERFORM cairn_note_draft_part_save(d, 'bbbbbbbb-0066-7000-8000-000000000001', 'term-1', 'nurse-a', 'after', NULL);
    SELECT recording_started, last_keystroke INTO r FROM note_draft_part WHERE draft_id = d;
    IF r.last_keystroke < r.recording_started THEN RAISE EXCEPTION 'part keystroke before its start'; END IF;
    SELECT recording_started, last_keystroke INTO r FROM note_draft WHERE draft_id = d;
    IF r.last_keystroke < r.recording_started THEN RAISE EXCEPTION 'header keystroke before its start'; END IF;
END $$;

-- directory_lists_clinical_roles_only: a superuser and an admin are absent; names in order.
DO $$
DECLARE w text;
BEGIN
    SELECT string_agg(staff_id, ',' ORDER BY ord) INTO w
      FROM (SELECT staff_id, row_number() OVER () AS ord FROM cairn_whos_here_directory()) s
     WHERE staff_id IN ('nurse-a', 'doctor-b', 'super-s', 'admin-x');
    IF w IS DISTINCT FROM 'doctor-b,nurse-a' THEN RAISE EXCEPTION 'directory_lists_clinical_roles_only: %', w; END IF;
    IF NOT EXISTS (SELECT 1 FROM cairn_whos_here_directory() WHERE staff_id = 'nurse-a'
                    AND display_name = 'Nurse A' AND role = 'nurse') THEN
        RAISE EXCEPTION 'the directory carries the name and the role';
    END IF;
END $$;

-- the_3a_doors_are_gone (a door's return shape is final, so 3a's three were removed, not reshaped).
DO $$
BEGIN
    IF to_regprocedure('cairn_note_draft_save(uuid,uuid,text,text,timestamptz)') IS NOT NULL
       OR to_regprocedure('cairn_note_drafts(uuid)') IS NOT NULL
       OR to_regprocedure('cairn_note_draft_spend(uuid,uuid)') IS NOT NULL THEN
        RAISE EXCEPTION 'the_3a_doors_are_gone: a 3a draft door still exists';
    END IF;
END $$;

-- no_table_grant, and the doors are the window's and nobody else's.
DO $$
DECLARE f text;
BEGIN
    IF has_table_privilege('cairn_window', 'note_draft_part', 'SELECT')
       OR has_table_privilege('cairn_window', 'note_draft_revision', 'SELECT')
       OR has_table_privilege('cairn_window', 'note_draft_part', 'INSERT')
       OR has_table_privilege('cairn_window', 'note_draft_revision', 'INSERT') THEN
        RAISE EXCEPTION 'no_table_grant: the window can reach a part table';
    END IF;
    FOREACH f IN ARRAY ARRAY[
        'cairn_note_draft_part_save(uuid, uuid, text, text, text, timestamptz)',
        'cairn_note_draft_parts(uuid)', 'cairn_note_draft_take(uuid, uuid, text)',
        'cairn_note_draft_spend_parts(uuid, uuid)', 'cairn_whos_here_directory()'] LOOP
        IF has_function_privilege('public', f, 'EXECUTE') THEN RAISE EXCEPTION '% must not be PUBLIC', f; END IF;
        IF NOT has_function_privilege('cairn_window', f, 'EXECUTE') THEN RAISE EXCEPTION '% is the window''s', f; END IF;
        IF NOT (SELECT prosecdef AND proconfig @> ARRAY['search_path=public, pg_temp'] FROM pg_proc
                 WHERE oid = f::regprocedure) THEN
            RAISE EXCEPTION '% must be definer-run with a pinned search_path', f;
        END IF;
    END LOOP;
END $$;

-- The window's own role meets the same refusal reading a part table directly.
CREATE ROLE note_draft_parts_probe NOLOGIN;
GRANT cairn_window TO note_draft_parts_probe;
SET LOCAL ROLE note_draft_parts_probe;
DO $$
BEGIN
    PERFORM 1 FROM note_draft_part;
    RAISE EXCEPTION 'probe read note_draft_part';
EXCEPTION WHEN insufficient_privilege THEN NULL;
END $$;
DO $$
BEGIN
    PERFORM 1 FROM note_draft_revision;
    RAISE EXCEPTION 'probe read note_draft_revision';
EXCEPTION WHEN insufficient_privilege THEN NULL;
END $$;
RESET ROLE;

ROLLBACK;
