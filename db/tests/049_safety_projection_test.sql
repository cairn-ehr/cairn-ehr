-- SQL mirror of crates/cairn-node/tests/safety_* (run by scripts/run-db-sql-tests.sh;
-- the disposable-database rule these mirrors share is in _scratch_database_guard.sql).
-- DESTRUCTIVE: runs only against a database marked disposable (#169).
\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql

-- ---------------------------------------------------------------------------
-- 1. Both ladders, and the monotone rung map. Mirrors safety_ladder.rs.
-- ---------------------------------------------------------------------------
DO $$
BEGIN
    ASSERT cairn_safety_severity_rank('none') = 0, 'none is the floor';
    ASSERT cairn_safety_severity_rank('low') < cairn_safety_severity_rank('critical'),
        'the severity ladder is ordered';
    ASSERT cairn_safety_severity_rank('severity:novel') = 2147483647,
        'an unrecognised severity ranks MAX — assume the worst (ADR-0063)';
    ASSERT cairn_safety_severity_rank(NULL) = 2147483647, 'NULL lands on the safe side';

    ASSERT cairn_safety_rung_rank('precise') < cairn_safety_rung_rank('kind'),
        'the rung ladder is ordered coarsest-last';
    ASSERT cairn_safety_rung_rank('kind') < cairn_safety_rung_rank('existence'),
        'the rung ladder is ordered coarsest-last';
    ASSERT cairn_safety_rung_rank('rung:novel') = 2147483647,
        'an unrecognised rung is treated as coarsest, never as show-everything';

    ASSERT cairn_safety_rung_for_rank(cairn_sensitivity_rank('routine')) = 'precise',
        'no standing grade discloses fully';
    ASSERT cairn_safety_rung_for_rank(cairn_sensitivity_rank('sensitive')) = 'kind';
    ASSERT cairn_safety_rung_for_rank(cairn_sensitivity_rank('restricted')) = 'existence';
    ASSERT cairn_safety_rung_for_rank(cairn_sensitivity_rank('sequestered')) = 'existence';
    ASSERT cairn_safety_rung_for_rank(cairn_sensitivity_rank('grade:future')) = 'existence',
        'an unrecognised grade ranks MAX (ADR-0062), hence coarsest here';
    ASSERT cairn_safety_rung_for_rank(NULL) = 'existence', 'no answer ⇒ disclose nothing';
END $$;

-- Monotonicity across the whole ladder, as a set: a higher grade may never disclose more.
DO $$
DECLARE v_bad int;
BEGIN
    SELECT count(*) INTO v_bad
    FROM (
        SELECT r, cairn_safety_rung_rank(cairn_safety_rung_for_rank(r)) AS rung_rank,
               lag(cairn_safety_rung_rank(cairn_safety_rung_for_rank(r)))
                   OVER (ORDER BY r) AS prev
        FROM unnest(ARRAY[0, 5, 10, 15, 20, 30, 2147483647]) AS r
    ) t
    WHERE prev IS NOT NULL AND rung_rank < prev;
    ASSERT v_bad = 0, 'the rung map must be monotone non-decreasing in grade rank';
END $$;

-- ---------------------------------------------------------------------------
-- 2. The structural floor. Mirrors safety_ladder.rs's floor tests.
-- ---------------------------------------------------------------------------
DO $$
DECLARE v_msg text;
BEGIN
    -- Admitted shapes.
    PERFORM cairn_check_safety_signal('{}'::jsonb);
    PERFORM cairn_check_safety_signal('{"safety":{"rung":"precise","class":"c","severity":"high"}}'::jsonb);
    PERFORM cairn_check_safety_signal('{"safety":{"rung":"kind","severity":"high"}}'::jsonb);
    PERFORM cairn_check_safety_signal('{"safety":{"rung":"existence"}}'::jsonb);
    -- A future peer's rung is ADMITTED: the floor gates effect, not presence (ADR-0056).
    PERFORM cairn_check_safety_signal('{"safety":{"rung":"rung:novel"}}'::jsonb);

    -- The disclosure guard: a class the rung does not license.
    BEGIN
        PERFORM cairn_check_safety_signal('{"safety":{"rung":"existence","class":"c"}}'::jsonb);
        ASSERT false, 'a class at a coarser rung must be refused';
    EXCEPTION WHEN others THEN
        GET STACKED DIAGNOSTICS v_msg = MESSAGE_TEXT;
        ASSERT v_msg LIKE '%class%', 'the refusal names the offending key: ' || v_msg;
    END;

    BEGIN
        PERFORM cairn_check_safety_signal('{"safety":{"severity":"high"}}'::jsonb);
        ASSERT false, 'a signal with no rung must be refused';
    EXCEPTION WHEN others THEN NULL;
    END;

    BEGIN
        PERFORM cairn_check_safety_signal('{"safety":{"rung":"precise","severity":"high"}}'::jsonb);
        ASSERT false, 'a precise rung with no class must be refused';
    EXCEPTION WHEN others THEN NULL;
    END;

    BEGIN
        PERFORM cairn_check_safety_signal('{"safety":"not-an-object"}'::jsonb);
        ASSERT false, 'a non-object signal must be refused';
    EXCEPTION WHEN others THEN NULL;
    END;
END $$;

-- ---------------------------------------------------------------------------
-- 3. The class map ships EMPTY, and the shipped state is the assertion.
--
--    Cairn ships the lookup MECHANISM, never the drug knowledge: a seeded row would be an
--    un-reviewable clinical policy choice smuggled into infrastructure (principle 9). This
--    is the same assertion db/tests/048 makes about sensitivity_category_map, and it is
--    asserted NOWHERE else — Tasks 5/6's Rust suites run against a long-lived shared
--    database that other tests seed rows into, so a row-count assertion there would be
--    flaky by construction. This mirror runs against a freshly created, freshly dropped
--    scratch database (scripts/run-db-sql-tests.sh), so a nonzero count here is unambiguous.
-- ---------------------------------------------------------------------------
DO $$
DECLARE v_n bigint;
BEGIN
    SELECT count(*) INTO v_n FROM safety_class_map;
    ASSERT v_n = 0, 'safety_class_map must ship EMPTY (principle 9)';
END $$;

-- ---------------------------------------------------------------------------
-- 4. The read model's totality, on seeded rows.
--
--    WHY NOT submit_event: that door needs a real Ed25519-signed envelope and this rig has
--    no signing key (the same limitation db/tests/047 and db/tests/048 explain). Seeding
--    event_log directly still exercises the REAL read functions, which is what is under
--    test here. Runs inside a transaction that ROLLBACKs, so it leaves no residue.
-- ---------------------------------------------------------------------------
BEGIN;

CREATE OR REPLACE FUNCTION _safety_seed_event(
    p_patient uuid, p_type text, p_safety jsonb, p_wall bigint
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_id    uuid  := gen_random_uuid();
    v_bytes bytea := convert_to(v_id::text || p_wall::text, 'UTF8');
BEGIN
    INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
                           hlc_wall, hlc_counter, node_origin, signed_bytes,
                           content_address, body, contributors, signer_key_id,
                           plaintext_twin, safety)
    VALUES (v_id, p_patient, p_type, p_type || '/1', p_wall, 0, 'sqltest', v_bytes,
            '\x1220'::bytea || digest(v_bytes, 'sha256'), '{}'::jsonb, '[]'::jsonb,
            'kid', 'twin', p_safety);
    RETURN v_id;
END $$;

DO $$
DECLARE
    v_patient uuid := gen_random_uuid();
    v_a uuid; v_b uuid; v_c uuid;
    v_rung text; v_class text;
BEGIN
    -- A self-contradictory signal: stored verbatim, but its class must never surface.
    v_a := _safety_seed_event(v_patient, 'note.added',
        '{"rung":"existence","class":"rh-sensitizing"}'::jsonb, 100);
    SELECT rung, class INTO v_rung, v_class FROM cairn_event_safety(v_a);
    ASSERT v_rung = 'existence', 'the stored rung stands when no grade coarsens it further';
    ASSERT v_class IS NULL,
        'a class is surfaced ONLY at rung precise, whatever the row holds — this totality '
        'is what makes the apply door''s leniency safe';

    -- An unrecognised rung reads as the coarsest NAMED rung, not echoed back.
    v_b := _safety_seed_event(v_patient, 'note.added',
        '{"rung":"rung:from-a-future-peer","severity":"critical"}'::jsonb, 101);
    SELECT rung INTO v_rung FROM cairn_event_safety(v_b);
    ASSERT v_rung = 'existence', 'an unrecognised rung discloses nothing';

    -- No signal at all yields no row: an existence marker on every event would
    -- manufacture a warning from nothing.
    v_c := _safety_seed_event(v_patient, 'note.added', NULL, 102);
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_event_safety(v_c)),
        'no signal means no row';
END $$;

DROP FUNCTION _safety_seed_event(uuid, text, jsonb, bigint);
ROLLBACK;
