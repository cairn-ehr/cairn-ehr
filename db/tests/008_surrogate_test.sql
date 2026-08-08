\set ON_ERROR_STOP on
-- Cairn walking skeleton — leakage-guard + interning-correctness tests for the
-- dual-identifier discipline (ADR-0031 / data-model §3.18, Spike 0001 Bet B5).
--
-- These are PURE SQL (no pgrx, no cairn_verify): the discipline they guard lives
-- entirely in the projection plane, so the test needs only 001_envelope.sql,
-- 002_projection.sql and 008_surrogate_projection.sql loaded. Each case is a
-- self-checking DO block that RAISEs on failure, so `psql -v ON_ERROR_STOP=1`
-- exits non-zero the moment any guard is breached.
--
-- The load-bearing claim under test: a node-local bigint surrogate may key the
-- projection for speed, but it must NEVER escape the projection — a leaked
-- surrogate means two nodes assign different integers to the same patient and
-- set-union sync silently diverges. We make that a *mechanically checked*
-- property, not a code-review habit.

-- ---------------------------------------------------------------------------
-- Fixture: append a minimal valid event straight to event_log (as owner), which
-- fires the AFTER INSERT projection triggers. We bypass submit_event on purpose
-- — this test is about the projection plane, not the write floor (Spike 0002).
-- content_address must satisfy the 001 CHECK: '\x1220' || sha256(signed_bytes).
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION _b5_seed_event(p_patient uuid, p_type text, p_wall bigint)
RETURNS uuid LANGUAGE plpgsql AS $$
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
        v_id, p_patient, p_type, 'v1',
        p_wall, 0, 'test-node',
        v_bytes, '\x1220'::bytea || digest(v_bytes, 'sha256'),
        jsonb_build_object('name','Test', 'note','n'), '[]'::jsonb,
        'k', 'twin');
    RETURN v_id;
END;
$$;

-- ===========================================================================
-- G1 — the `local_ref` domain exists and is a bigint (the type-system guard).
-- ===========================================================================
DO $$
DECLARE v_base regtype;
BEGIN
    SELECT t.typbasetype::regtype INTO v_base
        FROM pg_type t WHERE t.typname = 'local_ref' AND t.typtype = 'd';
    IF v_base IS NULL THEN
        RAISE EXCEPTION 'G1 FAILED: domain local_ref does not exist';
    END IF;
    IF v_base <> 'bigint'::regtype THEN
        RAISE EXCEPTION 'G1 FAILED: local_ref base type is % (want bigint)', v_base;
    END IF;
    RAISE NOTICE 'G1 OK: local_ref is a domain over bigint';
END $$;

-- ===========================================================================
-- G2 — the CANONICAL/SIGNED plane is surrogate-free. event_log carries no
--      local_ref column, and patient_id is still the canonical uuid. (If a
--      surrogate ever appeared here it could ride the wire — the core breach.)
-- ===========================================================================
DO $$
DECLARE v_leak int; v_pid_type text;
BEGIN
    SELECT count(*) INTO v_leak
        FROM information_schema.columns c
        WHERE c.table_name = 'event_log' AND c.domain_name = 'local_ref';
    IF v_leak <> 0 THEN
        RAISE EXCEPTION 'G2 FAILED: event_log has % local_ref column(s) — surrogate on the signed plane', v_leak;
    END IF;
    SELECT data_type INTO v_pid_type
        FROM information_schema.columns
        WHERE table_name = 'event_log' AND column_name = 'patient_id';
    IF v_pid_type <> 'uuid' THEN
        RAISE EXCEPTION 'G2 FAILED: event_log.patient_id is % (want uuid)', v_pid_type;
    END IF;
    RAISE NOTICE 'G2 OK: event_log is surrogate-free; patient_id stays canonical uuid';
END $$;

-- ===========================================================================
-- G3 — interning is deterministic and dense: the same UUID always maps to the
--      same ref (idempotent); distinct UUIDs map to distinct, contiguous refs.
-- ===========================================================================
DO $$
DECLARE
    pa uuid := gen_random_uuid();
    pb uuid := gen_random_uuid();
    ra1 bigint; ra2 bigint; rb bigint;
BEGIN
    ra1 := intern_patient(pa);
    ra2 := intern_patient(pa);          -- second call: must return the SAME ref
    rb  := intern_patient(pb);
    IF ra1 <> ra2 THEN
        RAISE EXCEPTION 'G3 FAILED: intern_patient not idempotent (% vs %)', ra1, ra2;
    END IF;
    IF ra1 = rb THEN
        RAISE EXCEPTION 'G3 FAILED: distinct patients collided on ref %', ra1;
    END IF;
    -- The anchor row binds BOTH fields, exactly once per patient.
    IF (SELECT count(*) FROM patient_ref WHERE patient_id = pa) <> 1 THEN
        RAISE EXCEPTION 'G3 FAILED: patient_ref does not hold exactly one anchor row for pa';
    END IF;
    RAISE NOTICE 'G3 OK: interning idempotent (%), distinct, anchor binds both', ra1;
END $$;

-- ===========================================================================
-- G4 — what the `local_ref` domain actually guards, stated honestly.
--
--   A `local_ref` is `CREATE DOMAIN local_ref AS BIGINT`. A domain over a base
--   type is NOT a symmetric, two-way type barrier: a `local_ref` value flows
--   into any plain `bigint` with no cast and no error, so the dangerous
--   direction — a surrogate leaking into a bare `bigint` — is NOT blocked by the
--   domain (G4c proves this, so no future reader over-trusts it). What the type
--   system DOES enforce is one-directional: a `uuid` cannot be used where a
--   `local_ref`/`bigint` is expected, and a surrogate cannot be cast into the
--   canonical `uuid` plane. The LOAD-BEARING leak guarantee is therefore G2
--   (event_log is surrogate-free) + the `uuid` column type, NOT this domain;
--   the domain is intent-signalling plus that one-directional guard.
--
--   We assert the two functions exist first, so the type checks below cannot
--   pass vacuously by resolving to "function does not exist" (the old G4 listed
--   `undefined_function` among the accepted errors and would have done exactly
--   that). undefined_function is deliberately NOT caught anywhere here.
-- ===========================================================================
DO $$ BEGIN
    IF to_regprocedure('intern_patient(uuid)') IS NULL THEN
        RAISE EXCEPTION 'G4 FAILED: intern_patient(uuid) does not exist — type checks would pass vacuously';
    END IF;
    IF to_regprocedure('patient_uuid(local_ref)') IS NULL THEN
        RAISE EXCEPTION 'G4 FAILED: patient_uuid(local_ref) does not exist — type checks would pass vacuously';
    END IF;

    -- G4a — the genuine guard: a real uuid value cannot be coerced to local_ref.
    -- (Note: a uuid value directly, not a text round-trip — the old test cast
    -- uuid::text::local_ref and caught the text→bigint PARSE failure, which is
    -- base coercion, not the type relationship we mean to assert.)
    BEGIN
        PERFORM gen_random_uuid()::local_ref;
        RAISE EXCEPTION 'G4a FAILED: a uuid was coerced to a local_ref';
    EXCEPTION
        WHEN cannot_coerce THEN
            RAISE NOTICE 'G4a OK: uuid cannot be coerced to local_ref (one-directional guard)';
    END;

    -- G4b — the load-bearing signed-plane guarantee: a surrogate (bigint) cannot
    -- be cast into the canonical uuid plane, so it can never reach
    -- event_log.patient_id by a stray cast. This is `bigint <> uuid` doing the
    -- work (with G2), which is the protection the prose must credit — not the
    -- local_ref domain.
    BEGIN
        PERFORM 99999::bigint::uuid;
        RAISE EXCEPTION 'G4b FAILED: a bigint surrogate was cast into the canonical uuid plane';
    EXCEPTION
        WHEN cannot_coerce THEN
            RAISE NOTICE 'G4b OK: a bigint surrogate cannot be cast to uuid (signed plane is protected by bigint<>uuid + G2)';
    END;

    -- G4c — the HONEST limit, asserted so it cannot regress into a false belief:
    -- a bare bigint IS silently accepted where a `local_ref` is expected, because
    -- the domain is transparent over its base type. This direction is NOT a
    -- barrier; the test records the true behaviour rather than claiming otherwise.
    BEGIN
        PERFORM patient_uuid(99999::bigint);  -- accepted silently; returns NULL (no such ref)
        RAISE NOTICE 'G4c OK: bigint flows into local_ref with no type error — domain is a one-directional hint, not a two-way barrier';
    EXCEPTION
        WHEN cannot_coerce OR datatype_mismatch THEN
            RAISE EXCEPTION 'G4c FAILED: bigint→local_ref unexpectedly rejected — the test''s honesty assumption is wrong, revisit the prose';
    END;
END $$;

-- ===========================================================================
-- G5 — the dual-identifier shape: the ANCHOR carries both, the REFERENCING
--      child carries ONLY the surrogate (no canonical uuid duplicated per row —
--      that would re-import the fan-out cost ADR-0031 removes).
-- ===========================================================================
DO $$
DECLARE has_pid boolean; has_lref boolean;
BEGIN
    -- anchor: both
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_name='patient_ref' AND column_name='patient_id') THEN
        RAISE EXCEPTION 'G5 FAILED: anchor patient_ref lacks canonical patient_id';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_name='patient_ref' AND column_name='local_ref') THEN
        RAISE EXCEPTION 'G5 FAILED: anchor patient_ref lacks local_ref';
    END IF;
    -- referencing child: surrogate only
    SELECT EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_name='chart_note_s' AND column_name='patient_id'),
           EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_name='chart_note_s' AND column_name='patient_lref')
      INTO has_pid, has_lref;
    IF has_pid THEN
        RAISE EXCEPTION 'G5 FAILED: chart_note_s duplicates canonical patient_id on every row';
    END IF;
    IF NOT has_lref THEN
        RAISE EXCEPTION 'G5 FAILED: chart_note_s has no surrogate FK';
    END IF;
    RAISE NOTICE 'G5 OK: anchor carries both, child carries only the surrogate';
END $$;

-- ===========================================================================
-- G6 — round trip through the projection: seeding events interns the patient
--      and the surrogate-keyed child resolves back to the EXACT canonical UUID
--      via the anchor join. Egress rehydrates the global ID; the ref stays in.
-- ===========================================================================
DO $$
DECLARE
    p uuid := gen_random_uuid();
    n int;
    got uuid;
BEGIN
    PERFORM _b5_seed_event(p, 'patient.amended', 1000);
    PERFORM _b5_seed_event(p, 'note.added', 1001);
    PERFORM _b5_seed_event(p, 'note.added', 1002);

    SELECT count(*) INTO n FROM chart_note_s s
        JOIN patient_ref r ON r.local_ref = s.patient_lref
        WHERE r.patient_id = p;
    IF n <> 2 THEN
        RAISE EXCEPTION 'G6 FAILED: expected 2 surrogate-keyed notes for p, got %', n;
    END IF;

    -- Egress view must expose the canonical uuid and NOT the surrogate.
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name='chart_note_s_egress' AND column_name='patient_lref') THEN
        RAISE EXCEPTION 'G6 FAILED: egress view leaks the surrogate patient_lref';
    END IF;
    SELECT patient_id INTO got FROM chart_note_s_egress WHERE patient_id = p LIMIT 1;
    IF got <> p THEN
        RAISE EXCEPTION 'G6 FAILED: egress did not rehydrate the canonical uuid (% vs %)', got, p;
    END IF;
    RAISE NOTICE 'G6 OK: surrogate child round-trips to the canonical uuid at egress';
END $$;

DROP FUNCTION _b5_seed_event(uuid, text, bigint);

\echo 'B5 surrogate/leakage guard: ALL PASS'
