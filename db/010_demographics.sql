-- Cairn — demographic identifier assertions (spec §4.1/§4.4/§4.5, ADR-0033/0034).
--
-- The first production clinical surface. Adds the `demographic.identifier.asserted`
-- event type, the §4.4 structural floor (culture-neutral: no profile, no checksum,
-- no format validation — those are advisory and live above the floor), the §4.5
-- authored-twin carry (added via the cairn_event_twin hook, NOT by re-declaring the
-- validated submit_event door), and a set-union `patient_identifier` projection.
-- Matching/veto (§5.2) is a separate, later subsystem and NOT here.

BEGIN;

-- Additive registration: a new event type adds a row (fail-closed registry, ADR-0010).
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('demographic.identifier.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- The §4.4 structural floor. Enforces ONLY culture-neutral invariants; never holds a
-- profile, runs a checksum, or validates a format (those flag-not-reject above the
-- floor — principle 12 / §4.4). Each violation is a distinct legible exception.
CREATE OR REPLACE FUNCTION cairn_check_identifier_assertion(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'identifier assertion: missing payload';
    END IF;
    -- value: present, string, non-empty (§4.4 mandatory, the evidence facet).
    IF jsonb_typeof(p -> 'value') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'value')) = 0 THEN
        RAISE EXCEPTION 'identifier assertion: value must be a non-empty string (§4.4 mandatory)';
    END IF;
    -- system: present, string, non-empty (§4.4 mandatory; may be the literal "unknown").
    IF jsonb_typeof(p -> 'system') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'system')) = 0 THEN
        RAISE EXCEPTION 'identifier assertion: system must be a non-empty string (§4.4 mandatory)';
    END IF;
    -- provenance: present, string, non-empty (§4.1 ladder; value-open, "unknown" is honest).
    IF jsonb_typeof(p -> 'provenance') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'provenance')) = 0 THEN
        RAISE EXCEPTION 'identifier assertion: provenance must be a non-empty string (§4.1)';
    END IF;
    -- normalized: optional; when present must be a string AND name a profile
    -- (the §4.4 materialised-key rule: a materialised matching key needs the bundle
    -- that produced it, so a profile-less node can trust it).
    IF (p ? 'normalized') AND (p -> 'normalized') IS DISTINCT FROM 'null'::jsonb THEN
        -- Trim-checked like value/system/provenance above: a whitespace-only key is
        -- not a real materialised key, and would otherwise become a whitespace
        -- match_key in patient_identifier, silently conflating distinct identifiers.
        IF jsonb_typeof(p -> 'normalized') IS DISTINCT FROM 'string'
           OR length(trim(p ->> 'normalized')) = 0 THEN
            RAISE EXCEPTION 'identifier assertion: normalized must be a non-empty string when present (§4.4)';
        END IF;
        IF jsonb_typeof(p -> 'profile') IS DISTINCT FROM 'string'
           OR length(trim(p ->> 'profile')) = 0 THEN
            RAISE EXCEPTION 'identifier assertion: normalized materialised requires a named profile (§4.4)';
        END IF;
    END IF;
END;
$$;

-- The §4.2 set-union projection: one row per (patient, system, match_key). Identifiers
-- are set-union, never LWW — first-seen wins, re-assertion is a no-op, same-system /
-- different-normalized keeps BOTH rows (the veto SIGNAL preserved as data; the veto
-- itself is out of scope). `use` is a reserved word, so the column is `use_type`.
CREATE TABLE IF NOT EXISTS patient_identifier (
    patient_id         UUID    NOT NULL,
    system             TEXT    NOT NULL,
    match_key          TEXT    NOT NULL,   -- coalesce(normalized, value)
    value              TEXT    NOT NULL,
    normalized         TEXT,
    profile            TEXT,
    use_type           TEXT,
    provenance         TEXT    NOT NULL,
    asserted_hlc_wall  BIGINT  NOT NULL,
    asserted_hlc_count INTEGER NOT NULL,
    asserted_origin    TEXT    NOT NULL,
    first_seen         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (patient_id, system, match_key)
);

-- Incremental set-union maintenance: fold exactly the one new identifier event into
-- the projection. event_log.body holds b->'payload' (see db/005 submit_event INSERT).
CREATE OR REPLACE FUNCTION patient_identifier_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p    jsonb := NEW.body;
    norm text  := NULLIF(p ->> 'normalized', '');
BEGIN
    INSERT INTO patient_identifier
        (patient_id, system, match_key, value, normalized, profile, use_type,
         provenance, asserted_hlc_wall, asserted_hlc_count, asserted_origin)
    VALUES
        (NEW.patient_id, p ->> 'system', COALESCE(norm, p ->> 'value'),
         p ->> 'value', norm, p ->> 'profile', p ->> 'use', p ->> 'provenance',
         NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
    ON CONFLICT (patient_id, system, match_key) DO NOTHING;
    RETURN NULL;
END;
$$;

DROP TRIGGER IF EXISTS patient_identifier_apply_trg ON event_log;
CREATE TRIGGER patient_identifier_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'demographic.identifier.asserted')
    EXECUTE FUNCTION patient_identifier_apply();

GRANT SELECT ON patient_identifier TO cairn_agent;

-- Demographics' ONLY change to the write path: extend the twin hook (NOT submit_event)
-- for the identifier assertion. submit_event (db/005) is reused verbatim — never
-- re-declared — so the validated door stays single-source and cannot drift. This
-- CREATE OR REPLACE runs after db/005's default, adding the demographic branch and
-- falling back to the skeleton (via cairn_twin_skeleton) for every other type.
CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
    v_twin text;
BEGIN
    IF p_type = 'demographic.identifier.asserted' THEN
        -- §4.4 structural floor + §4.5 authored twin: a demographic assertion carries
        -- its own legibility twin (never derived) and must pass the culture-neutral
        -- floor. An empty authored twin is refused (§4.5).
        PERFORM cairn_check_identifier_assertion(b);
        v_twin := b ->> 'plaintext_twin';
        IF v_twin IS NULL OR length(trim(v_twin)) = 0 THEN
            RAISE EXCEPTION 'submit_event: demographic assertion requires a non-empty authored twin (§4.5)';
        END IF;
        RETURN v_twin;
    END IF;
    RETURN cairn_twin_skeleton(p_type, b);
END;
$$;

COMMIT;
