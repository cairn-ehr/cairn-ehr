-- SQL mirror of crates/cairn-node/tests/sensitivity_* (see db/tests/README.md).
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
CREATE OR REPLACE FUNCTION _sensitivity_seed_event(
    p_patient uuid, p_type text, p_body jsonb, p_wall bigint
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_id    uuid := gen_random_uuid();
    v_bytes bytea := convert_to(v_id::text || p_type, 'UTF8');
BEGIN
    INSERT INTO event_log (
        event_id, patient_id, event_type, schema_version,
        hlc_wall, hlc_counter, node_origin,
        signed_bytes, content_address, body, contributors,
        signer_key_id, plaintext_twin)
    VALUES (
        v_id, p_patient, p_type, p_type || '/1',
        p_wall, 0, 'test-node',
        v_bytes, '\x1220'::bytea || digest(v_bytes, 'sha256'),
        p_body, '[]'::jsonb,
        'k', 'twin');
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
DO $$
DECLARE
    p      uuid := gen_random_uuid();
    target uuid;
    ca_hex text;
    got_grade text;
    still  int;
BEGIN
    target := _sensitivity_seed_event(p, 'note.added', jsonb_build_object('text', 'n'), 10);
    PERFORM _sensitivity_seed_event(p, 'sensitivity.grade.asserted',
        jsonb_build_object('subject_kind', 'patient', 'subject_id', p::text,
                            'grade', 'sequestered', 'source', 'human'), 11);

    SELECT encode(content_address, 'hex') INTO ca_hex
        FROM sensitivity_assertion WHERE patient_id = p;

    PERFORM _sensitivity_seed_event(p, 'sensitivity.grade-withdrawal.asserted',
        jsonb_build_object('withdraws', ca_hex, 'rationale', 'patient consent'), 12);

    SELECT grade INTO got_grade FROM cairn_effective_sensitivity(target);
    IF got_grade <> 'routine' THEN
        RAISE EXCEPTION 'FAIL: the withdrawn assertion must no longer stand, got %', got_grade;
    END IF;

    SELECT count(*) INTO still FROM sensitivity_assertion WHERE patient_id = p;
    IF still <> 1 THEN
        RAISE EXCEPTION 'FAIL: declassification is an overlay, never an erasure — expected 1 assertion row, got %', still;
    END IF;
END $$;

DROP FUNCTION _sensitivity_seed_event(uuid, text, jsonb, bigint);

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

    -- The function authors nothing: policy decides whether a candidate becomes an event.
    SELECT count(*) INTO n FROM event_log WHERE event_type = 'sensitivity.grade.asserted';
    ASSERT n = 0, 'the lookup must never author an assertion by itself';

    DELETE FROM sensitivity_category_map WHERE category = 'sti-screen';
END $$;
