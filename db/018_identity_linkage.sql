-- db/018_identity_linkage.sql
-- Cairn — §5.1/§5.7 identity linkage core (matcher piece C1).
--
-- WHAT: the authoritative destination for identity linkage. Adds the additive
-- `identity.link.asserted` / `identity.unlink.asserted` event types, a
-- culture-neutral structural floor, an HLC-overlay `patient_link` edge table, and
-- a `person_member` connected-component ("golden identity") projection with clean
-- unmerge (principle 2 — never merge, always link; unmerge is always clean).
--
-- The safety-critical write door submit_event (db/005) is REUSED verbatim: new
-- types register in event_type_class and add a branch to the cairn_event_twin hook.
-- Advisory matching (§5.2) and the proposal→apply seam (C2) are NOT here.

BEGIN;

-- 1. Register the two additive identity event types (fail-closed registry, ADR-0010).
--    additive + targets_other_author=FALSE: a link neither suppresses nor targets
--    another author's event, so the existing gate requires NO attestation for a
--    matcher-authored link (§5.2 "auto above threshold"); a human who vouches simply
--    includes a responsibility-bearing contributor, which the gate already attests.
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('identity.link.asserted',   'additive', FALSE),
    ('identity.unlink.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. The §5.7 structural floor. Culture-neutral: validates STRUCTURE only — two
--    distinct valid UUID subjects and a non-empty provenance. Each violation is a
--    distinct legible exception (the cairn_check_identifier_assertion pattern).
CREATE OR REPLACE FUNCTION cairn_check_link_assertion(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
    a text;
    c text;
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'link assertion: missing payload';
    END IF;
    -- subject_a / subject_b: present, string.
    IF jsonb_typeof(p -> 'subject_a') IS DISTINCT FROM 'string'
       OR jsonb_typeof(p -> 'subject_b') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'link assertion: subject_a and subject_b must be uuid strings (§5.7)';
    END IF;
    a := p ->> 'subject_a';
    c := p ->> 'subject_b';
    -- ...valid UUIDs (a bad cast here is a legible reject, not an opaque crash).
    BEGIN
        PERFORM a::uuid;
        PERFORM c::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'link assertion: subject_a/subject_b must be valid uuids (§5.7)';
    END;
    -- ...and distinct (a self-link is meaningless and would corrupt the component walk).
    IF a::uuid = c::uuid THEN
        RAISE EXCEPTION 'link assertion: self-link refused (subject_a = subject_b) (§5.1)';
    END IF;
    -- provenance: present, non-empty (§4.1 ladder; value-open, "unknown" is honest).
    IF jsonb_typeof(p -> 'provenance') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'provenance')) = 0 THEN
        RAISE EXCEPTION 'link assertion: provenance must be a non-empty string (§4.1)';
    END IF;
    -- confidence: optional; when present must not be JSON null (omit-when-absent
    -- discipline — a null confidence is a serialization bug, not "unknown", which is
    -- expressed by omitting the key; principle 4).
    IF (p ? 'confidence') AND (p -> 'confidence') = 'null'::jsonb THEN
        RAISE EXCEPTION 'link assertion: confidence must be omitted when absent, never null (principle 4)';
    END IF;
END;
$$;

-- 3. Extend the per-type twin hook. Identity link/unlink: run the floor + HARD-require
--    an authored twin (like demographics). This CREATE OR REPLACE PRESERVES db/010's
--    demographic branches and db/015's honest-degrade fallback for every other type —
--    it only adds the identity branch (submit_event itself is NEVER re-declared).
CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
    v_twin        text    := b ->> 'plaintext_twin';
    v_authored    boolean := v_twin IS NOT NULL AND length(regexp_replace(v_twin, '\s+', '', 'g')) > 0;
    v_demographic boolean := false;
    v_identity    boolean := false;
BEGIN
    -- Per-type structural floor.
    IF p_type = 'demographic.identifier.asserted' THEN
        PERFORM cairn_check_identifier_assertion(b);
        v_demographic := true;
    ELSIF p_type = 'demographic.field.asserted' THEN
        PERFORM cairn_check_demographic_field(b);
        v_demographic := true;
    ELSIF p_type IN ('identity.link.asserted', 'identity.unlink.asserted') THEN
        PERFORM cairn_check_link_assertion(b);
        v_identity := true;
    END IF;

    -- Authored twin present → carry it verbatim (principle 11; the conformant path).
    IF v_authored THEN
        RETURN v_twin;
    END IF;

    -- Absent/blank twin: demographic AND identity types HARD-require it; every other
    -- type degrades honestly to a flagged derived skeleton (ADR-0039).
    IF v_demographic THEN
        RAISE EXCEPTION 'submit_event: demographic assertion requires a non-empty authored twin (§4.5)';
    ELSIF v_identity THEN
        RAISE EXCEPTION 'submit_event: identity linkage assertion requires a non-empty authored twin (§5.7)';
    END IF;
    RETURN cairn_twin_skeleton(p_type, b);
END;
$$;

-- 4. patient_link: the standing-edge overlay (same shape as patient_identifier). One
--    row per canonical (low, high) pair; the latest-HLC link/unlink assertion wins the
--    `state`. Never merge, always overlay — link then a later unlink ⇒ edge gone.
CREATE TABLE IF NOT EXISTS patient_link (
    low         UUID    NOT NULL,
    high        UUID    NOT NULL,
    state       TEXT    NOT NULL CHECK (state IN ('link', 'unlink')),
    hlc_wall    BIGINT  NOT NULL,
    hlc_counter INTEGER NOT NULL,
    origin      TEXT    NOT NULL,
    provenance  TEXT    NOT NULL,
    confidence  TEXT,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (low, high),
    CHECK (low < high)
);
GRANT SELECT ON patient_link TO cairn_agent;

-- 5. person_member: the golden-identity projection. person_id = the MINIMUM UUID in
--    the connected component (a derived canonical representative — the "person" is a
--    projection, never a stored immortal id; principle 2). A UUID that once had an edge
--    and is now isolated gets a row mapping to itself; a UUID never touched by any
--    linkage event has no row at all (the person_chart VIEW coalesces to self).
CREATE TABLE IF NOT EXISTS person_member (
    patient_id UUID PRIMARY KEY,
    person_id  UUID NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON person_member TO cairn_agent;

-- Configurable oversize guard. A component larger than this is a matcher pathology
-- (mass false-merge); we REFUSE the offending event rather than silently corrupt
-- membership (never a silent cap — the db/017b oversized-block discipline). Reads a
-- session GUC so it is operationally tunable and testable; default 10000.
CREATE OR REPLACE FUNCTION cairn_max_component_size()
RETURNS integer LANGUAGE sql STABLE AS $$
    SELECT COALESCE(NULLIF(current_setting('cairn.max_component_size', true), '')::integer, 10000);
$$;

-- Recompute the connected component around one seed UUID over the STANDING link edges
-- (state='link'), and rewrite person_member for every member to point at the min-UUID
-- representative. Cost is bounded by the touched component's size, not the table's —
-- keeping chart reads O(1) (the ADR-0001/Bet-B incremental-projection discipline).
CREATE OR REPLACE FUNCTION cairn_recompute_component(p_seed uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_members uuid[];
    v_person  uuid;
BEGIN
    -- Bounded BFS: walk standing link edges outward from the seed (undirected — an
    -- edge stored as (low, high) is traversable from either endpoint).
    WITH RECURSIVE comp(node) AS (
        SELECT p_seed
        UNION
        SELECT CASE WHEN pl.low = comp.node THEN pl.high ELSE pl.low END
        FROM comp
        JOIN patient_link pl
          ON pl.state = 'link' AND (pl.low = comp.node OR pl.high = comp.node)
    )
    SELECT array_agg(node) INTO v_members FROM comp;

    -- Fail loud on a pathological component (mass false-merge) — never silently cap.
    IF array_length(v_members, 1) > cairn_max_component_size() THEN
        RAISE EXCEPTION
            'identity linkage: component around % exceeds max size % — refusing to project (matcher pathology)',
            p_seed, cairn_max_component_size();
    END IF;

    -- The canonical representative is the minimum UUID in the component. Postgres has
    -- no min()/max() aggregate for the uuid type, so order by the uuid `<` operator
    -- (which uuid does provide) and take the first — semantically identical to min().
    v_person := (SELECT m FROM unnest(v_members) AS m ORDER BY m LIMIT 1);

    INSERT INTO person_member (patient_id, person_id, updated_at)
    SELECT m, v_person, clock_timestamp() FROM unnest(v_members) AS m
    ON CONFLICT (patient_id) DO UPDATE SET
        person_id  = EXCLUDED.person_id,
        updated_at = clock_timestamp();
END;
$$;

-- Incremental maintenance: fold exactly the one new link/unlink event into the edge
-- overlay. The whole row overlays atomically only when the incoming HLC is strictly
-- greater than the stored one (ON CONFLICT ... WHERE) — so out-of-order arrival
-- converges to the highest-HLC assertion. After the edge overlay, recompute the
-- connected-component projection around both endpoints (see cairn_recompute_component
-- above).
CREATE OR REPLACE FUNCTION patient_link_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p       jsonb := NEW.body;
    a       uuid  := (p ->> 'subject_a')::uuid;
    b       uuid  := (p ->> 'subject_b')::uuid;
    lo      uuid  := LEAST(a, b);
    hi      uuid  := GREATEST(a, b);
    v_state text  := CASE WHEN NEW.event_type = 'identity.link.asserted' THEN 'link' ELSE 'unlink' END;
BEGIN
    INSERT INTO patient_link
        (low, high, state, hlc_wall, hlc_counter, origin, provenance, confidence)
    VALUES
        (lo, hi, v_state, NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
         p ->> 'provenance', p ->> 'confidence')
    ON CONFLICT (low, high) DO UPDATE SET
        state       = EXCLUDED.state,
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        provenance  = EXCLUDED.provenance,
        confidence  = EXCLUDED.confidence,
        updated_at  = clock_timestamp()
    WHERE (EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin)
        > (patient_link.hlc_wall, patient_link.hlc_counter, patient_link.origin);

    -- Recompute the touched component(s). Recomputing BOTH endpoints is always
    -- correct: a link merges (both endpoints reach the same union); an unlink splits
    -- into at most the piece containing `lo` and the piece containing `hi`, and every
    -- previously-connected node is reachable from one of them.
    PERFORM cairn_recompute_component(lo);
    PERFORM cairn_recompute_component(hi);
    RETURN NULL;  -- AFTER trigger
END;
$$;

DROP TRIGGER IF EXISTS patient_link_apply_trg ON event_log;
CREATE TRIGGER patient_link_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type IN ('identity.link.asserted', 'identity.unlink.asserted'))
    EXECUTE FUNCTION patient_link_apply();

-- 6. Demonstrated unified-read VIEW (§5.1 "the unified chart unions the event streams
--    of all member UUIDs"). Thin by design: every patient_chart row is tagged with its
--    person_id — its component representative, or its own patient_id when unknown to the
--    link graph. Selecting WHERE person_id = X returns all member charts. The REAL
--    unified-chart read surface (ordering, dedup, trust states) is the API/UI tier,
--    above the foundation line — deliberately out of scope for C1.
CREATE OR REPLACE VIEW person_chart AS
    SELECT COALESCE(pm.person_id, pc.patient_id) AS person_id, pc.*
    FROM patient_chart pc
    LEFT JOIN person_member pm ON pm.patient_id = pc.patient_id;

GRANT SELECT ON person_chart TO cairn_agent;

COMMIT;
