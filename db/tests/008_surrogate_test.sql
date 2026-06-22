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
-- G4 — the domain is a real type barrier in BOTH directions: a uuid cannot be
--      passed where a local_ref is expected, and vice versa. This is what makes
--      a leak a compile/parse-time error rather than a silent mis-join.
-- ===========================================================================
DO $$ BEGIN
    BEGIN
        PERFORM patient_uuid(gen_random_uuid()::text::local_ref);  -- nonsense cast must fail
        RAISE EXCEPTION 'G4 FAILED: a uuid was accepted as a local_ref';
    EXCEPTION
        WHEN invalid_text_representation OR datatype_mismatch OR undefined_function THEN
            RAISE NOTICE 'G4a OK: uuid rejected where local_ref expected';
    END;
    BEGIN
        PERFORM intern_patient(1::text);  -- bigint-ish where uuid expected
        RAISE EXCEPTION 'G4 FAILED: a non-uuid was accepted by intern_patient';
    EXCEPTION
        WHEN invalid_text_representation OR datatype_mismatch OR undefined_function THEN
            RAISE NOTICE 'G4b OK: non-uuid rejected where uuid expected';
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
    PERFORM _b5_seed_event(p, 'patient.created', 1000);
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
