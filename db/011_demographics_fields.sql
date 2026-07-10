-- Cairn — demographic provenance-precedence fields: DOB + sex-at-birth (spec §4.1/§4.2/§4.5).
--
-- Slice 2 of the demographics subsystem. Adds the generic `demographic.field.asserted`
-- event type, the culture-neutral §4.2 structural floor (no date parsing, no sex
-- vocabulary — those are advisory, above the floor), the §4.1 provenance ladder as a
-- rank function, and the winner-by-(rank, HLC) `patient_demographic` projection. The
-- §4.5 authored twin is carried via the cairn_event_twin hook (NOT by re-declaring the
-- validated submit_event door). Matching (§5.2) is a separate, later subsystem.

BEGIN;

-- Additive registration of the new event type (fail-closed registry, ADR-0010).
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('demographic.field.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- The §4.1 provenance ladder as a total order. fact-proven (70) is a new top tier
-- above document-verified (60): laboratory/scientifically-established truth (a
-- karyotype, a confirmed assay) can override what an official document merely
-- attests. An UNRECOGNIZED string ranks 0 (below inferred) — the safe default: a
-- term from a newer ladder, or a typo, can never DISPLACE a known-provenance value,
-- and a node that doesn't know a peer's newer term degrades to "lowest", never
-- "highest" (federation-safe). IMMUTABLE so it is index/trigger-safe.
CREATE OR REPLACE FUNCTION cairn_provenance_rank(p text)
RETURNS int LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE p
        WHEN 'fact-proven'        THEN 70
        WHEN 'document-verified'  THEN 60
        WHEN 'patient-stated'     THEN 50
        WHEN 'third-party-stated' THEN 40
        WHEN 'clinician-observed' THEN 30
        WHEN 'imported'           THEN 20
        WHEN 'unknown'            THEN 20
        WHEN 'inferred'           THEN 10
        ELSE 0
    END;
$$;

-- The §4.2 structural floor for a generic demographic field assertion. Enforces ONLY
-- culture-neutral invariants; never parses a date, never validates a sex vocabulary,
-- never rejects on validation (principle 12). Per-field structural checks apply only
-- to fields THIS node knows — an unknown field passes the generic checks (it is still
-- stored in event_log and legible via its twin; the PROJECTION, not the floor, is what
-- is gated to known fields). Each violation is a distinct legible exception.
CREATE OR REPLACE FUNCTION cairn_check_demographic_field(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p   jsonb := b -> 'payload';
    fld text;
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'demographic field assertion: missing payload';
    END IF;
    -- field: the discriminator the projection keys on (§4.2).
    IF jsonb_typeof(p -> 'field') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'field')) = 0 THEN
        RAISE EXCEPTION 'demographic field assertion: field must be a non-empty string';
    END IF;
    -- provenance: the §4.1 ladder term — required-present, value-open.
    IF jsonb_typeof(p -> 'provenance') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'provenance')) = 0 THEN
        RAISE EXCEPTION 'demographic field assertion: provenance must be a non-empty string (§4.1)';
    END IF;
    -- value: the core scalar (§4.2). Open string — never a closed enum.
    IF jsonb_typeof(p -> 'value') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'value')) = 0 THEN
        RAISE EXCEPTION 'demographic field assertion: value must be a non-empty string';
    END IF;

    fld := p ->> 'field';
    -- Per-field structural dispatch (known fields only).
    IF fld = 'dob' THEN
        -- precision is mandatory: a date must declare how precise it is (principle 4 —
        -- never an unqualified exact date by default). The floor does NOT parse the
        -- date value — a half-recalled "1980, year-only" must record.
        IF jsonb_typeof(p -> 'facets' -> 'precision') IS DISTINCT FROM 'string'
           OR length(trim(p -> 'facets' ->> 'precision')) = 0 THEN
            RAISE EXCEPTION 'demographic field assertion: dob requires a non-empty facets.precision (principle 4)';
        END IF;
        -- basis is optional; when present it must be non-empty text.
        IF (p -> 'facets' ? 'basis') AND (p -> 'facets' -> 'basis') IS DISTINCT FROM 'null'::jsonb THEN
            IF jsonb_typeof(p -> 'facets' -> 'basis') IS DISTINCT FROM 'string'
               OR length(trim(p -> 'facets' ->> 'basis')) = 0 THEN
                RAISE EXCEPTION 'demographic field assertion: dob facets.basis must be non-empty text when present';
            END IF;
        END IF;
    END IF;
    -- sex-at-birth: no extra structural requirement (value-open).
    -- unknown field: generic checks only — carried, legible, not projected.
END;
$$;

-- The §4.2 provenance-precedence projection: one row per (patient, field) holding the
-- current DISPLAY winner. Full assertion history (the matching evidence) stays in
-- event_log — this is the projected current truth, an overlay, never an edit
-- (principle 2). provenance_rank is cached so the trigger's winner test is a plain
-- tuple compare. `value` is the core scalar; `facets` carries field-specific extras.
CREATE TABLE IF NOT EXISTS patient_demographic (
    patient_id         UUID    NOT NULL,
    field              TEXT    NOT NULL,   -- 'dob' | 'sex-at-birth' (known fields only)
    value              TEXT    NOT NULL,
    facets             JSONB,
    provenance         TEXT    NOT NULL,
    provenance_rank    INT     NOT NULL,
    asserted_hlc_wall  BIGINT  NOT NULL,
    asserted_hlc_count INTEGER NOT NULL,
    asserted_origin    TEXT    NOT NULL,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (patient_id, field)
);

-- Incremental maintenance: fold exactly the one new field event into the projection.
-- event_log.body holds b->'payload' (see db/005 submit_event INSERT).
CREATE OR REPLACE FUNCTION patient_demographic_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p      jsonb := NEW.body;
    fld    text  := p ->> 'field';
    v_rank int   := cairn_provenance_rank(p ->> 'provenance');
BEGIN
    -- Projection gate: only known single-valued fields project. An unknown field
    -- (e.g. a newer node's gender-identity) is already in event_log and legible via
    -- its twin; it simply has no projection policy here. Required for set-union
    -- federation (ADR-0012) — never reject (that is the floor's job and it doesn't),
    -- never project a field we have no winner-policy for.
    IF fld NOT IN ('dob', 'sex-at-birth') THEN
        RETURN NULL;
    END IF;

    INSERT INTO patient_demographic AS pd
        (patient_id, field, value, facets, provenance, provenance_rank,
         asserted_hlc_wall, asserted_hlc_count, asserted_origin)
    VALUES
        (NEW.patient_id, fld, p ->> 'value', p -> 'facets', p ->> 'provenance', v_rank,
         NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
    -- Winner = max (provenance_rank, then HLC recency, then node_origin). Provenance
    -- beats recency (rank leads the tuple), so a later lower-provenance assertion
    -- cannot displace an earlier higher-provenance one ("verified value locks"); a
    -- later EQUAL-provenance assertion wins on HLC. node_origin is the final
    -- deterministic tiebreak, so every node converges to the same winner regardless
    -- of apply order. The WHERE gates the overlay: if the incoming row does not
    -- outrank the incumbent, the row is left unchanged.
    ON CONFLICT (patient_id, field) DO UPDATE SET
        value              = EXCLUDED.value,
        facets             = EXCLUDED.facets,
        provenance         = EXCLUDED.provenance,
        provenance_rank    = EXCLUDED.provenance_rank,
        asserted_hlc_wall  = EXCLUDED.asserted_hlc_wall,
        asserted_hlc_count = EXCLUDED.asserted_hlc_count,
        asserted_origin    = EXCLUDED.asserted_origin,
        updated_at         = clock_timestamp()
    -- `value` is the FINAL total-order tiebreak: (rank,wall,counter,origin) is unique per
    -- event only while nodes stamp distinct HLC tuples; a buggy node minting a duplicate
    -- HLC would otherwise leave the winner apply-order-dependent (cross-node divergence).
    -- With value appended the projected winner is display-convergent unconditionally.
    -- COLLATE "C" tiebreak per ADR-0045 (#69); this body is superseded by db/013.
    WHERE (EXCLUDED.provenance_rank, EXCLUDED.asserted_hlc_wall,
           EXCLUDED.asserted_hlc_count, EXCLUDED.asserted_origin COLLATE "C", EXCLUDED.value COLLATE "C")
        > (pd.provenance_rank, pd.asserted_hlc_wall,
           pd.asserted_hlc_count, pd.asserted_origin COLLATE "C", pd.value COLLATE "C");
    RETURN NULL;
END;
$$;

DROP TRIGGER IF EXISTS patient_demographic_apply_trg ON event_log;
CREATE TRIGGER patient_demographic_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'demographic.field.asserted')
    EXECUTE FUNCTION patient_demographic_apply();

GRANT SELECT ON patient_demographic TO cairn_agent;

-- Demographics' ONLY change to the write path: extend the twin hook (NOT submit_event)
-- to dispatch BOTH demographic event types through their structural floor, then a
-- single shared §4.5 authored-twin enforcement. This supersedes db/010's definition
-- (latest-loaded wins — the standard additive-migration pattern); the identifier
-- branch behaves identically. Legacy types fall back to the derived skeleton twin.
CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
    v_twin text;
BEGIN
    IF p_type = 'demographic.identifier.asserted' THEN
        PERFORM cairn_check_identifier_assertion(b);
    ELSIF p_type = 'demographic.field.asserted' THEN
        PERFORM cairn_check_demographic_field(b);
    ELSE
        RETURN cairn_twin_skeleton(p_type, b);
    END IF;
    -- Shared §4.5 authored-twin enforcement for every demographic assertion (written
    -- once, not duplicated per branch): the twin is materialised at authoring, so an
    -- empty/absent twin on a demographic event is refused.
    v_twin := b ->> 'plaintext_twin';
    IF v_twin IS NULL OR length(trim(v_twin)) = 0 THEN
        RAISE EXCEPTION 'submit_event: demographic assertion requires a non-empty authored twin (§4.5)';
    END IF;
    RETURN v_twin;
END;
$$;

COMMIT;
