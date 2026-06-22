-- Cairn — Spike 0001 Bet B5 measurement: does interning the patient FK to a
-- node-local bigint surrogate (ADR-0031 / data-model §3.18) actually pay?
--
-- Pure SQL so it runs anywhere `psql` does (the Pi included — see PI-RUNBOOK.md),
-- with no Rust rebuild and no pip install. It seeds one event stream and compares
-- the TWO child projections 008_surrogate_projection.sql maintains from it:
--   chart_note_u  — keyed by the 16-byte canonical UUID (today's shape)
--   chart_note_s  — keyed by the 8-byte node-local surrogate (the ADR-0031 shape)
--
-- It reports, for each shape: the patient foreign-key INDEX size, the table heap
-- size, and an EXPLAIN (ANALYZE, BUFFERS) of the realistic "all notes for one
-- patient" read (a direct UUID lookup vs. a surrogate lookup that joins the
-- anchor to rehydrate the canonical id at egress). The headline number is the
-- FK-index size ratio — that is the cost ADR-0031 targets.
--
-- Requires 001_envelope.sql, 002_projection.sql, 008_surrogate_projection.sql.
-- DESTRUCTIVE: it TRUNCATEs the log and projections so the measurement starts
-- from a known-empty state. Run only against a throwaway bench database.
--
-- Scale knobs (override on the psql command line, e.g.
--   psql -v patients=5000 -v notes_per=100 -f db/bench/b5_surrogate.sql):
\if :{?patients}
\else
    \set patients 2000
\endif
\if :{?notes_per}
\else
    \set notes_per 50
\endif

\set ON_ERROR_STOP on
\timing off

-- A known-empty start. CASCADE clears the FK-linked children and the dictionary.
TRUNCATE event_log, patient_chart, patient_ref, chart_note_u, chart_note_s
    RESTART IDENTITY CASCADE;

\echo '--- B5: seeding' :patients 'patients x' :notes_per 'notes each ---'

-- Helper: append a minimal valid event (owner path, bypassing submit_event — a
-- projection-cost measurement, not a write-floor test). content_address must
-- satisfy the 001 CHECK: '\x1220' || sha256(signed_bytes).
CREATE OR REPLACE FUNCTION _b5_seed(p_patient uuid, p_type text, p_wall bigint)
RETURNS void LANGUAGE plpgsql AS $$
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
        p_wall, 0, 'bench-node',
        v_bytes, '\x1220'::bytea || digest(v_bytes, 'sha256'),
        jsonb_build_object('name','Bench Patient'), '[]'::jsonb,
        'k', 'twin');
END;
$$;

-- Seed: register each patient, then attach notes_per notes to it. We keep one
-- patient UUID aside (the median patient) as the read target. The scale knobs are
-- threaded through GUCs because psql does not interpolate :vars inside a DO block.
SELECT set_config('b5.patients',  :'patients',  false);
SELECT set_config('b5.notes_per', :'notes_per', false);
DO $$
DECLARE
    n_patients int := current_setting('b5.patients')::int;
    n_notes    int := current_setting('b5.notes_per')::int;
    p uuid;
    target uuid;
    i int; j int;
BEGIN
    FOR i IN 1..n_patients LOOP
        p := gen_random_uuid();
        IF i = (n_patients / 2) THEN target := p; END IF;
        PERFORM _b5_seed(p, 'patient.created', 1000 + i);
        FOR j IN 1..n_notes LOOP
            PERFORM _b5_seed(p, 'note.added', 2000 + (i * 1000) + j);
        END LOOP;
    END LOOP;
    -- Stash the target for the read tests (a one-row scratch table).
    CREATE TEMP TABLE IF NOT EXISTS _b5_target (patient_id uuid, local_ref bigint);
    INSERT INTO _b5_target VALUES (target, intern_patient(target));
END $$;

DROP FUNCTION _b5_seed(uuid, text, bigint);

ANALYZE chart_note_u;
ANALYZE chart_note_s;
ANALYZE patient_ref;

\echo ''
\echo '=== B5.1  foreign-key INDEX size — the cost ADR-0031 targets ==='
SELECT
    pg_size_pretty(pg_relation_size('chart_note_u_patient_idx')) AS uuid_fk_index,
    pg_size_pretty(pg_relation_size('chart_note_s_patient_idx')) AS surrogate_fk_index,
    round(pg_relation_size('chart_note_u_patient_idx')::numeric
        / nullif(pg_relation_size('chart_note_s_patient_idx'), 0), 2) AS shrink_factor,
    (SELECT count(*) FROM chart_note_s) AS rows_indexed;

\echo ''
\echo '=== B5.2  table HEAP size (the per-row key width also lands here) ==='
SELECT
    pg_size_pretty(pg_relation_size('chart_note_u')) AS uuid_heap,
    pg_size_pretty(pg_relation_size('chart_note_s')) AS surrogate_heap,
    pg_size_pretty(pg_relation_size('patient_ref'))  AS anchor_dictionary;

\echo ''
\echo '=== B5.3  read: all notes for one patient — UUID-keyed (direct lookup) ==='
EXPLAIN (ANALYZE, BUFFERS, COSTS off, TIMING off, SUMMARY off)
SELECT u.event_id, u.recorded_at
FROM chart_note_u u
WHERE u.patient_id = (SELECT patient_id FROM _b5_target);

\echo ''
\echo '=== B5.4  read: all notes for one patient — surrogate-keyed (+anchor rehydrate) ==='
EXPLAIN (ANALYZE, BUFFERS, COSTS off, TIMING off, SUMMARY off)
SELECT s.event_id, r.patient_id, s.recorded_at
FROM chart_note_s s
JOIN patient_ref r ON r.local_ref = s.patient_lref
WHERE s.patient_lref = (SELECT local_ref FROM _b5_target);

\echo ''
\echo 'B5 done. Headline: shrink_factor in B5.1 is the FK-index win; B5.3/B5.4'
\echo 'show the surrogate read stays competitive once the anchor join is counted.'
