-- Cairn — demographic NAMES: the retained-set + display-winner projection (spec §4.2).
--
-- Slice 3 of the demographics subsystem. Names are the first field that needs BOTH a
-- retained set (every name kept as matching evidence) AND a single display-winner
-- selected from it. A name reuses the slice-2 generic `demographic.field.asserted`
-- event with field='name'; the generic floor (db/011 cairn_check_demographic_field)
-- and the authored-twin enforcement already accept it, so this migration adds NO floor
-- change and NO new event type — only the projection. The display-winner is a VIEW
-- (a pure deterministic function of the set), so there is no winner-pointer to maintain.
-- Matching (§5.2) is a separate, later subsystem.

BEGIN;

-- The §4.2 retained set: one row per distinct (patient, use, value) name. use_key
-- folds an absent/blank `use` to 'unspecified' so it is a valid NOT-NULL key component
-- (mirrors patient_identifier.match_key). provenance_rank is cached (reuses db/011's
-- cairn_provenance_rank) so the trigger's recency/provenance test is a plain tuple compare.
CREATE TABLE IF NOT EXISTS patient_name (
    patient_id         UUID    NOT NULL,
    use_key            TEXT    NOT NULL,   -- coalesce(NULLIF(trim(use),''),'unspecified')
    value              TEXT    NOT NULL,   -- the authored display string (opaque to the core)
    use_raw            TEXT,               -- the original `use` facet (NULL when absent)
    provenance         TEXT    NOT NULL,
    provenance_rank    INT     NOT NULL,
    last_hlc_wall      BIGINT  NOT NULL,
    last_hlc_count     INTEGER NOT NULL,
    asserted_origin    TEXT    NOT NULL,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (patient_id, use_key, value)
);

-- Incremental maintenance: fold exactly the one new name event into the retained set.
-- event_log.body holds b->'payload' (see db/005 submit_event INSERT).
CREATE OR REPLACE FUNCTION patient_name_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p      jsonb := NEW.body;
    fld    text  := p ->> 'field';
    v_use  text  := NULLIF(trim(p -> 'facets' ->> 'use'), '');
    v_key  text;
    v_rank int;
BEGIN
    -- Only NAME events project here. dob/sex-at-birth (db/011) and any unknown field
    -- are ignored — names get their own multi-valued shape. (This trigger and the
    -- patient_demographic trigger both fire on demographic.field.asserted; each gates
    -- to its own fields and writes a different table, so order is irrelevant.)
    IF fld <> 'name' THEN
        RETURN NULL;
    END IF;
    v_key  := coalesce(v_use, 'unspecified');
    v_rank := cairn_provenance_rank(p ->> 'provenance');

    INSERT INTO patient_name AS pn
        (patient_id, use_key, value, use_raw, provenance, provenance_rank,
         last_hlc_wall, last_hlc_count, asserted_origin)
    VALUES
        (NEW.patient_id, v_key, p ->> 'value', v_use, p ->> 'provenance', v_rank,
         NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
    -- Per (patient, use, value) member, keep the MOST-RECENT assertion as its
    -- representative (recency-first tuple, matching the display rule). The compare is a
    -- deterministic, apply-order-independent function of the member's assertion set, so
    -- every node converges to the same row. A re-assertion that does not advance the
    -- tuple leaves the row unchanged (set-union idempotency).
    ON CONFLICT (patient_id, use_key, value) DO UPDATE SET
        use_raw         = EXCLUDED.use_raw,
        provenance      = EXCLUDED.provenance,
        provenance_rank = EXCLUDED.provenance_rank,
        last_hlc_wall   = EXCLUDED.last_hlc_wall,
        last_hlc_count  = EXCLUDED.last_hlc_count,
        asserted_origin = EXCLUDED.asserted_origin,
        updated_at      = clock_timestamp()
    WHERE (EXCLUDED.last_hlc_wall, EXCLUDED.last_hlc_count,
           EXCLUDED.provenance_rank, EXCLUDED.asserted_origin)
        > (pn.last_hlc_wall, pn.last_hlc_count,
           pn.provenance_rank, pn.asserted_origin);
    RETURN NULL;
END;
$$;

DROP TRIGGER IF EXISTS patient_name_apply_trg ON event_log;
CREATE TRIGGER patient_name_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'demographic.field.asserted')
    EXECUTE FUNCTION patient_name_apply();

-- The §4.2 display-winner: one row per patient, selected from the retained set with NO
-- stored pointer. The ORDER BY is the whole rule:
--   1) prefer use_key='legal' (a legal name always outranks any non-legal — a 2010 legal
--      beats a 2024 alias);
--   2) recency-first within the tier (newest legal name wins — recency beats provenance
--      for names, the deliberate divergence from DOB's provenance-lock);
--   3) provenance_rank then asserted_origin break exact-recency ties deterministically.
-- When no legal name exists, the newest name of ANY use wins (the unidentified-patient
-- fallback) — paper-parity: the chart header always shows something.
CREATE OR REPLACE VIEW patient_name_current AS
SELECT DISTINCT ON (patient_id)
    patient_id, use_key, value, use_raw, provenance, provenance_rank,
    last_hlc_wall, last_hlc_count, asserted_origin, updated_at
FROM patient_name
ORDER BY patient_id,
         (use_key = 'legal') DESC,
         last_hlc_wall DESC, last_hlc_count DESC,
         provenance_rank DESC, asserted_origin DESC;

GRANT SELECT ON patient_name, patient_name_current TO cairn_agent;

COMMIT;
