-- 077_medication_charting.sql — charting a medication (charting slice 1a, design §3.4–§3.5).
--
-- Charting adds five OPTIONAL keys to clinical.medication.asserted, with the event type and
-- schema version unchanged (principle 11 — an event without them is the old event):
--   charted_as                 started-here | pre-existing | unknown
--   route                      an open lower-case token (po, iv, im, sc, …)
--   warning_override           {reason, against: [{source, ref}, …]} — charted despite a
--                              warning; source is an open word (`allergy` today)
--   substance.strength         free strength text ("875/125 mg")
--   substance.components       a combination: [{term, coding|null}, …], ≥ 2, never beside
--                              substance.coding
--
-- §1 is the floor; §2 the projection (medication_charting). The medication pick's drug search
-- reads drugref, so it lives in the drug read door, db/076 §5 — the one file that may
-- (no_drugref_dependency.rs).
--
-- WHY A SEPARATE FILE: a floor change needs its own SCHEMA_GENERATION bump (#188), so an
-- older binary cannot CREATE OR REPLACE the check back down; db/031 only gains the one
-- PERFORM line that calls it (plpgsql resolves the call at execution, so the later file is
-- fine — db/041's coding check is wired the same way).
BEGIN;

-- §1. The charting floor. TWO TIERS, db/041's split, because the per-type floor runs at
--     BOTH doors (db/020 dispatches the same check as submit_event):
--       type       (a key the projection could not read: not a string, not an array, an
--                  `against` entry that is not an object, a malformed component coding)
--                  -> refuse at BOTH doors
--       vocabulary / count / emptiness (charted_as outside the three words, fewer than two
--                  components, components beside a single coding, an empty route or reason,
--                  an empty `against`, an override missing its reason, `against`, or an
--                  entry's source / ref, a source that is not a lower-case word, an
--                  allergy ref that is not a uuid)
--                  -> refuse locally, ADMIT remotely (ADR-0051/ADR-0056: a peer may speak a
--                  newer word, and refusing a verifiable event wedges sync)
--
--     The window — not this floor — requires an override's reason (ADR-0021 soft policy);
--     the floor checks the override's shape only when it is present.
--
--     An absent key and an explicit JSON null read the same (honest unknown), as db/041
--     treats substance.coding: a peer that emits nulls for absent optionals is not refused.
CREATE OR REPLACE FUNCTION cairn_check_medication_charting(p jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_local   boolean := current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on';
    v_key     text;
    v_val     jsonb;
    v_elem    jsonb;
    v_against jsonb;  -- warning_override.against, a missing one read as []
BEGIN
    -- Two plain-string keys. charted_as's vocabulary is closed at the authoring door only.
    FOREACH v_key IN ARRAY ARRAY['charted_as', 'route'] LOOP
        v_val := p -> v_key;
        CONTINUE WHEN v_val IS NULL OR jsonb_typeof(v_val) = 'null';
        IF jsonb_typeof(v_val) <> 'string' THEN
            RAISE EXCEPTION 'medication assertion: % must be a string', v_key;
        END IF;
        IF v_local AND length(btrim(v_val #>> '{}')) = 0 THEN
            RAISE EXCEPTION 'medication assertion: % must be a non-empty string', v_key;
        END IF;
    END LOOP;
    IF v_local AND p ->> 'charted_as' NOT IN ('started-here', 'pre-existing', 'unknown') THEN
        RAISE EXCEPTION
            'medication assertion: charted_as must be started-here, pre-existing or unknown, got "%"',
            p ->> 'charted_as';
    END IF;

    -- substance.strength: free text.
    v_val := p -> 'substance' -> 'strength';
    IF v_val IS NOT NULL AND jsonb_typeof(v_val) <> 'null' THEN
        IF jsonb_typeof(v_val) <> 'string' THEN
            RAISE EXCEPTION 'medication assertion: substance.strength must be a string';
        END IF;
        IF v_local AND length(btrim(v_val #>> '{}')) = 0 THEN
            RAISE EXCEPTION 'medication assertion: substance.strength must be a non-empty string';
        END IF;
    END IF;

    -- substance.components: each element {term, coding|null}; the coding object is judged by
    -- db/041's one definition of a coding claim (structure at both doors, registry locally).
    v_val := p -> 'substance' -> 'components';
    IF v_val IS NOT NULL AND jsonb_typeof(v_val) <> 'null' THEN
        IF jsonb_typeof(v_val) <> 'array' THEN
            RAISE EXCEPTION 'medication assertion: substance.components must be an array of {term, coding}';
        END IF;
        FOR v_elem IN SELECT e FROM jsonb_array_elements(v_val) AS t(e) LOOP
            IF jsonb_typeof(v_elem) <> 'object'
               OR jsonb_typeof(v_elem -> 'term') IS DISTINCT FROM 'string' THEN
                RAISE EXCEPTION 'medication assertion: substance.components entries must be {term, coding} with a string term';
            END IF;
            IF v_local AND length(btrim(v_elem ->> 'term')) = 0 THEN
                RAISE EXCEPTION 'medication assertion: substance.components entries need a non-empty term';
            END IF;
            PERFORM cairn_check_coding_object(
                v_elem -> 'coding', 'medication assertion: substance.components[].coding');
            -- A component is one drug, never a group (db/041's rule for substance.coding).
            IF v_local AND v_elem #>> '{coding,system}' = 'drugref-class' THEN
                RAISE EXCEPTION 'medication assertion: substance.components[].coding names a drug group (drugref-class), never one drug';
            END IF;
        END LOOP;
        IF v_local AND jsonb_array_length(v_val) < 2 THEN
            RAISE EXCEPTION 'medication assertion: substance.components lists a combination — at least two, got %',
                jsonb_array_length(v_val);
        END IF;
        IF v_local AND jsonb_typeof(p #> '{substance,coding}') <> 'null' THEN
            RAISE EXCEPTION 'medication assertion: substance.components is never carried beside substance.coding';
        END IF;
    END IF;

    -- warning_override: {reason, against: [{source, ref}, …]} — charted despite a warning.
    -- General, not allergy-only: warnings that need a reason will also come from drug
    -- interactions, drug–disease and renal/hepatic dosing, and the wire is frozen once merged.
    -- A MISSING reason, `against`, source or ref (absent or JSON null) is refused locally but
    -- admitted remotely (ruling R20): the remote door checks the type only of what is present,
    -- so a future window that leaves a part optional never has its events refused by an older
    -- peer. `source` is an OPEN vocabulary at both doors (a newer word is not refused), held to
    -- the lower-case word shape at the authoring door only. Only an `allergy` ref has a known
    -- shape — the allergy record's event uuid — and that too is an authoring-door rule (the
    -- projection keeps `against` as jsonb, so a peer's malformed ref cannot break it).
    v_val := p -> 'warning_override';
    IF v_val IS NOT NULL AND jsonb_typeof(v_val) <> 'null' THEN
        IF jsonb_typeof(v_val) <> 'object' THEN
            RAISE EXCEPTION 'medication assertion: warning_override must be an object {reason, against}';
        END IF;
        IF coalesce(jsonb_typeof(v_val -> 'reason'), 'null') = 'null' THEN
            IF v_local THEN
                RAISE EXCEPTION 'medication assertion: warning_override.reason must be a string';
            END IF;
        ELSIF jsonb_typeof(v_val -> 'reason') <> 'string' THEN
            RAISE EXCEPTION 'medication assertion: warning_override.reason must be a string';
        ELSIF v_local AND length(btrim(v_val ->> 'reason')) = 0 THEN
            RAISE EXCEPTION 'medication assertion: warning_override.reason must be a non-empty string';
        END IF;
        IF coalesce(jsonb_typeof(v_val -> 'against'), 'null') = 'null' THEN
            IF v_local THEN
                RAISE EXCEPTION 'medication assertion: warning_override.against must be an array of {source, ref}';
            END IF;
        ELSIF jsonb_typeof(v_val -> 'against') <> 'array' THEN
            RAISE EXCEPTION 'medication assertion: warning_override.against must be an array of {source, ref}';
        END IF;
        v_against := coalesce(NULLIF(v_val -> 'against', 'null'::jsonb), '[]'::jsonb);
        FOR v_elem IN SELECT e FROM jsonb_array_elements(v_against) AS t(e) LOOP
            IF jsonb_typeof(v_elem) <> 'object' THEN
                RAISE EXCEPTION 'medication assertion: warning_override.against entries must be objects {source, ref}, got %',
                    v_elem;
            END IF;
            -- source, then ref: each a string when present; non-blank when authored here.
            FOREACH v_key IN ARRAY ARRAY['source', 'ref'] LOOP
                IF coalesce(jsonb_typeof(v_elem -> v_key), 'null') = 'null' THEN
                    IF v_local THEN
                        RAISE EXCEPTION 'medication assertion: warning_override.against[].% must be a non-empty string', v_key;
                    END IF;
                ELSIF jsonb_typeof(v_elem -> v_key) <> 'string' THEN
                    RAISE EXCEPTION 'medication assertion: warning_override.against[].% must be a string', v_key;
                ELSIF v_local AND length(btrim(v_elem ->> v_key)) = 0 THEN
                    RAISE EXCEPTION 'medication assertion: warning_override.against[].% must be a non-empty string', v_key;
                END IF;
            END LOOP;
            -- Authored here, a source is a lower-case word, so "Allergy" or " allergy" cannot
            -- step around the allergy ref's uuid rule just below. A peer's is admitted as given.
            IF v_local AND v_elem ->> 'source' !~ '^[a-z][a-z0-9-]*$' THEN
                RAISE EXCEPTION 'medication assertion: warning_override.against[].source must be a lower-case word (a-z, 0-9, -), got "%"',
                    v_elem ->> 'source';
            END IF;
            IF v_local AND v_elem ->> 'source' = 'allergy'
               AND NOT pg_input_is_valid(v_elem ->> 'ref', 'uuid') THEN
                RAISE EXCEPTION 'medication assertion: warning_override.against[].ref of an allergy must be the allergy record''s uuid, got "%"',
                    v_elem ->> 'ref';
            END IF;
        END LOOP;
        IF v_local AND jsonb_array_length(v_against) = 0 THEN
            RAISE EXCEPTION 'medication assertion: warning_override.against must name at least one warning';
        END IF;
    END IF;
END;
$$;
-- The cairn_check_* family is revoked from PUBLIC uniformly (#382).
REVOKE EXECUTE ON FUNCTION cairn_check_medication_charting(jsonb) FROM PUBLIC;

-- §2. The charting projection. Its OWN table, written by a SECOND apply fn on
--     clinical.medication.asserted, rather than columns on medication_statement: a later file
--     may not widen db/031's views (every db/*.sql replays on every connect), and one fact
--     keeps one home (db/031 §4b's argument for medication_coding). Readers JOIN it by
--     medication_id. No FK to medication_statement, for the same arrival-order reason.
--
--     Columns mirror the five keys: `components` is the combination as signed (jsonb, a
--     peer's one-element list included — Task 1 admits it remotely); `override_against` the
--     warnings the line was charted against, the `[{source, ref}, …]` array as signed (jsonb,
--     never cast: a peer's newer source or malformed ref must still project). hlc_* / origin / content_address are the
--     overlay-winner tuple, as medication_coding's, and content_address is what a shred
--     (db/037) deletes by.
CREATE TABLE IF NOT EXISTS medication_charting (
    medication_id    UUID PRIMARY KEY,
    patient_id       UUID NOT NULL,
    charted_as       TEXT,
    route            TEXT,
    strength         TEXT,
    components       JSONB,
    override_reason  TEXT,
    override_against JSONB,
    hlc_wall         BIGINT  NOT NULL,
    hlc_counter      INTEGER NOT NULL,
    origin           TEXT    NOT NULL,
    content_address  BYTEA   NOT NULL,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_charting TO cairn_agent;
-- One-off for a dev database that loaded an earlier draft of this unmerged file, when the
-- column was UUID[] and the payload key had its earlier allergy-only name. Guarded on the
-- column's current type, so a replay on a converged database is a no-op and takes no lock
-- (#726). The old values are DROPPED (USING NULL), not converted with to_jsonb: a converted
-- value would be a bare-uuid array — neither the new `[{source, ref}]` shape nor what a
-- re-fold produces, since the events behind those rows carry the old key, which this apply fn
-- no longer reads. For this column a re-fold of those events writes NULL too.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
                WHERE table_schema = 'public' AND table_name = 'medication_charting'
                  AND column_name = 'override_against' AND data_type = 'ARRAY') THEN
        ALTER TABLE medication_charting ALTER COLUMN override_against TYPE JSONB USING NULL;
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS medication_charting_content_address_idx
    ON medication_charting (content_address);

-- The apply fn, registered below beside medication_statement_apply (db/031) for the same
-- event type. FLOW:
--   1. read the clear payload (ADR-0052); sealed without custody here -> nothing to project;
--   2. pull the five keys out; a JSON null reads as absent (the floor's rule);
--   3. none present -> write nothing. An uncharted assertion has no row, and — as for
--      medication_coding — a later uncharted re-assertion never clears an earlier charting;
--   4. upsert under the overlay-winner rule, so set-union sync converges.
--
-- Never raises on a shape the remote door admits: charted_as is stored as text (a peer may
-- speak a newer word), components and `against` as given (any length, any source word, any
-- ref) — nothing is cast, so nothing a peer signs can make this raise.
--
-- patient_id is the thread's STANDING chart when one is known (db/031's #192 reasoning for
-- medication_coding), else this event's. Both apply fns share run_order 20, so by name this
-- one runs BEFORE medication_statement_apply: on a thread's first event no row exists yet
-- and e.patient_id is the chart the statement is about to take.
CREATE OR REPLACE FUNCTION medication_charting_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p         jsonb := cairn_clear_payload(e);
    v_med     uuid;
    v_comp    jsonb;
    v_against jsonb;
BEGIN
    IF p IS NULL THEN RETURN; END IF;
    v_comp := NULLIF(p #> '{substance,components}', 'null'::jsonb);
    IF jsonb_typeof(p #> '{warning_override,against}') = 'array' THEN
        v_against := p #> '{warning_override,against}';
    END IF;
    -- `->>` already reads a JSON null as SQL NULL, so the text keys need no NULLIF.
    IF p ->> 'charted_as' IS NULL AND p ->> 'route' IS NULL
       AND p #>> '{substance,strength}' IS NULL AND v_comp IS NULL
       AND p #>> '{warning_override,reason}' IS NULL AND v_against IS NULL THEN
        RETURN;
    END IF;
    v_med := (p ->> 'medication_id')::uuid;

    INSERT INTO medication_charting
        (medication_id, patient_id, charted_as, route, strength, components,
         override_reason, override_against, hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        v_med,
        COALESCE(cairn_medication_thread_patient(v_med), e.patient_id),
        p ->> 'charted_as',
        p ->> 'route',
        p #>> '{substance,strength}',
        v_comp,
        p #>> '{warning_override,reason}',
        v_against,
        e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    ON CONFLICT (medication_id) DO UPDATE SET
        patient_id       = EXCLUDED.patient_id,
        charted_as       = EXCLUDED.charted_as,
        route            = EXCLUDED.route,
        strength         = EXCLUDED.strength,
        components       = EXCLUDED.components,
        override_reason  = EXCLUDED.override_reason,
        override_against = EXCLUDED.override_against,
        hlc_wall         = EXCLUDED.hlc_wall,
        hlc_counter      = EXCLUDED.hlc_counter,
        origin           = EXCLUDED.origin,
        content_address  = EXCLUDED.content_address,
        updated_at       = clock_timestamp()
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        medication_charting.hlc_wall, medication_charting.hlc_counter,
        medication_charting.origin, medication_charting.content_address);
END;
$$;
REVOKE EXECUTE ON FUNCTION medication_charting_apply(event_log) FROM PUBLIC;

-- Registered for the generic dispatcher and cairn_reproject (db/005, db/039, ADR-0057), and so
-- for the shred's re-fold (db/037 #863). Converges on replay, write-free once converged (#214).
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('clinical.medication.asserted', 'medication_charting_apply', ARRAY['medication_charting'], 20, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
