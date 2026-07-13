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
-- Signature unified to (p_type text, b jsonb) for the #173 registry dispatch; p_type is
-- unused here (this check validates the body). DROP clears any stale (jsonb) overload on
-- an upgraded-in-place dev DB.
DROP FUNCTION IF EXISTS cairn_check_link_assertion(jsonb);
CREATE OR REPLACE FUNCTION cairn_check_link_assertion(p_type text, b jsonb)
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

-- 3. Register both link verbs' structural floor + hard twin requirement in the #173
--    registry (replaces the copied cairn_event_twin dispatch chain; the single db/005
--    dispatcher reads these rows). Placed after the floor fn above so the fail-closed
--    registry trigger (db/005) sees cairn_check_link_assertion(text, jsonb) already
--    declared at load time.
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('identity.link.asserted',   'cairn_check_link_assertion', 'identity linkage assertion requires a non-empty authored twin (§5.7)'),
    ('identity.unlink.asserted', 'cairn_check_link_assertion', 'identity linkage assertion requires a non-empty authored twin (§5.7)')
ON CONFLICT (event_type) DO NOTHING;

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
    content_address BYTEA NOT NULL,   -- winning event's content address; the #115 tiebreak
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (low, high),
    CHECK (low < high)
);
GRANT SELECT ON patient_link TO cairn_agent;
-- The component BFS joins on (pl.low = node OR pl.high = node). The PK indexes the `low`
-- side; without an index on `high` the walk sequentially scans the edge table, which
-- cliffs as the link graph grows. Index the high side so both directions are indexed.
CREATE INDEX IF NOT EXISTS patient_link_high_idx ON patient_link (high) WHERE state = 'link';

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

-- Worklist of projection recomputes SKIPPED on the sync apply path (issue #91/A5b).
-- The cap above is a NODE-LOCAL GUC; RAISE-ing on it while applying a validly-signed
-- replicated event that peers already accepted would let a config difference fork the
-- event set between honest nodes. So at apply time the recompute clamps-and-flags:
-- the event lands, person_member is left stale-but-honest, and a row here is the loud
-- alarm + repair worklist. Append-only by usage (rows are evidence, never edited).
CREATE TABLE IF NOT EXISTS identity_projection_flag (
    flag_id       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    seed          UUID        NOT NULL,   -- the component endpoint whose recompute was skipped
    observed_size INTEGER     NOT NULL,   -- how big the BFS said the component is
    cap           INTEGER     NOT NULL,   -- cairn_max_component_size() at the time
    flagged_at    TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON identity_projection_flag TO cairn_agent;

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

    -- Pathological component (mass false-merge) — never a SILENT cap, but the loud
    -- response differs by door (issue #91/A5b):
    --   * local authoring (submit_event): FAIL LOUD — refuse the event. Nothing has
    --     accepted it yet, so refusal loses no data and stops the pathology earliest.
    --   * sync apply (apply_remote_event sets the transaction-local marker below):
    --     CLAMP AND FLAG — peers already hold this validly-signed event, and this cap
    --     is a node-local GUC, so vetoing here would fork the event set between honest
    --     nodes on a config difference. The event applies; the recompute is skipped
    --     (person_member stays stale-but-honest); the flag row is the alarm/worklist.
    IF array_length(v_members, 1) > cairn_max_component_size() THEN
        IF current_setting('cairn.remote_apply', true) = 'on' THEN
            INSERT INTO identity_projection_flag (seed, observed_size, cap)
            VALUES (p_seed, array_length(v_members, 1), cairn_max_component_size());
            RETURN;
        END IF;
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
    v_cur   record;
BEGIN
    -- Serialize linkage applies (RACE FIX). cairn_recompute_component is a read-modify-
    -- write of person_member over the STANDING edges; under READ COMMITTED two concurrent
    -- applies (e.g. link(A,B) and link(B,C)) each BFS without seeing the other's uncommitted
    -- edge, so the union {A,B,C} is left half-computed until something else touches it.
    -- A single transaction-scoped advisory lock serializes all linkage recomputes. It is a
    -- coarse lock, but link/unlink is rare relative to clinical writes and the clinical hot
    -- path never takes it, so there is no contention with normal event submission. (Keyed on
    -- a fixed project constant distinct from the test-serialization guard key.)
    PERFORM pg_advisory_xact_lock(x'4341524E4C4B'::bigint);  -- 'CARNLK'

    -- #157: detect a Byzantine HLC-triple collision against the current standing link and record
    -- an advisory signal before overlaying. lo/hi are the canonical pair already computed above.
    -- NOTE: the pg_advisory_xact_lock above is INCIDENTAL to this detection — it exists only for
    -- the component-recompute race fix, but happens to also serialize this overlay's SELECT-then-
    -- upsert, so patient_link cannot suffer the concurrent-apply miss described in db/029. The
    -- other four overlays (db/002/023/024/025) hold no such lock and rely on the sequential-apply
    -- assumption documented there.
    SELECT hlc_wall, hlc_counter, origin, content_address
      INTO v_cur
      FROM patient_link WHERE low = lo AND high = hi;
    IF FOUND AND cairn_hlc_triple_collision(
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
            v_cur.hlc_wall, v_cur.hlc_counter, v_cur.origin, v_cur.content_address) THEN
        PERFORM cairn_record_hlc_collision(
            'patient_link', lo::text || '|' || hi::text,
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
            NEW.content_address, v_cur.content_address);
    END IF;

    INSERT INTO patient_link
        (low, high, state, hlc_wall, hlc_counter, origin, provenance, confidence,
         content_address)
    VALUES
        (lo, hi, v_state, NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
         p ->> 'provenance', p ->> 'confidence', NEW.content_address)
    ON CONFLICT (low, high) DO UPDATE SET
        state       = EXCLUDED.state,
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        provenance  = EXCLUDED.provenance,
        confidence  = EXCLUDED.confidence,
        content_address = EXCLUDED.content_address,
        updated_at  = clock_timestamp()
    -- Overlay only when the incoming event outranks the stored winner, with content_address
    -- as the deterministic final tiebreaker (#115) so an HLC-triple collision converges.
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        patient_link.hlc_wall, patient_link.hlc_counter, patient_link.origin,
        patient_link.content_address);

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
