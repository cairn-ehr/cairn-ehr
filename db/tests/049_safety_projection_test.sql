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

    -- The SECOND disclosure guard: a severity at the coarsest rung (2026-08-14 review).
    -- Section 7 gates severity off at 'existence', so admitting it here minted bytes no
    -- reader would ever surface. Keyed on the RANK, so an unrecognised rung — which ranks
    -- coarsest everywhere else — inherits the guard too.
    BEGIN
        PERFORM cairn_check_safety_signal('{"safety":{"rung":"existence","severity":"critical"}}'::jsonb);
        ASSERT false, 'a severity at the coarsest rung must be refused';
    EXCEPTION WHEN others THEN
        GET STACKED DIAGNOSTICS v_msg = MESSAGE_TEXT;
        ASSERT v_msg LIKE '%severity%', 'the refusal names the offending key: ' || v_msg;
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
    v_rung text; v_class text; v_severity text;
BEGIN
    -- A self-contradictory signal: stored verbatim, but its class must never surface.
    v_a := _safety_seed_event(v_patient, 'note.added',
        '{"rung":"existence","class":"rh-sensitizing"}'::jsonb, 100);
    SELECT rung, class INTO v_rung, v_class FROM cairn_event_safety(v_a);
    ASSERT v_rung = 'existence', 'the stored rung stands when no grade coarsens it further';
    ASSERT v_class IS NULL,
        'a class is surfaced ONLY at rung precise, whatever the row holds — this totality '
        'is what makes the apply door''s leniency safe';

    -- THE MIDDLE RUNG IS THE ONLY ONE THAT TELLS THE TWO GATES APART (2026-08-14 review
    -- finding C1). Section 7 gates `class` to 'precise' but `severity` to
    -- ('precise','kind'); at 'existence' both gate off together, so the arm above cannot
    -- distinguish a correct class gate from one widened to IN ('precise','kind'). That
    -- widening — the obvious "make it match the line below" edit — published a withheld
    -- drug class with the whole suite green until this arm existed.
    --
    -- Seeded at rung 'kind' directly rather than via a `sensitive` grade, because this rig
    -- has no signing key and so cannot author a sensitivity assertion (see this section's
    -- header). With no grade standing, 'routine' licenses 'precise', and the coarser of
    -- (kind, precise) is 'kind' — which is the branch under test.
    v_b := _safety_seed_event(v_patient, 'note.added',
        '{"rung":"kind","class":"rh-sensitizing","severity":"high"}'::jsonb, 103);
    SELECT rung, class, severity INTO v_rung, v_class, v_severity
        FROM cairn_event_safety(v_b);
    ASSERT v_rung = 'kind', 'the emitted middle rung stands when no grade coarsens it further';
    ASSERT v_class IS NULL,
        'the class must NOT survive at rung kind — if this fails, db/049 section 7''s class '
        'gate has been widened and a withheld drug class is being published';
    ASSERT v_severity = 'high',
        'the severity DOES survive at rung kind — otherwise this arm would pass for the '
        'wrong reason (both columns gated off, i.e. the existence shape already covered above)';

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

-- ---------------------------------------------------------------------------
-- 5. ADR-0064 / #405 part 2: the overclaim ledger exists, is keyed on content address, and
--    is idempotent on replay. Not itself pinned by crates/cairn-node/tests/safety_overclaim.rs
--    — that suite drives the ledger through the daemon's submit/apply path; this checks the
--    SQL function's own ON CONFLICT DO NOTHING directly, which is a genuine gap: nothing
--    else calls it twice with the SAME content address and checks for a single surviving
--    row. Runs autocommitting (no seeded event_log row is needed — the function only
--    writes safety_overclaim_flag), so cleanup below is explicit.
-- ---------------------------------------------------------------------------
DO $$
DECLARE
    v_ca bytea := '\x1220'::bytea || digest('overclaim-mirror', 'sha256');
    v_p  uuid  := gen_random_uuid();
    n    int;
BEGIN
    PERFORM cairn_record_safety_overclaim_flag(v_ca, v_p, 'precise', 'existence');
    PERFORM cairn_record_safety_overclaim_flag(v_ca, v_p, 'precise', 'existence');
    SELECT count(*) INTO n FROM safety_overclaim_flag WHERE content_address = v_ca;
    ASSERT n = 1, 'the overclaim ledger is idempotent on replay (PK = content address)';
    DELETE FROM safety_overclaim_flag WHERE content_address = v_ca;
END $$;

-- ---------------------------------------------------------------------------
-- 6. #405 part 1: the column floor under the read model. Mirrors
--    crates/cairn-node/tests/safety_read_grants.rs.
--
--    WHY THE MIRROR EARNS ITS PLACE (corrected 2026-08-16). It used to say this was "the
--    half the Rust suite CANNOT test", which is false: every test in safety_read_grants.rs
--    calls connect_and_load_schema, and that replays every db/*.sql on each connect, so the
--    Rust side does observe fresh migration state and asserts the same column privilege.
--    What the mirror actually adds is (a) the definer/search_path CATALOGUE facts, which the
--    Rust file can only reach behaviourally, and (b) coverage in the psql-only CI lane, on a
--    database built from db/*.sql in order rather than one healed by a running node. Stating
--    the real reason matters — a maintainer told the Rust suite "cannot" test this will not
--    think to keep the two halves in step.
--
--    Read as a pair — either assertion alone passes for the wrong reason. Withholding the
--    column while the read functions stay invoker-rights would break the read path for the
--    product's own role; making them definers without withholding the column would close
--    nothing.
-- ---------------------------------------------------------------------------
DO $$
BEGIN
    ASSERT NOT has_column_privilege('cairn_agent', 'event_log', 'safety', 'SELECT'),
        'cairn_agent must NOT hold SELECT on event_log.safety — db/049 section 8 replaces '
        'db/005''s table-level grant with a column list precisely because a table grant '
        'keeps conferring columns added later (#405 part 1)';

    -- A representative granted column, so the section-8 block cannot pass by having
    -- revoked everything.
    ASSERT has_column_privilege('cairn_agent', 'event_log', 'plaintext_twin', 'SELECT'),
        'the column-level GRANT must still confer the rest of event_log';

    -- Addressed by regprocedure, not by bare proname: db/*.sql OVERLOADS rather than
    -- replaces when an argument list changes (the safety_ladder.rs warning), and a bare
    -- `WHERE proname = …` subquery would then raise "more than one row" instead of
    -- asserting anything. A missing function yields NULL, and ASSERT NULL already fails.
    ASSERT (SELECT prosecdef FROM pg_proc WHERE oid = 'cairn_event_safety(uuid)'::regprocedure),
        'cairn_event_safety must be SECURITY DEFINER, or the coarsened read is unreadable '
        'by the only role allowed to call it';
    ASSERT (SELECT prosecdef FROM pg_proc WHERE oid = 'cairn_patient_safety(uuid)'::regprocedure),
        'cairn_patient_safety touches event_log.safety in its OWN where-clause, so it '
        'needs the definer rights independently of cairn_event_safety';

    -- The search_path half of the definer contract, and the reason it is asserted rather
    -- than trusted: `SET search_path = public` does NOT exclude pg_temp, so a definer
    -- carrying only that can be blinded by a caller-created temp `event_log` (2026-08-16
    -- review — it returned ZERO rows for a chart carrying a real signal). A future
    -- CREATE OR REPLACE that keeps SECURITY DEFINER and drops the pg_temp term would pass
    -- both asserts above and re-open it silently, so pin the exact setting.
    ASSERT (SELECT 'search_path=public, pg_temp' = ANY(proconfig)
              FROM pg_proc WHERE oid = 'cairn_event_safety(uuid)'::regprocedure),
        'cairn_event_safety must pin search_path to "public, pg_temp" — pg_temp is searched '
        'FIRST for relation names when the path omits it, so omitting it lets any caller '
        'shadow event_log and suppress the signal';
    ASSERT (SELECT 'search_path=public, pg_temp' = ANY(proconfig)
              FROM pg_proc WHERE oid = 'cairn_patient_safety(uuid)'::regprocedure),
        'cairn_patient_safety names event_log in its own FROM clause, so it needs the '
        'pg_temp-safe path independently of cairn_event_safety';
END $$;
