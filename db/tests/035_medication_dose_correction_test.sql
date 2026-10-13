\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/tests/035_medication_dose_correction_test.sql
-- #346 — the SQL mirror of db/035's dose-correction `strike` rule, for the case the Rust suite
-- cannot reach.
--
-- WHY THIS FILE EXISTS. #346 audited every registered floor check for one three-valued-logic
-- trap: a refusal written as `IF <predicate over a maybe-NULL value> THEN RAISE` does not fire
-- when the predicate is NULL, so the check silently ADMITS the input it was written to refuse,
-- permanently, into an append-only record. One instance survived, here:
--
--     WHERE g NOT IN ('dose', 'effective', 'reason')
--
-- over `jsonb_array_elements_text(p -> 'strike')`. A JSON `null` element yields g = NULL, and
-- `NULL NOT IN (...)` is NULL, so the bad element was never selected and `strike: [null]` passed.
-- It then ALSO satisfied the principle-4 floor "must set or strike at least one group", because
-- that rule counts the array's length — so a correction that changes nothing was admitted.
--
-- WHY SQL AND NOT RUST. Every Rust caller builds `strike` from `&[&str]`, so no builder can emit a
-- null element; only a hand-built body (any peer, or a raw-SQL client — principle 12) can. These
-- blocks call `cairn_check_medication_dose` directly, exactly as db/tests/045 does for its floor.
--
-- Runs inside a transaction that ROLLBACKs, so it leaves no residue. Picked up automatically:
-- scripts/run-db-sql-tests.sh globs db/tests/[0-9]*.sql.
BEGIN;

-- Each block re-raises its own 'FAIL:' message unchanged before inspecting the caught text;
-- without that, `WHEN others` swallows the assertion it was meant to report (db/tests/045's rule).

-- 1. The defect: a strike list whose only element is JSON null is refused, not admitted.
DO $$
BEGIN
    PERFORM cairn_check_medication_dose('clinical.medication-dose-correction.asserted',
        jsonb_build_object('payload', jsonb_build_object(
            'medication_id', '0190e0c2-0000-7000-8000-000000000001',
            'corrects',      '0190e0c2-0000-7000-8000-000000000002',
            'strike',        jsonb_build_array(NULL::text))));
    RAISE EXCEPTION 'FAIL: a strike list of [null] was admitted — a correction that strikes nothing (#346)';
EXCEPTION WHEN others THEN
    IF position('FAIL:' in SQLERRM) = 1 THEN RAISE; END IF;
    IF position('strike may only contain' in SQLERRM) = 0 THEN
        RAISE EXCEPTION 'FAIL: wrong refusal for strike [null]: %', SQLERRM;
    END IF;
END $$;

-- 2. A null element beside a valid one is refused too: the rule is "only these names", and null
--    is not one of them. (Admitting it would not change what is struck, but a floor that admits
--    a value it cannot name is the same trap in a milder costume.)
DO $$
BEGIN
    PERFORM cairn_check_medication_dose('clinical.medication-dose-correction.asserted',
        jsonb_build_object('payload', jsonb_build_object(
            'medication_id', '0190e0c2-0000-7000-8000-000000000001',
            'corrects',      '0190e0c2-0000-7000-8000-000000000002',
            'strike',        jsonb_build_array('dose', NULL::text))));
    RAISE EXCEPTION 'FAIL: a strike list carrying a null beside "dose" was admitted (#346)';
EXCEPTION WHEN others THEN
    IF position('FAIL:' in SQLERRM) = 1 THEN RAISE; END IF;
    IF position('strike may only contain' in SQLERRM) = 0 THEN
        RAISE EXCEPTION 'FAIL: wrong refusal for strike ["dose", null]: %', SQLERRM;
    END IF;
END $$;

-- 3. The POSITIVE CONTROL: a well-formed strike is still admitted. Without it, a fix that refused
--    every strike would turn blocks 1 and 2 green while breaking "set this to unknown" outright.
DO $$
BEGIN
    PERFORM cairn_check_medication_dose('clinical.medication-dose-correction.asserted',
        jsonb_build_object('payload', jsonb_build_object(
            'medication_id', '0190e0c2-0000-7000-8000-000000000001',
            'corrects',      '0190e0c2-0000-7000-8000-000000000002',
            'strike',        jsonb_build_array('dose', 'effective'))));
EXCEPTION WHEN others THEN
    RAISE EXCEPTION 'FAIL: a well-formed strike ["dose","effective"] was refused: %', SQLERRM;
END $$;

ROLLBACK;
