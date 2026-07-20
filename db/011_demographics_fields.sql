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
-- Signature unified to (p_type text, b jsonb) for the #173 registry dispatch; p_type is
-- unused here (this check validates the body). DROP clears any stale (jsonb) overload on
-- an upgraded-in-place dev DB (this is the EARLIEST declaration; db/014 re-declares the
-- unified signature without re-dropping).
DROP FUNCTION IF EXISTS cairn_check_demographic_field(jsonb);
CREATE OR REPLACE FUNCTION cairn_check_demographic_field(p_type text, b jsonb)
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

-- Register this type's structural floor + hard twin requirement in the #173 registry
-- (replaces the copied cairn_event_twin dispatch chain; the single db/005 dispatcher reads
-- this row). Placed after the floor fn above so the fail-closed registry trigger (db/005)
-- sees cairn_check_demographic_field(text, jsonb) already declared at load time.
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('demographic.field.asserted', 'cairn_check_demographic_field',
     'demographic assertion requires a non-empty authored twin (§4.5)')
ON CONFLICT (event_type) DO NOTHING;

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
    content_address    BYTEA,              -- winning event's content address; #194 tiebreak
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (patient_id, field)
);
-- Additive widening (issue #194, same discipline as #207): the CREATE above no-ops on an
-- existing table, so the column must ALSO ship as an idempotent ALTER. Nullable: pre-#194
-- rows degrade honestly to the first-applied tie behavior; every new upsert writes it.
-- Guarded by migration_replay_widening.rs.
ALTER TABLE patient_demographic ADD COLUMN IF NOT EXISTS content_address BYTEA;

-- Incremental maintenance: fold exactly the one new field event into the projection.
-- event_log.body holds b->'payload' (see db/005 submit_event INSERT).
--
-- The per-type trigger is superseded by cairn_projection_dispatch_trg (db/005,
-- ADR-0057); this fn is now registered in cairn_projection_apply below.
DROP TRIGGER IF EXISTS patient_demographic_apply_trg ON event_log;
-- The old zero-arg trigger-function signature is superseded by the (event_log)
-- apply-fn signature below; CREATE OR REPLACE cannot change a function's arg
-- list (it would overload, not replace), so drop the old signature explicitly
-- (same idiom as db/005's `DROP FUNCTION IF EXISTS submit_event(bytea, bytea, bytea);`).
DROP FUNCTION IF EXISTS patient_demographic_apply();

CREATE OR REPLACE FUNCTION patient_demographic_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p      jsonb := e.body;
    fld    text  := p ->> 'field';
    v_rank int   := cairn_provenance_rank(p ->> 'provenance');
BEGIN
    -- ADR-0052 §2 seal-robustness (#10): a wrongly-sealed NON-clinical row holds CIPHERTEXT
    -- in e.body (refused at submit; admitted lenient at apply for lossless sync). Reading it
    -- below would drive NULLs into this projection and freeze the sync watermark — so a sealed
    -- row projects NOTHING (harmless ciphertext noise; no custody, no leak).
    IF e.sealed THEN RETURN; END IF;
    -- Projection gate: only known single-valued fields project. An unknown field
    -- (e.g. a newer node's gender-identity) is already in event_log and legible via
    -- its twin; it simply has no projection policy here. Required for set-union
    -- federation (ADR-0012) — never reject (that is the floor's job and it doesn't),
    -- never project a field we have no winner-policy for.
    IF fld NOT IN ('dob', 'sex-at-birth') THEN
        RETURN;
    END IF;

    INSERT INTO patient_demographic AS pd
        (patient_id, field, value, facets, provenance, provenance_rank,
         asserted_hlc_wall, asserted_hlc_count, asserted_origin, content_address)
    VALUES
        (e.patient_id, fld, p ->> 'value', p -> 'facets', p ->> 'provenance', v_rank,
         e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
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
        content_address    = EXCLUDED.content_address,
        updated_at         = clock_timestamp()
    -- `value` then `content_address` are the FINAL total-order tiebreaks:
    -- (rank,wall,counter,origin) is unique per event only while nodes stamp distinct HLC
    -- tuples, and `value` alone was not total either (#194/finding A6: two events sharing
    -- triple AND value but differing in facets compared equal both ways, so first-applied
    -- won — and the db/016 veto floor reads the winner's facets/provenance_rank, so two
    -- honest nodes could compute DIFFERENT hard-veto verdicts). content_address is unique
    -- per distinct event (bytea byte-order, no collation), making the winner convergent
    -- unconditionally. COLLATE "C" per ADR-0045; this body is superseded by db/013.
    WHERE (EXCLUDED.provenance_rank, EXCLUDED.asserted_hlc_wall,
           EXCLUDED.asserted_hlc_count, EXCLUDED.asserted_origin COLLATE "C", EXCLUDED.value COLLATE "C",
           EXCLUDED.content_address)
        > (pd.provenance_rank, pd.asserted_hlc_wall,
           pd.asserted_hlc_count, pd.asserted_origin COLLATE "C", pd.value COLLATE "C",
           pd.content_address);
    RETURN;
END;
$$;

-- A trigger fn could never be called directly; a plain fn gets PUBLIC EXECUTE by
-- default. Same discipline as every privileged fn in db/005 (Task-1 review finding).
REVOKE EXECUTE ON FUNCTION patient_demographic_apply(event_log) FROM PUBLIC;

GRANT SELECT ON patient_demographic TO cairn_agent;

-- Registered apply fn for the #208/ADR-0057 generic dispatcher (db/005) + cairn_reproject
-- heal/rebuild (db/039). #214 + steady-state discipline: converge this row to the migration
-- text on every connect, but stay write-free once already converged (no dead tuples, no
-- validate-trigger fire).
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe)
VALUES ('demographic.field.asserted', 'patient_demographic_apply', ARRAY['patient_demographic'], 20, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
