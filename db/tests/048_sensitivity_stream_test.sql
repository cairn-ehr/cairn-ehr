-- SQL mirror of crates/cairn-node/tests/sensitivity_* (run by scripts/run-db-sql-tests.sh;
-- the disposable-database rule these mirrors share is in _scratch_database_guard.sql).
-- DESTRUCTIVE: runs only against a database marked disposable (#169).
\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql

DO $$
BEGIN
    ASSERT cairn_sensitivity_rank('routine') = 0, 'routine ranks 0';
    ASSERT cairn_sensitivity_rank('sensitive') < cairn_sensitivity_rank('restricted'),
        'the ladder is ordered';
    ASSERT cairn_sensitivity_rank('restricted') < cairn_sensitivity_rank('sequestered'),
        'the ladder is ordered';
    ASSERT cairn_sensitivity_rank('grade:protected-witness') = 2147483647,
        'an unrecognised grade ranks MAX (inverting db/040 deliberately — ADR-0062)';
    ASSERT cairn_sensitivity_rank(NULL) = 2147483647, 'NULL lands on the safe side';
END $$;

-- ---------------------------------------------------------------------------
-- SQL mirror of crates/cairn-node/tests/sensitivity_ladder.rs's
-- the_effective_grade_is_the_max_over_event_thread_and_chart and
-- a_withdrawal_lowers_the_effective_grade_and_the_assertion_survives.
--
-- WHY NOT submit_event: that door needs a real Ed25519-signed envelope, and this rig has
-- no signing key (same limitation db/tests/047's header explains for its own case). What
-- IS checkable with plain SQL is the projection + read-side logic under test here — the
-- floor's structural checks were already pinned in Rust (sensitivity_floor.rs) and the
-- ladder itself just above. So this mirror seeds event_log rows DIRECTLY (bypassing
-- submit_event, exactly as db/tests/008_surrogate_test.sql's _b5_seed_event and
-- db/tests/047's block 2 already do) — a plain INSERT into event_log still fires the real
-- cairn_projection_dispatch_trg (db/005) AFTER INSERT trigger, so sensitivity_assertion_apply
-- and sensitivity_withdrawal_apply run through their REAL path, not a hand-rolled stand-in.
--
-- Runs inside a transaction that ROLLBACKs, so it leaves no residue (same discipline as
-- db/tests/045 and db/tests/047), even though 048 is currently the last file the runner loads.
BEGIN;

-- Seed one event_log row of the given type/body, patient-attributed, at the given HLC wall
-- clock. Returns the new event's id so the caller can name it in a later assertion (the
-- sensitivity apply functions and cairn_effective_sensitivity both key off event_id/patient_id,
-- never off anything submit_event alone would add). content_address satisfies db/001's CHECK
-- ('\x1220' || sha256(signed_bytes)) with signed_bytes standing in for the real signed envelope
-- — its actual bytes are never verified off this path, only its hash-derived address.
--
-- p_attester_key (#380, ADR-0064) is OPTIONAL and NULL by default: every pre-existing call
-- below seeds a bare event with no attester, which is exactly what `cairn_claim_authority`
-- must read as 'unverified' — R1 needs `attester_key IS NOT NULL` and R2 needs both rows'
-- actor_id populated, neither of which this bare seed sets. A caller seeding a WITHDRAWAL
-- that must actually lower the grade (block 2 below) passes an enrolled human's key here so
-- the seeded row carries a real, vouched (no unvouched marker exists for it) attestation —
-- the raw-SQL-rig equivalent of what `apply_remote_attested` verifies cryptographically in
-- the Rust suite.
CREATE OR REPLACE FUNCTION _sensitivity_seed_event(
    p_patient uuid, p_type text, p_body jsonb, p_wall bigint,
    p_attester_key bytea DEFAULT NULL
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_id    uuid := gen_random_uuid();
    v_bytes bytea := convert_to(v_id::text || p_type, 'UTF8');
BEGIN
    INSERT INTO event_log (
        event_id, patient_id, event_type, schema_version,
        hlc_wall, hlc_counter, node_origin,
        signed_bytes, content_address, body, contributors,
        signer_key_id, plaintext_twin, attester_key)
    VALUES (
        v_id, p_patient, p_type, p_type || '/1',
        p_wall, 0, 'test-node',
        v_bytes, '\x1220'::bytea || digest(v_bytes, 'sha256'),
        p_body, '[]'::jsonb,
        'k', 'twin', p_attester_key);
    RETURN v_id;
END;
$$;

-- 1. Max-over-three-subjects: no assertion -> routine; a chart-wide grade reaches an event
--    with none of its own; an event-scoped grade outranks the chart-wide one, and the
--    winning subject_kind is named. Mirrors the Rust test of the same shape.
DO $$
DECLARE
    p       uuid := gen_random_uuid();
    target  uuid;
    got_grade text;
    got_kind  text;
BEGIN
    target := _sensitivity_seed_event(p, 'note.added', jsonb_build_object('text', 'routine note'), 10);

    SELECT grade, subject_kind INTO got_grade, got_kind
        FROM cairn_effective_sensitivity(target);
    IF got_grade <> 'routine' THEN
        RAISE EXCEPTION 'FAIL: absence of assertions must read as routine, got %', got_grade;
    END IF;

    PERFORM _sensitivity_seed_event(p, 'sensitivity.grade.asserted',
        jsonb_build_object('subject_kind', 'patient', 'subject_id', p::text,
                            'grade', 'sensitive', 'source', 'human'), 11);
    SELECT grade, subject_kind INTO got_grade, got_kind
        FROM cairn_effective_sensitivity(target);
    IF (got_grade, got_kind) IS DISTINCT FROM ('sensitive', 'patient') THEN
        RAISE EXCEPTION 'FAIL: a chart-wide grade must reach an event carrying none of its own, got (%, %)',
            got_grade, got_kind;
    END IF;

    PERFORM _sensitivity_seed_event(p, 'sensitivity.grade.asserted',
        jsonb_build_object('subject_kind', 'event', 'subject_id', target::text,
                            'grade', 'restricted', 'source', 'human'), 12);
    SELECT grade, subject_kind INTO got_grade, got_kind
        FROM cairn_effective_sensitivity(target);
    IF (got_grade, got_kind) IS DISTINCT FROM ('restricted', 'event') THEN
        RAISE EXCEPTION 'FAIL: an event-scoped grade must outrank the chart-wide one, got (%, %)',
            got_grade, got_kind;
    END IF;
END $$;

-- 2. Withdrawal: lowers the effective grade back to routine, and the withdrawn assertion is
--    NOT erased — it stays on the record, still re-assertable (never merge, always overlay).
--
--    #380/ADR-0064: since cairn_sensitivity_standing now consults cairn_claim_authority, the
--    withdrawal seeded here must be AUTHORITATIVE or it will not lower anything (that is the
--    behaviour block 2b right below this one pins). This rig has no signing key (see the file
--    header), so "attested" is simulated the same way the seed itself is: enroll a human actor
--    directly via enroll_actor() with a runtime-random key (house rule 6 — no literal crypto
--    material), then stamp that key as the withdrawal's attester_key. cairn_attestation_vouched
--    reads as vacuously TRUE (no event_attestation_unvouched row exists for a seeded event), so
--    R1 is satisfied exactly like a real vouched attestation would be.
DO $$
DECLARE
    p           uuid := gen_random_uuid();
    target      uuid;
    ca_hex      text;
    got_grade   text;
    still       int;
    human_key   text := encode(gen_random_bytes(32), 'hex');
    human_key_b bytea := decode(human_key, 'hex');
BEGIN
    target := _sensitivity_seed_event(p, 'note.added', jsonb_build_object('text', 'n'), 10);
    PERFORM _sensitivity_seed_event(p, 'sensitivity.grade.asserted',
        jsonb_build_object('subject_kind', 'patient', 'subject_id', p::text,
                            'grade', 'sequestered', 'source', 'human'), 11);

    SELECT encode(content_address, 'hex') INTO ca_hex
        FROM sensitivity_assertion WHERE patient_id = p;

    PERFORM enroll_actor('human', jsonb_build_object('role', 'test-witness'), human_key);
    PERFORM _sensitivity_seed_event(p, 'sensitivity.grade-withdrawal.asserted',
        jsonb_build_object('withdraws', ca_hex, 'rationale', 'patient consent'), 12,
        human_key_b);

    SELECT grade INTO got_grade FROM cairn_effective_sensitivity(target);
    IF got_grade <> 'routine' THEN
        RAISE EXCEPTION 'FAIL: the withdrawn assertion must no longer stand, got %', got_grade;
    END IF;

    SELECT count(*) INTO still FROM sensitivity_assertion WHERE patient_id = p;
    IF still <> 1 THEN
        RAISE EXCEPTION 'FAIL: declassification is an overlay, never an erasure — expected 1 assertion row, got %', still;
    END IF;
END $$;

-- 2b. #380/ADR-0064's OWN mirror: the same withdrawal, but with NO attester_key at all — the
--     un-attested, un-vouched shape a peer write with no responsibility claim carries. It must
--     still LAND (nothing is refused at either door for this rule) but must NOT lower the grade.
--     Mirrors crates/cairn-node/tests/claim_authority.rs's
--     an_unattested_withdrawal_lands_and_converges_but_does_not_lower.
DO $$
DECLARE
    p         uuid := gen_random_uuid();
    target    uuid;
    ca_hex    text;
    got_grade text;
    landed    int;
BEGIN
    target := _sensitivity_seed_event(p, 'note.added', jsonb_build_object('text', 'n'), 10);
    PERFORM _sensitivity_seed_event(p, 'sensitivity.grade.asserted',
        jsonb_build_object('subject_kind', 'patient', 'subject_id', p::text,
                            'grade', 'sequestered', 'source', 'human'), 11);

    SELECT encode(content_address, 'hex') INTO ca_hex
        FROM sensitivity_assertion WHERE patient_id = p;

    -- No p_attester_key argument: this row's attester_key stays NULL, exactly the
    -- 'unverified' shape cairn_claim_authority must never grade as authoritative.
    PERFORM _sensitivity_seed_event(p, 'sensitivity.grade-withdrawal.asserted',
        jsonb_build_object('withdraws', ca_hex, 'rationale', 'strip it'), 12);

    SELECT count(*) INTO landed FROM sensitivity_withdrawal WHERE patient_id = p;
    IF landed <> 1 THEN
        RAISE EXCEPTION 'FAIL: an un-attested withdrawal must still land and converge, got % rows', landed;
    END IF;

    SELECT grade INTO got_grade FROM cairn_effective_sensitivity(target);
    IF got_grade <> 'sequestered' THEN
        RAISE EXCEPTION 'FAIL: an un-attested withdrawal must not lower the grade (#380), got %', got_grade;
    END IF;
END $$;

DROP FUNCTION _sensitivity_seed_event(uuid, text, jsonb, bigint, bytea);

ROLLBACK;

-- ---------------------------------------------------------------------------
-- SQL mirror of crates/cairn-node/tests/claim_authority.rs and
-- claim_authority_worklist.rs (#380, ADR-0064). Block 2b just above already exercises the
-- REAL event_log/projection path for an unattested withdrawal (an unattested withdrawal
-- lands but does not lower cairn_effective_sensitivity's answer) — this block is narrower
-- and complementary, not a restatement: it drives cairn_claim_authority and
-- cairn_sensitivity_standing directly, and it is the ONLY place in db/tests that touches
-- sensitivity_withdrawal_worklist at all.
--
-- Seeds sensitivity_assertion/sensitivity_withdrawal DIRECTLY rather than through
-- _sensitivity_seed_event: cairn_claim_authority only ever reads event_log, and no
-- event_log row exists for either v_assert or v_withdraw here, so both R1 (needs
-- attester_key IS NOT NULL) and R2 (needs both rows' actor_id populated) fail on an empty
-- EXISTS — the deliberately unresolvable shape the first assertion below names. Runs
-- autocommitting, outside a transaction, so cleanup at the end is explicit — same
-- discipline as the category-blacklist block below.
DO $$
DECLARE
    v_patient  uuid := gen_random_uuid();
    v_assert   uuid := gen_random_uuid();
    v_withdraw uuid := gen_random_uuid();
    v_ca_a     bytea := '\x1220'::bytea || digest('authority-mirror-assert', 'sha256');
    v_ca_w     bytea := '\x1220'::bytea || digest('authority-mirror-withdraw', 'sha256');
    n          int;
BEGIN
    INSERT INTO sensitivity_assertion
        (content_address, event_id, patient_id, subject_kind, subject_id, grade, source,
         hlc_wall, hlc_counter, node_origin)
    VALUES (v_ca_a, v_assert, v_patient, 'patient', v_patient, 'sequestered', 'human',
            10, 0, 'mirror');
    INSERT INTO sensitivity_withdrawal
        (content_address, event_id, withdraws, patient_id, rationale,
         hlc_wall, hlc_counter, node_origin)
    VALUES (v_ca_w, v_withdraw, v_ca_a, v_patient, 'strip', 20, 0, 'mirror');

    -- No event_log rows exist for either id, so neither R1 nor R2 can be satisfied.
    ASSERT cairn_claim_authority(v_withdraw, v_assert) = 'unverified',
        'a withdrawal with no resolvable human behind it is unverified';

    SELECT count(*) INTO n FROM cairn_sensitivity_standing(v_patient);
    ASSERT n = 1,
        'the assertion still STANDS: an unverified withdrawal does not lower (ADR-0064/#380)';

    -- And it is on the worklist, as `inert`.
    SELECT count(*) INTO n FROM sensitivity_withdrawal_worklist
     WHERE patient_id = v_patient AND reason = 'inert';
    ASSERT n = 1, 'an inert withdrawal is listed';

    DELETE FROM sensitivity_withdrawal WHERE content_address = v_ca_w;
    DELETE FROM sensitivity_assertion  WHERE content_address = v_ca_a;
END $$;

-- ---------------------------------------------------------------------------
-- #410 review finding C3: the SQL mirror of "an unrecognised verdict withholds".
--
-- The seam tests `cairn_claim_authority(...) IN ('attested','self')`, POSITIVELY. Written
-- the other way — `<> 'unverified'` — every verdict that is not byte-for-byte that string
-- gains the power to strip a grade, including a FOURTH verdict some future ADR adds. That
-- is not a hostile scenario: ADR-0064 says "every future dial" will delegate here.
--
-- The real definition is captured with pg_get_functiondef and replayed to restore, rather
-- than hand-copied, so this test cannot drift away from the thing it restores (the same
-- reason the Rust twin replays db/005 from the migration file itself). The Rust twin is
-- claim_authority.rs's `an_unrecognised_verdict_withholds_the_power_to_strip`.
DO $$
DECLARE
    v_patient  uuid := gen_random_uuid();
    v_assert   uuid := gen_random_uuid();
    v_withdraw uuid := gen_random_uuid();
    v_ca_a     bytea := '\x1220'::bytea || digest('c3-mirror-assert', 'sha256');
    v_ca_w     bytea := '\x1220'::bytea || digest('c3-mirror-withdraw', 'sha256');
    v_real_def text;
    n          int;
BEGIN
    v_real_def := pg_get_functiondef('cairn_claim_authority(uuid,uuid)'::regprocedure);

    INSERT INTO sensitivity_assertion
        (content_address, event_id, patient_id, subject_kind, subject_id, grade, source,
         hlc_wall, hlc_counter, node_origin)
    VALUES (v_ca_a, v_assert, v_patient, 'patient', v_patient, 'sequestered', 'human',
            10, 0, 'mirror');
    INSERT INTO sensitivity_withdrawal
        (content_address, event_id, withdraws, patient_id, rationale,
         hlc_wall, hlc_counter, node_origin)
    VALUES (v_ca_w, v_withdraw, v_ca_a, v_patient, 'strip', 20, 0, 'mirror');

    -- Stage a verdict that exists in no ladder today.
    EXECUTE $future$
        CREATE OR REPLACE FUNCTION cairn_claim_authority(p_event_id uuid, p_target_event_id uuid)
        RETURNS text LANGUAGE sql STABLE
        SECURITY DEFINER SET search_path = public, pg_temp
        AS 'SELECT ''delegated-registry''::text';
    $future$;

    SELECT count(*) INTO n FROM cairn_sensitivity_standing(v_patient);

    -- Restore BEFORE asserting, so a failure cannot leave a stub predicate behind that
    -- would silently disarm every later block in this file.
    EXECUTE v_real_def;

    ASSERT n = 1,
        'an UNRECOGNISED verdict must withhold the power to strip: the assertion must '
        'still stand (#410 finding C3 — a negative <> test would strip it instead)';

    -- And the restore genuinely put the real predicate back.
    ASSERT cairn_claim_authority(v_withdraw, v_assert) = 'unverified',
        'the real predicate must be restored after the staged fourth verdict';

    DELETE FROM sensitivity_withdrawal WHERE content_address = v_ca_w;
    DELETE FROM sensitivity_assertion  WHERE content_address = v_ca_a;
END $$;

-- ---------------------------------------------------------------------------
-- Task 4 review gap (not itself in the task-6 brief): nothing pins
-- sensitivity_withdrawal_worklist's column set, order or types.
--
-- Postgres itself already refuses a CREATE OR REPLACE VIEW that drops, retypes or
-- reorders an EXISTING output column (verified empirically against this exact view while
-- writing this test: each attempt errors "cannot change data type/name of view column").
-- What it does NOT catch: silently APPENDING a new trailing column (still legal under
-- CREATE OR REPLACE, and still a contract change nobody would notice); ALTER VIEW ...
-- RENAME COLUMN, a completely separate statement with none of CREATE OR REPLACE's
-- protections (also verified — it renamed a live column here with no error); and any
-- future full rewrite (DROP VIEW + CREATE VIEW), which faces none of the above
-- restrictions at all. Migration replay on a long-lived developer database would carry
-- any of those forward with nothing catching it. Pinned structurally against
-- information_schema, rather than against any one row's shape, which a projection
-- default could satisfy by accident regardless of whether the contract actually held.
DO $$
DECLARE
    expected_cols  text[] := ARRAY['content_address', 'event_id', 'patient_id', 'withdraws',
                                    'reason', 'node_origin', 'rationale'];
    expected_types text[] := ARRAY['bytea', 'uuid', 'uuid', 'bytea', 'text', 'text', 'text'];
    got_cols  text[];
    got_types text[];
BEGIN
    SELECT array_agg(column_name ORDER BY ordinal_position),
           array_agg(data_type ORDER BY ordinal_position)
      INTO got_cols, got_types
      FROM information_schema.columns
     WHERE table_schema = 'public' AND table_name = 'sensitivity_withdrawal_worklist';

    ASSERT got_cols = expected_cols,
        format('sensitivity_withdrawal_worklist column set/order drifted from the pinned '
               '7-column contract (content_address, event_id, patient_id, withdraws, '
               'reason, node_origin, rationale): got %s', got_cols);
    ASSERT got_types = expected_types,
        format('sensitivity_withdrawal_worklist column types drifted from the pinned '
               'contract: got %s', got_types);
END $$;

-- ---------------------------------------------------------------------------
-- ADR-0064's privilege posture for cairn_claim_authority. No db/tests/005_* mirror has a
-- function-inventory section to extend (checked: db/tests/005_submit_test.sql covers only
-- C5.1/C5.4), so per the brief's own fallback this lives here, beside the function's own
-- read-path tests above. crates/cairn-node/tests/safety_ladder.rs already pins the SIBLING
-- posture for cairn_record_safety_overclaim_flag; this is the complement, covering the one
-- function that Rust suite never touches.
--
-- The two postures are NOT the same shape, and this comment used to conflate them (#410
-- review finding A4). `cairn_record_safety_overclaim_flag` is a plain `LANGUAGE sql` writer
-- — NOT security definer — REVOKEd from PUBLIC and given no cairn_agent grant at all,
-- because submit_event calls it as its owner. `cairn_claim_authority` is the only
-- SECURITY DEFINER function this slice added, and it IS granted to cairn_agent because the
-- product's read path calls it directly. Same file, opposite grant posture, for opposite
-- reasons — which is exactly why both are pinned rather than assumed.
DO $$
BEGIN
    ASSERT NOT has_function_privilege('public', 'cairn_claim_authority(uuid,uuid)', 'EXECUTE'),
        'cairn_claim_authority is SECURITY DEFINER — PUBLIC must not hold EXECUTE';
    ASSERT has_function_privilege('cairn_agent', 'cairn_claim_authority(uuid,uuid)', 'EXECUTE'),
        'cairn_agent reads the effective grade and therefore needs EXECUTE';
    ASSERT (SELECT prosecdef FROM pg_proc WHERE proname = 'cairn_claim_authority'),
        'SECURITY DEFINER is load-bearing: cairn_attestation_vouched is REVOKEd from PUBLIC';
END $$;

-- ---------------------------------------------------------------------------
-- The category blacklist (Task 7, issue #232 part A continued): the AUTOMATIC tagging
-- source. Runs as its own top-level, autocommitting block, deliberately OUTSIDE the
-- BEGIN/ROLLBACK transaction above — that transaction's own seeded 'sensitivity.grade.asserted'
-- rows are still live inside it until the ROLLBACK unwinds them, so a block sharing that
-- transaction would see non-zero event_log rows of that type and the "authors nothing"
-- assertion below would be testing stale state, not a clean one. Being outside it also means
-- this block's own writes are real, hence the explicit DELETE at the end (note 4): these
-- mirrors share one throwaway database across the whole file list, so residue here would
-- leak into whatever mirror runs next.
DO $$
DECLARE r record; n int;
BEGIN
    -- Ships EMPTY. Cairn provides the lookup mechanism, never the list (ADR-0006 §3).
    SELECT count(*) INTO n FROM sensitivity_category_map;
    ASSERT n = 0, 'the category map ships empty — the list is deployment configuration';

    SELECT count(*) INTO n FROM cairn_sensitivity_candidate('{"category":"sti-screen"}'::jsonb);
    ASSERT n = 0, 'an unmapped category yields no candidate';

    INSERT INTO sensitivity_category_map (category, grade, note)
    VALUES ('sti-screen', 'restricted', 'test fixture');

    SELECT * INTO r FROM cairn_sensitivity_candidate('{"category":"sti-screen"}'::jsonb);
    ASSERT r.grade = 'restricted', 'a mapped category yields its grade';
    ASSERT r.category = 'sti-screen', 'and names what matched, for LOCAL audit only';

    -- THE SUBJECT IS NEVER THE PATIENT. The return shape has no patient/subject column at
    -- all, so a coded hit on one drug cannot express a chart-wide candidate even by accident
    -- — that is the whole point of section 13, and it is a property of the shape rather than
    -- of any value, so assert it against the catalog rather than by inspecting a row.
    SELECT count(*) INTO n
      FROM information_schema.routines rt
      JOIN information_schema.parameters pa
        ON pa.specific_name = rt.specific_name
     WHERE rt.routine_name = 'cairn_sensitivity_candidate'
       AND pa.parameter_mode = 'OUT'
       AND pa.parameter_name IN ('patient_id', 'subject_id', 'subject_kind');
    ASSERT n = 0,
        'cairn_sensitivity_candidate must have no patient/subject output column — a coded hit '
        'must not be able to express a chart-wide candidate';

    DELETE FROM sensitivity_category_map WHERE category = 'sti-screen';
END $$;

-- ---------------------------------------------------------------------------
-- The section 10b TYPE GATE (`cairn_event_type_has_no_thread`), which decides whether an
-- event whose thread cannot be resolved takes the conservative bound.
--
-- Only ONE of its six prefixes was behaviourally exercised anywhere, and the deliberate
-- "an unrecognised type returns FALSE so it KEEPS the bound" ruling was not exercised at all.
-- Two concrete defects that used to pass everything: dropping `identity.%` (which silently
-- makes a chart's whole report read as its most-graded thread, because the chart-wide reading
-- resolves off the registration event), and inverting the function into a whitelist of types
-- that DO have threads (which keeps every current test green while removing the bound from
-- every FUTURE clinical stream — the disclosure direction, found years later).
DO $$
DECLARE t text;
BEGIN
    -- TRUE: types this version has positively confirmed cannot be on a medication thread.
    FOREACH t IN ARRAY ARRAY[
        'demographic.name.asserted', 'identity.registration.asserted', 'note.added',
        'patient.merged', 'sensitivity.grade.asserted', 'erasure.shred.asserted'
    ] LOOP
        ASSERT cairn_event_type_has_no_thread(t),
            format('%s structurally cannot carry a medication thread, so it must NOT take '
                   'the unresolved-thread bound', t);
    END LOOP;

    -- FALSE: medication's own namespace, and — the load-bearing half — a type this version
    -- has never heard of. A future clinical stream inherits the bound for free by simply not
    -- appearing in the list above; nobody has to remember to add it.
    FOREACH t IN ARRAY ARRAY[
        'clinical.medication.asserted', 'clinical.medication-dose-change.asserted',
        'lab.result.asserted', 'imaging.study.asserted'
    ] LOOP
        ASSERT NOT cairn_event_type_has_no_thread(t),
            format('%s must keep the conservative bound — unknown must coarsen, never expose', t);
    END LOOP;
END $$;

-- ---------------------------------------------------------------------------
-- `cairn_thread_patient` degrades to NULL rather than raising when it cannot answer.
-- On the cairn-sync subset the medication projections do not exist at all; here they DO, so
-- this pins the other NULL cause — a thread this node has simply never seen. Callers depend
-- on that NULL meaning "cannot tell" (never "wrong chart"), because treating it as a
-- mis-target would fire on every not-yet-replicated thread and, on a custody-less node where
-- medication_statement is empty for every thread, on all of them at once.
DO $$
DECLARE v uuid;
BEGIN
    SELECT cairn_thread_patient('00000000-0000-0000-0000-0000000000ff'::uuid) INTO v;
    ASSERT v IS NULL, 'an unknown thread must read as "cannot tell", not raise';
END $$;
