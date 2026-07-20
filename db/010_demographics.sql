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
-- Signature unified to (p_type text, b jsonb) for the #173 registry dispatch; p_type is
-- unused here (this check validates the body). DROP clears any stale (jsonb) overload on
-- an upgraded-in-place dev DB.
DROP FUNCTION IF EXISTS cairn_check_identifier_assertion(jsonb);
CREATE OR REPLACE FUNCTION cairn_check_identifier_assertion(p_type text, b jsonb)
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

-- Register this type's structural floor + hard twin requirement in the #173 registry
-- (replaces the copied cairn_event_twin dispatch chain; the single db/005 dispatcher reads
-- this row). Placed after the floor fn above so the fail-closed registry trigger (db/005)
-- sees cairn_check_identifier_assertion(text, jsonb) already declared at load time.
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('demographic.identifier.asserted', 'cairn_check_identifier_assertion',
     'demographic assertion requires a non-empty authored twin (§4.5)')
ON CONFLICT (event_type) DO NOTHING;

-- The §4.2 set-union projection: one row per (patient, system, match_key). Identifiers
-- are set-union: same-system / different-normalized keeps BOTH rows (the veto SIGNAL
-- preserved as data; the veto itself is out of scope). Within ONE (system, match_key)
-- member, the representative is the HLC-latest assertion (deterministic overlay), NOT
-- first-applied — see the apply function. `use` is a reserved word, so it is `use_type`.
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
    content_address    BYTEA,              -- winning event's content address; #194 tiebreak
    first_seen         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (patient_id, system, match_key)
);
-- Additive widening (issue #194, same discipline as #207): the CREATE above no-ops on an
-- existing table, so the column must ALSO ship as an idempotent ALTER. Nullable: pre-#194
-- rows degrade honestly to the first-applied tie behavior; every new upsert writes it.
-- Guarded by migration_replay_widening.rs.
ALTER TABLE patient_identifier ADD COLUMN IF NOT EXISTS content_address BYTEA;

-- Incremental set-union maintenance: fold exactly the one new identifier event into
-- the projection. event_log.body holds b->'payload' (see db/005 submit_event INSERT).
--
-- The per-type trigger is superseded by cairn_projection_dispatch_trg (db/005,
-- ADR-0057); this fn is now registered in cairn_projection_apply below.
DROP TRIGGER IF EXISTS patient_identifier_apply_trg ON event_log;
-- The old zero-arg trigger-function signature is superseded by the (event_log)
-- apply-fn signature below; CREATE OR REPLACE cannot change a function's arg
-- list (it would overload, not replace), so drop the old signature explicitly
-- (same idiom as db/005's `DROP FUNCTION IF EXISTS submit_event(bytea, bytea, bytea);`).
DROP FUNCTION IF EXISTS patient_identifier_apply();

CREATE OR REPLACE FUNCTION patient_identifier_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p    jsonb := e.body;
    norm text  := NULLIF(p ->> 'normalized', '');
BEGIN
    -- ADR-0052 §2 seal-robustness (#10): a wrongly-sealed NON-clinical row holds CIPHERTEXT
    -- in e.body (refused at submit; admitted lenient at apply for lossless sync). Reading it
    -- below would drive NULLs into this projection and freeze the sync watermark — so a sealed
    -- row projects NOTHING (harmless ciphertext noise; no custody, no leak).
    IF e.sealed THEN RETURN; END IF;
    INSERT INTO patient_identifier
        (patient_id, system, match_key, value, normalized, profile, use_type,
         provenance, asserted_hlc_wall, asserted_hlc_count, asserted_origin,
         content_address)
    VALUES
        (e.patient_id, p ->> 'system', COALESCE(norm, p ->> 'value'),
         p ->> 'value', norm, p ->> 'profile', p ->> 'use', p ->> 'provenance',
         e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    -- CONVERGENCE FIX: DO NOTHING kept the FIRST-APPLIED row, whose non-key columns
    -- (value, provenance, ...) can differ between two assertions that share a match_key
    -- (e.g. "943 476 5919" vs "9434765919", or patient-stated then document-verified).
    -- "First applied" is node-local apply ORDER, not a function of the event set, so two
    -- honest nodes could keep DIFFERENT rows for the same patient — and the db/016 veto
    -- reads .value/.normalized, so they could then compute DIFFERENT hard-veto verdicts.
    -- Keep the HLC-latest assertion as the deterministic representative instead (the same
    -- apply-order-independent overlay every other demographic projection uses), with
    -- `value` then `content_address` as the final total-order tiebreaks. `value` alone was
    -- NOT total (#194/finding A6): two events sharing (triple, value) but differing in
    -- use/profile compared equal in both directions, so first-applied won and two honest
    -- nodes could keep different rows — and the db/016 veto floor reads this row, so they
    -- could compute DIFFERENT hard-veto verdicts from the same event set. content_address
    -- is unique per distinct event (bytea, byte-order — no collation), closing the gap.
    -- A NULL incumbent address (pre-#194 row) leaves the incumbent in place — the honest
    -- pre-#194 behavior for legacy ties only; every new write stamps the column.
    ON CONFLICT (patient_id, system, match_key) DO UPDATE SET
        value              = EXCLUDED.value,
        normalized         = EXCLUDED.normalized,
        profile            = EXCLUDED.profile,
        use_type           = EXCLUDED.use_type,
        provenance         = EXCLUDED.provenance,
        asserted_hlc_wall  = EXCLUDED.asserted_hlc_wall,
        asserted_hlc_count = EXCLUDED.asserted_hlc_count,
        asserted_origin    = EXCLUDED.asserted_origin,
        content_address    = EXCLUDED.content_address
    WHERE (EXCLUDED.asserted_hlc_wall, EXCLUDED.asserted_hlc_count,
           EXCLUDED.asserted_origin COLLATE "C", EXCLUDED.value COLLATE "C",
           EXCLUDED.content_address)
        > (patient_identifier.asserted_hlc_wall, patient_identifier.asserted_hlc_count,
           patient_identifier.asserted_origin COLLATE "C", patient_identifier.value COLLATE "C",
           patient_identifier.content_address);
    RETURN;
END;
$$;

-- A trigger fn could never be called directly; a plain fn gets PUBLIC EXECUTE by
-- default. Same discipline as every privileged fn in db/005 (Task-1 review finding).
REVOKE EXECUTE ON FUNCTION patient_identifier_apply(event_log) FROM PUBLIC;

GRANT SELECT ON patient_identifier TO cairn_agent;

-- Registered apply fn for the #208/ADR-0057 generic dispatcher (db/005) + cairn_reproject
-- heal/rebuild (db/039). #214 + steady-state discipline: converge this row to the migration
-- text on every connect, but stay write-free once already converged (no dead tuples, no
-- validate-trigger fire).
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe)
VALUES ('demographic.identifier.asserted', 'patient_identifier_apply', ARRAY['patient_identifier'], 10, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
