-- Cairn — demographic ADDRESS: the §4.3 three-facet value (display/geo/structured).
--
-- Slice 5 of the demographics subsystem. An address reuses the slice-2 generic
-- `demographic.field.asserted` event with field='address'; `value` carries the
-- mandatory `display` string (the value-core), so the generic floor's non-empty-value
-- check already enforces "display non-empty". This migration adds NO new event type:
-- it (1) extends the shared structural floor with an address branch (structured⇒profile,
-- parts are text, geo shape) — culture-neutral, never holds a profile, never rejects on
-- validation; and (2) adds a retained-set table + a per-use display-winner VIEW
-- (recency-first within each use — addresses are volatile, so a fresh "I moved" must
-- beat a stale verified address, mirroring names/ADR-0036, NOT DOB's provenance-lock).
-- Matching (§5.2) is a separate, later subsystem.

BEGIN;

-- Extend the shared §4.2/§4.3 structural floor with the address branch. CREATE OR REPLACE
-- supersedes db/011's definition (latest-loaded wins — db/013 left this function
-- untouched), so the dob branch is carried forward VERBATIM and the generic checks
-- (payload/field/provenance/value all present and non-empty) are unchanged. The address
-- branch enforces ONLY culture-neutral structural shape: it never interprets a part name,
-- never holds a profile, never validates geo semantics (lat/lon bounds are advisory).
CREATE OR REPLACE FUNCTION cairn_check_demographic_field(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p     jsonb := b -> 'payload';
    fld   text;
    geo   jsonb;
    part  record;
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'demographic field assertion: missing payload';
    END IF;
    IF jsonb_typeof(p -> 'field') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'field')) = 0 THEN
        RAISE EXCEPTION 'demographic field assertion: field must be a non-empty string';
    END IF;
    IF jsonb_typeof(p -> 'provenance') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'provenance')) = 0 THEN
        RAISE EXCEPTION 'demographic field assertion: provenance must be a non-empty string (§4.1)';
    END IF;
    -- value: the core scalar (§4.2/§4.3). For an address this IS the mandatory `display`.
    IF jsonb_typeof(p -> 'value') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'value')) = 0 THEN
        RAISE EXCEPTION 'demographic field assertion: value must be a non-empty string';
    END IF;

    fld := p ->> 'field';
    -- dob (carried forward from db/011, unchanged): precision mandatory; basis text when present.
    IF fld = 'dob' THEN
        IF jsonb_typeof(p -> 'facets' -> 'precision') IS DISTINCT FROM 'string'
           OR length(trim(p -> 'facets' ->> 'precision')) = 0 THEN
            RAISE EXCEPTION 'demographic field assertion: dob requires a non-empty facets.precision (principle 4)';
        END IF;
        IF (p -> 'facets' ? 'basis') AND (p -> 'facets' -> 'basis') IS DISTINCT FROM 'null'::jsonb THEN
            IF jsonb_typeof(p -> 'facets' -> 'basis') IS DISTINCT FROM 'string'
               OR length(trim(p -> 'facets' ->> 'basis')) = 0 THEN
                RAISE EXCEPTION 'demographic field assertion: dob facets.basis must be non-empty text when present';
            END IF;
        END IF;
    -- address (§4.3): structured ⇒ profile present + parts are text; geo shape when present.
    ELSIF fld = 'address' THEN
        -- structured: when present, profile is a non-empty string and every part value is text.
        IF (p -> 'facets' ? 'structured')
           AND (p -> 'facets' -> 'structured') IS DISTINCT FROM 'null'::jsonb THEN
            IF jsonb_typeof(p -> 'facets' -> 'structured' -> 'profile') IS DISTINCT FROM 'string'
               OR length(trim(p -> 'facets' -> 'structured' ->> 'profile')) = 0 THEN
                RAISE EXCEPTION 'demographic field assertion: address structured requires a non-empty profile (§4.3)';
            END IF;
            IF (p -> 'facets' -> 'structured' ? 'parts')
               AND (p -> 'facets' -> 'structured' -> 'parts') IS DISTINCT FROM 'null'::jsonb THEN
                IF jsonb_typeof(p -> 'facets' -> 'structured' -> 'parts') IS DISTINCT FROM 'object' THEN
                    RAISE EXCEPTION 'demographic field assertion: address structured.parts must be an object';
                END IF;
                FOR part IN
                    SELECT value AS v
                    FROM jsonb_each(p -> 'facets' -> 'structured' -> 'parts')
                LOOP
                    IF jsonb_typeof(part.v) IS DISTINCT FROM 'string' THEN
                        RAISE EXCEPTION 'demographic field assertion: address structured.parts values must be text (opaque to the core)';
                    END IF;
                END LOOP;
            END IF;
        END IF;
        -- geo: when present, lat/lon are numbers, accuracy_m a non-negative number, basis non-empty text.
        IF (p -> 'facets' ? 'geo') AND (p -> 'facets' -> 'geo') IS DISTINCT FROM 'null'::jsonb THEN
            geo := p -> 'facets' -> 'geo';
            IF jsonb_typeof(geo -> 'lat') IS DISTINCT FROM 'number'
               OR jsonb_typeof(geo -> 'lon') IS DISTINCT FROM 'number' THEN
                RAISE EXCEPTION 'demographic field assertion: address geo.lat/geo.lon must be numbers';
            END IF;
            -- Two-step so the ::numeric cast only runs once the value is confirmed a JSON
            -- number: PostgreSQL does NOT guarantee short-circuit OR, so a single
            -- `typeof <> 'number' OR ::numeric < 0` could attempt the cast on a string
            -- (e.g. "north") and raise a raw cast error instead of this clean message.
            IF jsonb_typeof(geo -> 'accuracy_m') IS DISTINCT FROM 'number' THEN
                RAISE EXCEPTION 'demographic field assertion: address geo.accuracy_m must be a non-negative number';
            END IF;
            IF (geo ->> 'accuracy_m')::numeric < 0 THEN
                RAISE EXCEPTION 'demographic field assertion: address geo.accuracy_m must be a non-negative number';
            END IF;
            IF jsonb_typeof(geo -> 'basis') IS DISTINCT FROM 'string'
               OR length(trim(geo ->> 'basis')) = 0 THEN
                RAISE EXCEPTION 'demographic field assertion: address geo.basis must be non-empty text';
            END IF;
        END IF;
    END IF;
    -- unknown field: generic checks only — carried, legible, not projected.
END;
$$;

-- The §4.3 retained set: one row per distinct (patient, use, display) address. use_key
-- folds an absent/blank `use` to 'unspecified' and ASCII-lower-cases it (COLLATE "C")
-- exactly as patient_name does — `use` is an OPEN vocabulary, so "Residential"/"residential"
-- are one category, folded deterministically so the per-use winner and member dedup stay
-- convergent across the fleet (a locale lower() is collation-dependent). display is the
-- member discriminant (the value-core); geo/structured travel as the member's representative
-- facets, the most-recent assertion winning. provenance_rank is cached (reuses db/011's
-- cairn_provenance_rank) so the recency/provenance test is a plain tuple compare.
CREATE TABLE IF NOT EXISTS patient_address (
    patient_id         UUID    NOT NULL,
    use_key            TEXT    NOT NULL,   -- lower(coalesce(NULLIF(trim(use),''),'unspecified') COLLATE "C")
    display            TEXT    NOT NULL,   -- the mandatory human-readable address (value-core)
    use_raw            TEXT,               -- the original `use` facet (NULL when absent)
    geo                JSONB,              -- optional precision-aware geolocation facet
    structured         JSONB,              -- optional {profile, parts} facet
    provenance         TEXT    NOT NULL,
    provenance_rank    INT     NOT NULL,
    last_hlc_wall      BIGINT  NOT NULL,
    last_hlc_count     INTEGER NOT NULL,
    asserted_origin    TEXT    NOT NULL,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (patient_id, use_key, display)
);

-- Incremental maintenance: fold exactly the one new address event into the retained set.
-- event_log.body holds b->'payload' (see db/005 submit_event INSERT).
CREATE OR REPLACE FUNCTION patient_address_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p      jsonb := NEW.body;
    fld    text  := p ->> 'field';
    v_use  text  := NULLIF(trim(p -> 'facets' ->> 'use'), '');
    v_key  text;
    v_rank int;
BEGIN
    -- Only ADDRESS events project here. dob/sex-at-birth (db/011/013), name (db/012), and
    -- any unknown field are ignored — each projection gates to its own fields and writes a
    -- different table, so the several triggers on demographic.field.asserted are order-free.
    IF fld <> 'address' THEN
        RETURN NULL;
    END IF;
    v_key  := lower(coalesce(v_use, 'unspecified') COLLATE "C");
    v_rank := cairn_provenance_rank(p ->> 'provenance');

    INSERT INTO patient_address AS pa
        (patient_id, use_key, display, use_raw, geo, structured,
         provenance, provenance_rank, last_hlc_wall, last_hlc_count, asserted_origin)
    VALUES
        (NEW.patient_id, v_key, p ->> 'value', v_use,
         p -> 'facets' -> 'geo', p -> 'facets' -> 'structured',
         p ->> 'provenance', v_rank, NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
    -- Per (patient, use, display) member, keep the MOST-RECENT assertion as its
    -- representative (recency-first tuple — matches the display rule). The compare is a
    -- deterministic, apply-order-independent function of the member's assertion set, so
    -- every node converges. A re-assertion that does not advance the tuple is a no-op
    -- (set-union idempotency).
    ON CONFLICT (patient_id, use_key, display) DO UPDATE SET
        use_raw         = EXCLUDED.use_raw,
        geo             = EXCLUDED.geo,
        structured      = EXCLUDED.structured,
        provenance      = EXCLUDED.provenance,
        provenance_rank = EXCLUDED.provenance_rank,
        last_hlc_wall   = EXCLUDED.last_hlc_wall,
        last_hlc_count  = EXCLUDED.last_hlc_count,
        asserted_origin = EXCLUDED.asserted_origin,
        updated_at      = clock_timestamp()
    WHERE (EXCLUDED.last_hlc_wall, EXCLUDED.last_hlc_count,
           EXCLUDED.provenance_rank, EXCLUDED.asserted_origin)
        > (pa.last_hlc_wall, pa.last_hlc_count,
           pa.provenance_rank, pa.asserted_origin);
    RETURN NULL;
END;
$$;

DROP TRIGGER IF EXISTS patient_address_apply_trg ON event_log;
CREATE TRIGGER patient_address_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'demographic.field.asserted')
    EXECUTE FUNCTION patient_address_apply();

-- The §4.3 per-use display-winner: ONE row per (patient, use), selected from the retained
-- set with NO stored pointer. The ORDER BY is the whole rule: recency-first within the use
-- (newest address wins — recency beats provenance for a volatile field, the deliberate
-- divergence from DOB's provenance-lock), with provenance_rank then asserted_origin as
-- deterministic tiebreaks so every node converges to the same current address per use.
CREATE OR REPLACE VIEW patient_address_current AS
SELECT DISTINCT ON (patient_id, use_key)
    patient_id, use_key, display, use_raw, geo, structured,
    provenance, provenance_rank, last_hlc_wall, last_hlc_count, asserted_origin, updated_at
FROM patient_address
ORDER BY patient_id, use_key,
         last_hlc_wall DESC, last_hlc_count DESC,
         provenance_rank DESC, asserted_origin DESC;

GRANT SELECT ON patient_address, patient_address_current TO cairn_agent;

COMMIT;
