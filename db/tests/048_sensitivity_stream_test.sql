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
