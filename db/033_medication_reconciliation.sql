-- 033_medication_reconciliation.sql — slice 3 of the clinical.medication surface.
--
-- Two append-only verbs over a canonical (low, high) medication_id thread pair:
--   clinical.medication-reconciliation.asserted — the two threads are the same real drug
--   clinical.medication-separation.asserted     — the never-erase reversal (two distinct drugs)
--
-- Mirrors the identity patient_link -> person_member connected-component machinery
-- (db/018), one level down over medication_id threads. Additive: a reconciliation
-- forecloses nothing (both threads' histories survive); ADR-0043's owner-gate does
-- not apply, so cross-author reconciliation is allowed. db/031 and db/032 are
-- untouched EXCEPT the three view reworks in part 3 (same-column-set replay rule).
-- See ADR-0047.
BEGIN;

-- 1. Register both verbs (fail-closed registry, ADR-0010). Additive, never targeting
--    another author (a reconciliation neither suppresses nor forecloses either thread).
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.medication-reconciliation.asserted', 'additive', FALSE),
    ('clinical.medication-separation.asserted',     'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. Structural floor for both verbs. Culture-neutral: two distinct valid UUID
--    subjects + a valid patient + non-empty provenance. Nothing clinical is blocked
--    (principle 4). Mirrors cairn_check_link_assertion.
CREATE OR REPLACE FUNCTION cairn_check_medication_reconciliation(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
    a text;
    c text;
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'medication reconciliation: missing payload';
    END IF;
    IF jsonb_typeof(p -> 'subject_a') IS DISTINCT FROM 'string'
       OR jsonb_typeof(p -> 'subject_b') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication reconciliation: subject_a and subject_b must be uuid strings';
    END IF;
    a := p ->> 'subject_a';
    c := p ->> 'subject_b';
    BEGIN
        PERFORM a::uuid;
        PERFORM c::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication reconciliation: subject_a/subject_b must be valid uuids';
    END;
    IF a::uuid = c::uuid THEN
        RAISE EXCEPTION 'medication reconciliation: self-reconcile refused (subjects must be distinct)';
    END IF;
    -- patient_id is a top-level envelope column; the floor sees the full body b.
    IF jsonb_typeof(b -> 'patient_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication reconciliation: patient_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (b ->> 'patient_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication reconciliation: patient_id must be a valid uuid';
    END;
    IF jsonb_typeof(p -> 'provenance') IS DISTINCT FROM 'string'
       OR length(btrim(p ->> 'provenance')) = 0 THEN
        RAISE EXCEPTION 'medication reconciliation: provenance must be a non-empty string (§4.1)';
    END IF;
END;
$$;

-- 3. Extend the shared twin hook. PRESERVES every existing branch from db/032
--    verbatim and adds ONLY the two reconciliation branches. submit_event itself is
--    never re-declared. Identity-critical linkage -> HARD-require an authored twin.
CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
    v_twin          text := b ->> 'plaintext_twin';
    v_authored      boolean := v_twin IS NOT NULL AND length(regexp_replace(v_twin, '\s+', '', 'g')) > 0;
    v_twin_required text := NULL;
BEGIN
    IF p_type = 'demographic.identifier.asserted' THEN
        PERFORM cairn_check_identifier_assertion(b);
        v_twin_required := 'demographic assertion requires a non-empty authored twin (§4.5)';
    ELSIF p_type = 'demographic.field.asserted' THEN
        PERFORM cairn_check_demographic_field(b);
        v_twin_required := 'demographic assertion requires a non-empty authored twin (§4.5)';
    ELSIF p_type IN ('identity.link.asserted', 'identity.unlink.asserted') THEN
        PERFORM cairn_check_link_assertion(b);
        v_twin_required := 'identity linkage assertion requires a non-empty authored twin (§5.7)';
    ELSIF p_type IN ('identity.dispute.asserted', 'identity.dispute.resolved') THEN
        PERFORM cairn_check_dispute_assertion(p_type, b);
        v_twin_required := 'identity dispute assertion requires a non-empty authored twin (§5.7)';
    ELSIF p_type IN ('identity.pending.asserted', 'identity.identify.asserted') THEN
        PERFORM cairn_check_identity_state_assertion(p_type, b);
        v_twin_required := 'identity-state assertion requires a non-empty authored twin (§5.7)';
    ELSIF p_type = 'identity.repudiate.asserted' THEN
        PERFORM cairn_check_repudiation_assertion(b);
        v_twin_required := 'identity repudiation assertion requires a non-empty authored twin (§5.7)';
    ELSIF p_type IN ('clinical.medication.asserted', 'clinical.medication-cessation.asserted') THEN
        PERFORM cairn_check_medication_assertion(p_type, b);
        v_twin_required := 'medication assertion requires a non-empty authored twin (§3.13/§3.15)';
    ELSIF p_type IN ('clinical.medication-dose-change.asserted', 'clinical.medication-dose-correction.asserted') THEN
        PERFORM cairn_check_medication_dose(p_type, b);
        v_twin_required := 'medication dose assertion requires a non-empty authored twin (§3.13/§3.15)';
    ELSIF p_type IN ('clinical.medication-reconciliation.asserted', 'clinical.medication-separation.asserted') THEN
        PERFORM cairn_check_medication_reconciliation(p_type, b);
        v_twin_required := 'medication reconciliation requires a non-empty authored twin (§3.13/§3.15)';
    END IF;

    IF v_authored THEN
        RETURN v_twin;
    END IF;
    IF v_twin_required IS NOT NULL THEN
        RAISE EXCEPTION 'submit_event: %', v_twin_required;
    END IF;
    RETURN cairn_twin_skeleton(p_type, b);
END;
$$;

COMMIT;

BEGIN;

-- 4. The standing-edge HLC overlay. One row per canonical (low, high) thread pair;
--    the latest-HLC assertion wins the state with the content_address tiebreak (#115).
--    Same shape/index discipline as db/018 patient_link.
CREATE TABLE IF NOT EXISTS medication_reconciliation (
    low             UUID    NOT NULL,
    high            UUID    NOT NULL,
    state           TEXT    NOT NULL CHECK (state IN ('reconciled', 'separated')),
    hlc_wall        BIGINT  NOT NULL,
    hlc_counter     INTEGER NOT NULL,
    origin          TEXT    NOT NULL,
    provenance      TEXT    NOT NULL,
    content_address BYTEA   NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (low, high),
    CHECK (low < high)
);
GRANT SELECT ON medication_reconciliation TO cairn_agent;
-- Index the high side so the component BFS is indexed in both directions.
CREATE INDEX IF NOT EXISTS medication_reconciliation_high_idx
    ON medication_reconciliation (high) WHERE state = 'reconciled';

-- 5. The golden-medication projection: medication_id -> group_id (min-UUID
--    representative of the connected component). A thread never touched by a
--    reconciliation event has no row and collapses to itself. Mirrors person_member.
CREATE TABLE IF NOT EXISTS medication_group_member (
    medication_id UUID PRIMARY KEY,
    group_id      UUID NOT NULL,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_group_member TO cairn_agent;

-- Oversize guard: a component larger than this is a pathology (mass false-merge);
-- refuse on local authoring, clamp-and-flag on remote apply (a node-local GUC must
-- not fork the event set between honest nodes). Mirrors cairn_max_component_size.
CREATE OR REPLACE FUNCTION cairn_max_medication_group_size()
RETURNS integer LANGUAGE sql STABLE AS $$
    SELECT COALESCE(NULLIF(current_setting('cairn.max_medication_group_size', true), '')::integer, 10000);
$$;

CREATE TABLE IF NOT EXISTS medication_projection_flag (
    flag_id       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    seed          UUID        NOT NULL,
    observed_size INTEGER     NOT NULL,
    cap           INTEGER     NOT NULL,
    flagged_at    TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_projection_flag TO cairn_agent;

-- Recompute the connected component around one seed thread over the STANDING
-- reconciled edges, rewriting medication_group_member to the min-UUID representative.
-- Cost bounded by the touched component, not the table (ADR-0001 incremental
-- discipline). Mirrors cairn_recompute_component.
CREATE OR REPLACE FUNCTION cairn_recompute_medication_group(p_seed uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_members uuid[];
    v_group   uuid;
BEGIN
    WITH RECURSIVE comp(node) AS (
        SELECT p_seed
        UNION
        SELECT CASE WHEN mr.low = comp.node THEN mr.high ELSE mr.low END
        FROM comp
        JOIN medication_reconciliation mr
          ON mr.state = 'reconciled' AND (mr.low = comp.node OR mr.high = comp.node)
    )
    SELECT array_agg(node) INTO v_members FROM comp;

    IF array_length(v_members, 1) > cairn_max_medication_group_size() THEN
        IF current_setting('cairn.remote_apply', true) = 'on' THEN
            INSERT INTO medication_projection_flag (seed, observed_size, cap)
            VALUES (p_seed, array_length(v_members, 1), cairn_max_medication_group_size());
            RETURN;
        END IF;
        RAISE EXCEPTION
            'medication reconciliation: group around % exceeds max size % — refusing to project (matcher pathology)',
            p_seed, cairn_max_medication_group_size();
    END IF;

    -- Canonical representative = the minimum UUID (uuid has a `<` operator but no
    -- min() aggregate). A seed with no edges maps to itself.
    v_group := (SELECT m FROM unnest(v_members) AS m ORDER BY m LIMIT 1);

    INSERT INTO medication_group_member (medication_id, group_id, updated_at)
    SELECT m, v_group, clock_timestamp() FROM unnest(v_members) AS m
    ON CONFLICT (medication_id) DO UPDATE SET
        group_id   = EXCLUDED.group_id,
        updated_at = clock_timestamp();
END;
$$;

-- Fold one reconciliation/separation event into the edge overlay, then recompute the
-- component around both endpoints. A txn-scoped advisory lock serializes recomputes
-- (the db/018 read-modify-write race). #157 collision recording is deferred (design §10).
CREATE OR REPLACE FUNCTION medication_reconciliation_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p       jsonb := NEW.body;
    a       uuid  := (p ->> 'subject_a')::uuid;
    b       uuid  := (p ->> 'subject_b')::uuid;
    lo      uuid  := LEAST(a, b);
    hi      uuid  := GREATEST(a, b);
    v_state text  := CASE WHEN NEW.event_type = 'clinical.medication-reconciliation.asserted'
                          THEN 'reconciled' ELSE 'separated' END;
BEGIN
    PERFORM pg_advisory_xact_lock(x'4341524E4D52'::bigint);  -- 'CARNMR'

    INSERT INTO medication_reconciliation
        (low, high, state, hlc_wall, hlc_counter, origin, provenance, content_address)
    VALUES
        (lo, hi, v_state, NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
         p ->> 'provenance', NEW.content_address)
    ON CONFLICT (low, high) DO UPDATE SET
        state       = EXCLUDED.state,
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        provenance  = EXCLUDED.provenance,
        content_address = EXCLUDED.content_address,
        updated_at  = clock_timestamp()
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        medication_reconciliation.hlc_wall, medication_reconciliation.hlc_counter,
        medication_reconciliation.origin, medication_reconciliation.content_address);

    -- Recompute BOTH endpoints (a reconcile merges; a separation splits into at most
    -- the piece with lo and the piece with hi — both reachable from these seeds).
    PERFORM cairn_recompute_medication_group(lo);
    PERFORM cairn_recompute_medication_group(hi);
    RETURN NULL;  -- AFTER trigger
END;
$$;
DROP TRIGGER IF EXISTS medication_reconciliation_apply_trg ON event_log;
CREATE TRIGGER medication_reconciliation_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type IN
        ('clinical.medication-reconciliation.asserted', 'clinical.medication-separation.asserted'))
    EXECUTE FUNCTION medication_reconciliation_apply();

COMMIT;
