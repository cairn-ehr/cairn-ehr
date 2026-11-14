\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/080 §2 — the in-DB mirror for "checked: not current"
-- (`clinical.medication-set-aside-checked.asserted`, batch charting slice 2b).
--
-- The floor has TWO TIERS (ADR-0056). STRUCTURAL rules (version_id / info_source shape, the
-- current-line keys absent, no §5.9 claim) refuse at BOTH doors. The REFERENCE rules (the schema
-- tag; version_id is a set-aside version of THIS chart) refuse at the AUTHORING door only: the
-- same body is ADMITTED at the remote door so an orphan check never wedges sync. Each refusal is
-- pinned by message fragment. The Rust suite (medication_version_checked_floor.rs) drives the
-- real sign -> submit path, the shred and the reprojection. Everything ROLLS BACK.

\o /dev/null
BEGIN;

CREATE FUNCTION pg_temp.chart() RETURNS text LANGUAGE sql AS $$ SELECT '78787878-0000-7878-8878-787878787878' $$;
CREATE FUNCTION pg_temp.other() RETURNS text LANGUAGE sql AS $$ SELECT '78787878-0000-7878-8878-787878787879' $$;
CREATE FUNCTION pg_temp.ver()   RETURNS text LANGUAGE sql AS $$ SELECT '78787878-0001-7878-8878-787878787878' $$;

-- One set-aside version on chart().
INSERT INTO medication_set_aside
    (content_address, event_id, patient_id, version_id, alternative_to, term, info_source, t_recorded)
VALUES ('\x7801', gen_random_uuid(), pg_temp.chart()::uuid, pg_temp.ver()::uuid, gen_random_uuid(),
        'Lipitor', 'external-record', now());

-- A valid check of `ver` on `pat`, `over` merged into the payload.
CREATE FUNCTION pg_temp.k(ver text, pat text DEFAULT pg_temp.chart(), over jsonb DEFAULT '{}') RETURNS jsonb
LANGUAGE sql AS $$
    SELECT jsonb_build_object(
        'schema_version', 'clinical.medication-set-aside-checked/1', 'patient_id', pat,
        'payload', jsonb_build_object('version_id', ver, 'info_source', 'external-record') || over);
$$;

CREATE FUNCTION pg_temp.door(remote boolean) RETURNS void LANGUAGE sql AS $$
    SELECT set_config('cairn.remote_apply', CASE WHEN remote THEN 'on' ELSE '' END, true);
$$;

CREATE FUNCTION pg_temp.accepts(b jsonb) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM cairn_check_medication_version_checked('clinical.medication-set-aside-checked.asserted', b);
EXCEPTION WHEN raise_exception THEN
    RAISE EXCEPTION 'FAIL: refused a body it must accept (%) at remote_apply=%: %',
        SQLERRM, current_setting('cairn.remote_apply', true), b;
END $$;

CREATE FUNCTION pg_temp.refuses(b jsonb, frag text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM cairn_check_medication_version_checked('clinical.medication-set-aside-checked.asserted', b);
    RAISE EXCEPTION 'FAIL: accepted a body it must refuse (wanted %) at remote_apply=%: %',
        frag, current_setting('cairn.remote_apply', true), b;
EXCEPTION WHEN raise_exception THEN
    IF SQLERRM NOT SIMILAR TO 'medication (version-checked|assertion):%' OR position(frag IN SQLERRM) = 0 THEN
        RAISE EXCEPTION 'FAIL: refused for the wrong reason (wanted %): %', frag, SQLERRM;
    END IF;
END $$;

-- ---- Structural: both doors ----
DO $$
DECLARE r boolean;
BEGIN
    FOREACH r IN ARRAY ARRAY[false, true] LOOP
        PERFORM pg_temp.door(r);
        PERFORM pg_temp.refuses(pg_temp.k('nope'), 'version_id');
        PERFORM pg_temp.refuses(pg_temp.k(pg_temp.ver(), over => '{"version_id":7}'), 'version_id');
        PERFORM pg_temp.refuses(pg_temp.k(pg_temp.ver(), over => '{"info_source":"  "}'), 'info_source');
        PERFORM pg_temp.refuses(pg_temp.k(pg_temp.ver(), over => '{"info_source":3}'), 'info_source');
        PERFORM pg_temp.refuses(pg_temp.k(pg_temp.ver(), over => '{"charted_as":"brand"}'), 'charted_as');
        PERFORM pg_temp.refuses(pg_temp.k(pg_temp.ver(), over => '{"charted_as":null}'), 'charted_as');
        PERFORM pg_temp.refuses(pg_temp.k(pg_temp.ver(), over => '{"warning_override":{"reason":"x"}}'), 'warning_override');
        PERFORM pg_temp.refuses(pg_temp.k(pg_temp.ver(), over => '{"restores":"78787878-0001-7878-8878-787878787878"}'), 'restores');
        PERFORM pg_temp.refuses(pg_temp.k(pg_temp.ver(), over => '{"stat":true}'), 'stat must be absent');
        PERFORM pg_temp.refuses(pg_temp.k(pg_temp.ver(), over => '{"why":true}'), 'why');
        PERFORM pg_temp.refuses(pg_temp.k(pg_temp.ver()) || '{"safety":{"rung":"existence"}}', 'safety');
        PERFORM pg_temp.accepts(pg_temp.k(pg_temp.ver()) || '{"safety":null}');
        PERFORM pg_temp.accepts(pg_temp.k(pg_temp.ver(), over => '{"source_detail":"pharmacy","why":"x"}'));
    END LOOP;
END $$;

-- ---- Authoring door only ----
DO $$
BEGIN
    PERFORM pg_temp.door(false);
    PERFORM pg_temp.accepts(pg_temp.k(pg_temp.ver()));
    PERFORM pg_temp.refuses(pg_temp.k(pg_temp.ver(), pg_temp.other()), 'not a set-aside version on this chart');
    PERFORM pg_temp.refuses(pg_temp.k(gen_random_uuid()::text), 'not a set-aside version on this chart');
    PERFORM pg_temp.refuses(
        jsonb_set(pg_temp.k(pg_temp.ver()), '{schema_version}', '"clinical.medication-set-aside-checked/2"'),
        'schema_version');
    -- The same refusals' bodies are ADMITTED at the remote door (other chart, orphan).
    PERFORM pg_temp.door(true);
    PERFORM pg_temp.accepts(pg_temp.k(pg_temp.ver(), pg_temp.other()));
    PERFORM pg_temp.accepts(pg_temp.k(gen_random_uuid()::text));
END $$;

-- ---- Registration: the type is classified, floored, projected, and shredded ----
DO $$
BEGIN
    ASSERT EXISTS (SELECT 1 FROM event_type_class
                    WHERE event_type = 'clinical.medication-set-aside-checked.asserted' AND NOT targets_other_author),
        'the type is registered in event_type_class';
    ASSERT (SELECT check_fn FROM cairn_event_twin_check
             WHERE event_type = 'clinical.medication-set-aside-checked.asserted')
           = 'cairn_check_medication_version_checked', 'the floor is registered';
    ASSERT (SELECT apply_fn FROM cairn_projection_apply
             WHERE event_type = 'clinical.medication-set-aside-checked.asserted')
           = 'medication_set_aside_checked_apply', 'the applier is registered';
    ASSERT position('medication_set_aside_checked' IN pg_get_functiondef('cairn_execute_shred'::regproc)) > 0,
        'the shred takes the check row';
END $$;

ROLLBACK;
\o
