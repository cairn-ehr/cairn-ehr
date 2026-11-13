\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/079 §2 — the in-DB mirror for the set-aside medication version
-- (`clinical.medication-set-aside.asserted`, batch charting slice 2a).
--
-- WHY THIS MIRROR EXISTS: the floor has TWO TIERS (ADR-0056). A STRUCTURAL rule (key shapes,
-- charted_as / warning_override absent) refuses at BOTH doors. A REFERENCE rule (the companion
-- is a CURRENT thread on THIS chart, the version id is new, the schema tag, no §5.9 claim)
-- refuses at the AUTHORING door only; the same body is ADMITTED at the remote door
-- (`cairn.remote_apply = 'on'`) so an orphan version, whose companion has not synced, never
-- wedges sync. Each rule is pinned by message fragment, so a refusal for the wrong reason fails
-- too. The Rust suite (crates/cairn-node/tests/medication_set_aside_floor.rs) drives the real
-- seal -> sign -> submit path, the shred and the reprojection.
--
-- The floor is called DIRECTLY (as the schema owner) with hand-built bodies; the companion
-- thread is a medication_statement row inserted as the owner. Everything ROLLS BACK.

\o /dev/null
BEGIN;

CREATE FUNCTION pg_temp.chart()  RETURNS text LANGUAGE sql AS $$ SELECT '77777777-0000-7777-8777-777777777777' $$;
CREATE FUNCTION pg_temp.other()  RETURNS text LANGUAGE sql AS $$ SELECT '77777777-0000-7777-8777-777777777778' $$;
CREATE FUNCTION pg_temp.live()   RETURNS text LANGUAGE sql AS $$ SELECT '77777777-0001-7777-8777-777777777777' $$;
CREATE FUNCTION pg_temp.ceased() RETURNS text LANGUAGE sql AS $$ SELECT '77777777-0002-7777-8777-777777777777' $$;
CREATE FUNCTION pg_temp.foreign_() RETURNS text LANGUAGE sql AS $$ SELECT '77777777-0003-7777-8777-777777777777' $$;

-- Three threads: a live one and a stopped one on chart(), a live one on other().
INSERT INTO medication_statement
    (medication_id, patient_id, term, info_source, hlc_wall, hlc_counter, origin, content_address)
VALUES
    (pg_temp.live()::uuid,     pg_temp.chart()::uuid, 'atorvastatin', 'clinician-observed', 1, 0, 'n1', '\x01'),
    (pg_temp.ceased()::uuid,   pg_temp.chart()::uuid, 'ramipril',     'clinician-observed', 1, 0, 'n1', '\x02'),
    (pg_temp.foreign_()::uuid, pg_temp.other()::uuid, 'metformin',    'clinician-observed', 1, 0, 'n1', '\x03');
INSERT INTO medication_cessation (medication_id, patient_id, hlc_wall, hlc_counter, origin, content_address)
VALUES (pg_temp.ceased()::uuid, pg_temp.chart()::uuid, 2, 0, 'n1', '\x04');

-- A valid version beside `alt` on chart(), `over` merged into the payload.
CREATE FUNCTION pg_temp.v(alt text, over jsonb DEFAULT '{}') RETURNS jsonb LANGUAGE sql AS $$
    SELECT jsonb_build_object(
        'schema_version', 'clinical.medication-set-aside/1',
        'patient_id', pg_temp.chart(),
        'payload', jsonb_build_object(
            'version_id', gen_random_uuid(), 'alternative_to', alt,
            'substance', '{"term":"Lipitor"}'::jsonb, 'info_source', 'external-record') || over);
$$;

CREATE FUNCTION pg_temp.door(remote boolean) RETURNS void LANGUAGE sql AS $$
    SELECT set_config('cairn.remote_apply', CASE WHEN remote THEN 'on' ELSE '' END, true);
$$;

CREATE FUNCTION pg_temp.accepts(b jsonb) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM cairn_check_medication_set_aside('clinical.medication-set-aside.asserted', b);
EXCEPTION WHEN raise_exception THEN
    RAISE EXCEPTION 'FAIL: refused a body it must accept (%) at remote_apply=%: %',
        SQLERRM, current_setting('cairn.remote_apply', true), b;
END $$;

CREATE FUNCTION pg_temp.refuses(b jsonb, frag text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM cairn_check_medication_set_aside('clinical.medication-set-aside.asserted', b);
    RAISE EXCEPTION 'FAIL: accepted a body it must refuse (wanted %) at remote_apply=%: %',
        frag, current_setting('cairn.remote_apply', true), b;
EXCEPTION WHEN raise_exception THEN
    IF SQLERRM NOT SIMILAR TO 'medication (set-aside|assertion):%' OR position(frag IN SQLERRM) = 0 THEN
        RAISE EXCEPTION 'FAIL: refused for the wrong reason (wanted %): %', frag, SQLERRM;
    END IF;
END $$;

-- ---- Structural: both doors ----
DO $$
DECLARE r boolean;
BEGIN
    FOREACH r IN ARRAY ARRAY[false, true] LOOP
        PERFORM pg_temp.door(r);
        PERFORM pg_temp.refuses(pg_temp.v(pg_temp.live(), '{"version_id":"nope"}'), 'version_id');
        PERFORM pg_temp.refuses(pg_temp.v(pg_temp.live(), '{"alternative_to":7}'), 'alternative_to');
        PERFORM pg_temp.refuses(pg_temp.v(pg_temp.live(), '{"substance":{"term":"  "}}'), 'substance.term');
        PERFORM pg_temp.refuses(pg_temp.v(pg_temp.live(), '{"info_source":3}'), 'info_source');
        PERFORM pg_temp.refuses(pg_temp.v(pg_temp.live(), '{"charted_as":"brand"}'), 'charted_as');
        PERFORM pg_temp.refuses(pg_temp.v(pg_temp.live(), '{"warning_override":{"reason":"x"}}'), 'warning_override');
        PERFORM pg_temp.refuses(pg_temp.v(pg_temp.live(), '{"why":true}'), 'why');
        PERFORM pg_temp.refuses(pg_temp.v(pg_temp.live(), '{"route":{"a":1}}'), 'route');
        PERFORM pg_temp.refuses(pg_temp.v(pg_temp.live()) || '{"safety":{"rung":"existence"}}', 'safety');
        PERFORM pg_temp.accepts(pg_temp.v(pg_temp.live(), '{"route":"oral"}'));
    END LOOP;
END $$;

-- ---- Authoring door only ----
DO $$
BEGIN
    PERFORM pg_temp.door(false);
    PERFORM pg_temp.accepts(pg_temp.v(pg_temp.live()));
    PERFORM pg_temp.refuses(pg_temp.v(pg_temp.foreign_()), 'not a medication on this chart');
    PERFORM pg_temp.refuses(pg_temp.v(gen_random_uuid()::text), 'not a medication on this chart');
    PERFORM pg_temp.refuses(pg_temp.v(pg_temp.ceased()), 'not a current medication');
    PERFORM pg_temp.refuses(
        jsonb_set(pg_temp.v(pg_temp.live()), '{schema_version}', '"clinical.medication-set-aside/2"'),
        'schema_version');
    -- The same refusals' bodies are ADMITTED at the remote door (orphan, other chart, stopped).
    PERFORM pg_temp.door(true);
    PERFORM pg_temp.accepts(pg_temp.v(pg_temp.foreign_()));
    PERFORM pg_temp.accepts(pg_temp.v(gen_random_uuid()::text));
    PERFORM pg_temp.accepts(pg_temp.v(pg_temp.ceased()));
END $$;

-- ---- Registration: the type is classified, floored, and projected ----
DO $$
BEGIN
    ASSERT EXISTS (SELECT 1 FROM event_type_class
                    WHERE event_type = 'clinical.medication-set-aside.asserted'),
        'the type is registered in event_type_class';
    ASSERT (SELECT check_fn FROM cairn_event_twin_check
             WHERE event_type = 'clinical.medication-set-aside.asserted')
           = 'cairn_check_medication_set_aside', 'the floor is registered';
    ASSERT (SELECT apply_fn FROM cairn_projection_apply
             WHERE event_type = 'clinical.medication-set-aside.asserted')
           = 'medication_set_aside_apply', 'the applier is registered';
END $$;

ROLLBACK;
\o
