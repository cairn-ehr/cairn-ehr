-- 077_medication_charting.sql — charting a medication (charting slice 1a, design §3.4–§3.5).
--
-- Charting adds five OPTIONAL keys to clinical.medication.asserted, with the event type and
-- schema version unchanged (principle 11 — an event without them is the old event):
--   charted_as                 started-here | pre-existing | unknown
--   route                      an open lower-case token (po, iv, im, sc, …)
--   allergy_override           {reason, against: [allergy record uuid, …]}
--   substance.strength         free strength text ("875/125 mg")
--   substance.components       a combination: [{term, coding|null}, …], ≥ 2, never beside
--                              substance.coding
--
-- §1 (this task) is the floor. Later sections add the projection and the drug search.
--
-- WHY A SEPARATE FILE: a floor change needs its own SCHEMA_GENERATION bump (#188), so an
-- older binary cannot CREATE OR REPLACE the check back down; db/031 only gains the one
-- PERFORM line that calls it (plpgsql resolves the call at execution, so the later file is
-- fine — db/041's coding check is wired the same way).
BEGIN;

-- §1. The charting floor. TWO TIERS, db/041's split, because the per-type floor runs at
--     BOTH doors (db/020 dispatches the same check as submit_event):
--       type       (a key the projection could not read: not a string, not an array, an
--                  `against` entry that is not a uuid, a malformed component coding)
--                  -> refuse at BOTH doors
--       vocabulary / count / emptiness (charted_as outside the three words, fewer than two
--                  components, components beside a single coding, an empty route or reason,
--                  an empty `against`)
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
    v_local boolean := current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on';
    v_key   text;
    v_val   jsonb;
    v_elem  jsonb;
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

    -- allergy_override: {reason, against}. `against` entries become uuids in the projection,
    -- so a non-uuid entry is a type error at both doors.
    v_val := p -> 'allergy_override';
    IF v_val IS NOT NULL AND jsonb_typeof(v_val) <> 'null' THEN
        IF jsonb_typeof(v_val) <> 'object' THEN
            RAISE EXCEPTION 'medication assertion: allergy_override must be an object {reason, against}';
        END IF;
        IF jsonb_typeof(v_val -> 'reason') IS DISTINCT FROM 'string' THEN
            RAISE EXCEPTION 'medication assertion: allergy_override.reason must be a string';
        END IF;
        IF v_local AND length(btrim(v_val ->> 'reason')) = 0 THEN
            RAISE EXCEPTION 'medication assertion: allergy_override.reason must be a non-empty string';
        END IF;
        IF jsonb_typeof(v_val -> 'against') IS DISTINCT FROM 'array' THEN
            RAISE EXCEPTION 'medication assertion: allergy_override.against must be an array of allergy record uuids';
        END IF;
        FOR v_elem IN SELECT e FROM jsonb_array_elements(v_val -> 'against') AS t(e) LOOP
            IF jsonb_typeof(v_elem) <> 'string' OR NOT pg_input_is_valid(v_elem #>> '{}', 'uuid') THEN
                RAISE EXCEPTION 'medication assertion: allergy_override.against entries must be uuid strings, got %',
                    v_elem;
            END IF;
        END LOOP;
        IF v_local AND jsonb_array_length(v_val -> 'against') = 0 THEN
            RAISE EXCEPTION 'medication assertion: allergy_override.against must name at least one allergy record';
        END IF;
    END IF;
END;
$$;
-- The cairn_check_* family is revoked from PUBLIC uniformly (#382).
REVOKE EXECUTE ON FUNCTION cairn_check_medication_charting(jsonb) FROM PUBLIC;

COMMIT;
