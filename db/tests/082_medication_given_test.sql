\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/082 §1 + db/077's `stat` — the in-DB mirror for a stat order and the dose given
-- (`clinical.medication-given.asserted`, charting slice 2).
--
-- The given's floor has TWO TIERS (ADR-0056). STRUCTURAL rules (medication_id / completes shape,
-- no charting key, no §5.9 claim) refuse at BOTH doors. The REFERENCE rules (the schema tag, a
-- time given, the line is on THIS chart) refuse at the AUTHORING door only: the same body is
-- ADMITTED at the remote door so an orphan given never wedges sync. `completes` naming a stop is
-- pinned by the Rust suite (medication_given_floor.rs), which needs a signed stop in the log; so
-- are the projection, the shred and the reprojection. Everything ROLLS BACK.

\o /dev/null
BEGIN;

CREATE FUNCTION pg_temp.chart() RETURNS text LANGUAGE sql AS $$ SELECT '82828282-0000-7828-8828-828282828282' $$;
CREATE FUNCTION pg_temp.other() RETURNS text LANGUAGE sql AS $$ SELECT '82828282-0000-7828-8828-828282828283' $$;
CREATE FUNCTION pg_temp.med()   RETURNS text LANGUAGE sql AS $$ SELECT '82828282-0001-7828-8828-828282828282' $$;

-- One line on chart(), inserted as the owner.
INSERT INTO medication_statement
    (medication_id, patient_id, term, info_source, hlc_wall, hlc_counter, origin, content_address)
VALUES (pg_temp.med()::uuid, pg_temp.chart()::uuid, 'adrenaline', 'clinician-observed', 1, 0, 'n', '\x8201');

-- A valid given of `med` on `pat`, `over` merged into the payload.
CREATE FUNCTION pg_temp.g(med text, pat text DEFAULT pg_temp.chart(), over jsonb DEFAULT '{}') RETURNS jsonb
LANGUAGE sql AS $$
    SELECT jsonb_build_object(
        'schema_version', 'clinical.medication-given/1', 'patient_id', pat,
        't_effective', '2026-11-14T10:00:00Z',
        'payload', jsonb_build_object('medication_id', med, 'completes', null) || over);
$$;

CREATE FUNCTION pg_temp.door(remote boolean) RETURNS void LANGUAGE sql AS $$
    SELECT set_config('cairn.remote_apply', CASE WHEN remote THEN 'on' ELSE '' END, true);
$$;

-- `fn` is the floor's name: the given's two-argument check, or db/077's one-argument charting check.
CREATE FUNCTION pg_temp.run(fn text, b jsonb) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    IF fn = 'given' THEN
        PERFORM cairn_check_medication_given('clinical.medication-given.asserted', b);
    ELSE
        PERFORM cairn_check_medication_charting(b);
    END IF;
END $$;

CREATE FUNCTION pg_temp.accepts(fn text, b jsonb) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_temp.run(fn, b);
EXCEPTION WHEN raise_exception THEN
    RAISE EXCEPTION 'FAIL: refused a body it must accept (%) at remote_apply=%: %',
        SQLERRM, current_setting('cairn.remote_apply', true), b;
END $$;

CREATE FUNCTION pg_temp.refuses(fn text, b jsonb, frag text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_temp.run(fn, b);
    RAISE EXCEPTION 'FAIL: accepted a body it must refuse (wanted %) at remote_apply=%: %',
        frag, current_setting('cairn.remote_apply', true), b;
EXCEPTION WHEN raise_exception THEN
    IF SQLERRM NOT SIMILAR TO 'medication (given|assertion):%' OR position(frag IN SQLERRM) = 0 THEN
        RAISE EXCEPTION 'FAIL: refused for the wrong reason (wanted %): %', frag, SQLERRM;
    END IF;
END $$;

-- ---- Structural: both doors ----
DO $$
DECLARE r boolean;
BEGIN
    FOREACH r IN ARRAY ARRAY[false, true] LOOP
        PERFORM pg_temp.door(r);
        -- db/077: stat is JSON true or absent (null = absent), never false.
        PERFORM pg_temp.refuses('charting', '{"stat":false}', 'stat');
        PERFORM pg_temp.refuses('charting', '{"stat":"true"}', 'stat');
        PERFORM pg_temp.accepts('charting', '{"stat":true}');
        PERFORM pg_temp.accepts('charting', '{"stat":null}');
        -- db/082: the given's shape.
        PERFORM pg_temp.refuses('given', pg_temp.g('nope'), 'medication_id');
        PERFORM pg_temp.refuses('given', pg_temp.g(pg_temp.med(), over => '{"completes":"nope"}'), 'completes');
        PERFORM pg_temp.refuses('given', pg_temp.g(pg_temp.med(), over => '{"completes":7}'), 'completes');
        PERFORM pg_temp.refuses('given', pg_temp.g(pg_temp.med(), over => '{"charted_as":null}'), 'charted_as');
        PERFORM pg_temp.refuses('given', pg_temp.g(pg_temp.med(), over => '{"warning_override":{"reason":"x"}}'), 'warning_override');
        PERFORM pg_temp.refuses('given', pg_temp.g(pg_temp.med(), over => '{"restores":"82828282-0001-7828-8828-828282828282"}'), 'restores');
        PERFORM pg_temp.refuses('given', pg_temp.g(pg_temp.med(), over => '{"stat":true}'), 'stat');
        PERFORM pg_temp.refuses('given', pg_temp.g(pg_temp.med()) || '{"safety":{"rung":"existence"}}', 'safety');
        PERFORM pg_temp.accepts('given', pg_temp.g(pg_temp.med()) || '{"safety":null}');
        PERFORM pg_temp.accepts('given', pg_temp.g(pg_temp.med()) #- '{payload,completes}');
    END LOOP;
END $$;

-- ---- Authoring door only ----
DO $$
BEGIN
    PERFORM pg_temp.door(false);
    PERFORM pg_temp.accepts('given', pg_temp.g(pg_temp.med()));
    PERFORM pg_temp.refuses('given', pg_temp.g(pg_temp.med(), pg_temp.other()), 'not a medication on this chart');
    PERFORM pg_temp.refuses('given', pg_temp.g(gen_random_uuid()::text), 'not a medication on this chart');
    PERFORM pg_temp.refuses('given', pg_temp.g(pg_temp.med()) - 't_effective', 't_effective');
    PERFORM pg_temp.refuses('given',
        jsonb_set(pg_temp.g(pg_temp.med()), '{schema_version}', '"clinical.medication-given/2"'),
        'schema_version');
    PERFORM pg_temp.refuses('given',
        pg_temp.g(pg_temp.med(), over => jsonb_build_object('completes', gen_random_uuid()::text)),
        'completes');
    -- The same refusals' bodies are ADMITTED at the remote door (other chart, orphan, no time).
    PERFORM pg_temp.door(true);
    PERFORM pg_temp.accepts('given', pg_temp.g(pg_temp.med(), pg_temp.other()));
    PERFORM pg_temp.accepts('given', pg_temp.g(gen_random_uuid()::text));
    PERFORM pg_temp.accepts('given', pg_temp.g(pg_temp.med()) - 't_effective');
END $$;

-- ---- Registration: the type is classified, floored, projected, shredded and threaded ----
DO $$
BEGIN
    ASSERT EXISTS (SELECT 1 FROM event_type_class
                    WHERE event_type = 'clinical.medication-given.asserted'
                      AND mode = 'additive' AND NOT targets_other_author),
        'the type is registered in event_type_class';
    ASSERT (SELECT check_fn FROM cairn_event_twin_check
             WHERE event_type = 'clinical.medication-given.asserted')
           = 'cairn_check_medication_given', 'the floor is registered';
    ASSERT (SELECT apply_fn FROM cairn_projection_apply
             WHERE event_type = 'clinical.medication-given.asserted')
           = 'medication_given_apply', 'the applier is registered';
    ASSERT position('medication_given' IN pg_get_functiondef('cairn_execute_shred'::regproc)) > 0,
        'the shred takes the given row';
    ASSERT position('medication_given' IN pg_get_functiondef('cairn_event_thread'::regproc)) > 0,
        'a given resolves to its thread';
    ASSERT EXISTS (SELECT 1 FROM information_schema.columns
                    WHERE table_name = 'medication_charting' AND column_name = 'stat'),
        'medication_charting carries stat';
END $$;

ROLLBACK;
\o
