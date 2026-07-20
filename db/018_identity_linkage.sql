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
-- Additive widening (#115 → issue #207): the CREATE above no-ops on a pre-widening DB, so
-- the column must ALSO ship as an idempotent ALTER or the overlay INSERT fails at trigger
-- depth (write outage). Nullable here — pre-#115 rows carry no recorded winner address and
-- degrade honestly to the pre-#115 first-applied tiebreak; every new upsert writes it.
-- Fresh DBs get NOT NULL from the CREATE. Guarded by migration_replay_widening.rs.
ALTER TABLE patient_link ADD COLUMN IF NOT EXISTS content_address BYTEA;
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
-- Natural-key uniqueness so a heal-mode replay of the IDENTICAL skip (same seed,
-- same observed size, same cap) converges instead of appending a duplicate alarm
-- row every reproject run; a genuinely NEW observation (the component grew
-- further, or the cap changed) still gets its own row (ADR-0057 replay-idempotency
-- — see the ON CONFLICT on the INSERT below).
CREATE UNIQUE INDEX IF NOT EXISTS identity_projection_flag_natural_idx
    ON identity_projection_flag (seed, observed_size, cap);

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
            -- ADR-0057 replay-idempotency: a heal-mode reproject re-running this SAME
            -- skipped recompute must not append a duplicate worklist alarm.
            INSERT INTO identity_projection_flag (seed, observed_size, cap)
            VALUES (p_seed, array_length(v_members, 1), cairn_max_component_size())
            ON CONFLICT (seed, observed_size, cap) DO NOTHING;
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
-- #190 (finding A2): the standing worklist of UN-ATTESTED links that tripped the
-- db/016 hard veto at THIS node's door on the sync-apply path. The event itself is
-- admitted and projected (a node-local veto verdict reads node-local demographic
-- state — two honest nodes can disagree, so refusing a replicated link would fork
-- the event set); this table is what keeps the merge from being SILENT: both
-- subjects read 'under-review' in chart_trust (db/024) until a human resolves the
-- pair — an unlink, or a human-attested re-link, clears the row. Node-local derived
-- state, never on the wire.
CREATE TABLE IF NOT EXISTS link_veto_flag (
    low             UUID  NOT NULL,
    high            UUID  NOT NULL,
    content_address BYTEA NOT NULL,   -- the vetoed link event
    flagged_at      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (low, high)
);
GRANT SELECT ON link_veto_flag TO cairn_agent;

-- The per-type trigger is superseded by cairn_projection_dispatch_trg (db/005,
-- ADR-0057); this fn is now registered in cairn_projection_apply below.
DROP TRIGGER IF EXISTS patient_link_apply_trg ON event_log;
-- The old zero-arg trigger-function signature is superseded by the (event_log)
-- apply-fn signature below; CREATE OR REPLACE cannot change a function's arg
-- list (it would overload, not replace), so drop the old signature explicitly
-- (same idiom as db/005's `DROP FUNCTION IF EXISTS submit_event(bytea, bytea, bytea);`).
DROP FUNCTION IF EXISTS patient_link_apply();

CREATE OR REPLACE FUNCTION patient_link_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p       jsonb := e.body;
    -- a/b/lo/hi are NOT initialized in DECLARE: DECLARE initializers run at block entry,
    -- BEFORE the seal guard below can return, so a `(p ->> 'subject_a')::uuid` cast here
    -- would raise `invalid input syntax for type uuid` on a wrongly-sealed row whose ciphertext
    -- container carries a non-UUID top-level subject_a — aborting apply on a VERIFIABLE event
    -- and wedging the sync watermark (code-review finding #1). The casts are deferred to AFTER
    -- the guard. (This projection is the one non-clinical trigger that casts a body field to a
    -- strict type; its siblings do only NULL-safe text ops in DECLARE, so the guard alone
    -- protected them — here the cast must sit below the guard, not in DECLARE.)
    a       uuid;
    b       uuid;
    lo      uuid;
    hi      uuid;
    v_state text  := CASE WHEN e.event_type = 'identity.link.asserted' THEN 'link' ELSE 'unlink' END;
    v_cur   record;
    -- The STANDING overlay winner for (lo, hi), read back AFTER the upsert below so the
    -- #190 flag lifecycle keys on what actually won, not on the arriving event (finding 1).
    v_win_state    text;
    v_win_ca       bytea;
    v_win_attested boolean;
BEGIN
    -- ADR-0052 §2 seal-robustness (#10): a wrongly-sealed NON-clinical row holds CIPHERTEXT
    -- in e.body (refused at submit; admitted lenient at apply for lossless sync). Reading it
    -- below would drive NULLs into this projection and freeze the sync watermark — so a sealed
    -- row projects NOTHING (harmless ciphertext noise; no custody, no leak). This guard MUST
    -- precede the subject_a/subject_b casts below (they are deliberately not in DECLARE, see there).
    IF e.sealed THEN RETURN; END IF;
    -- Canonical (lo, hi) pair — cast here, AFTER the seal guard, so a sealed row never reaches
    -- these strict UUID casts (the floor already validated subject_a/subject_b for an UNSEALED
    -- link via cairn_check_link_assertion at both doors).
    a  := (p ->> 'subject_a')::uuid;
    b  := (p ->> 'subject_b')::uuid;
    lo := LEAST(a, b);
    hi := GREATEST(a, b);
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
            e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address,
            v_cur.hlc_wall, v_cur.hlc_counter, v_cur.origin, v_cur.content_address) THEN
        PERFORM cairn_record_hlc_collision(
            'patient_link', lo::text || '|' || hi::text,
            e.hlc_wall, e.hlc_counter, e.node_origin,
            e.content_address, v_cur.content_address);
    END IF;

    -- #190 hard-veto floor AT THE DOOR (finding A2; principle 12 — the veto was
    -- previously enforced only in the Rust auto-apply seam, so an enrolled agent with
    -- raw SQL could merge two hard-vetoed charts through submit_event unflagged).
    -- Contract: the veto FORCES A HUMAN DECISION, never auto-rejects one — so a
    -- human-attested link (e.attester_key set: the stored, verified vouch) passes;
    -- an UN-ATTESTED link that trips the veto is refused on the LOCAL door only —
    -- nothing has been accepted yet, so fail loud at source. On the sync-apply path the
    -- event is admitted (the verdict reads node-local demographic state, so a remote
    -- refusal would fork honest nodes) and the merge is caught by the flag lifecycle
    -- AFTER the overlay below. The Rust auto-apply pre-check stays as the polite early
    -- skip; this is the floor behind it.
    IF v_state = 'link' AND e.attester_key IS NULL AND cairn_has_hard_veto(lo, hi)
       AND current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on' THEN
        RAISE EXCEPTION
            'identity link %/%: hard veto — the §5.2 floor forces a human decision; an un-attested (agent) link may not merge these charts (issue #190)',
            lo, hi;
    END IF;

    INSERT INTO patient_link
        (low, high, state, hlc_wall, hlc_counter, origin, provenance, confidence,
         content_address)
    VALUES
        (lo, hi, v_state, e.hlc_wall, e.hlc_counter, e.node_origin,
         p ->> 'provenance', p ->> 'confidence', e.content_address)
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

    -- #190 flag lifecycle, DERIVED FROM THE STANDING OVERLAY WINNER (PR #219 review,
    -- finding 1) — never from the arriving event's verb. The upsert above is HLC-guarded,
    -- so an arriving event can LOSE the overlay; keying the flag on arrival desynced it
    -- from the standing edge in two exploitable ways: a BACKDATED un-attested unlink that
    -- loses the overlay would clear the flag while the vetoed merge still stands (a silent
    -- merge the ADR-0030 writer triggers with one cheap event — unlinks are never veto-
    -- gated), and a STALE vetoed link losing to a standing unlink would raise a phantom
    -- flag (arrival-order-dependent trust state across honest nodes — the class of bug
    -- #194 closes for the demographic projections). Read back who actually won and flag
    -- iff the standing winner is an UN-ATTESTED link that still trips the veto; otherwise
    -- clear. The winner's attestation is looked up via its content_address (UNIQUE in
    -- event_log); patient_link always has a row here (the upsert inserted or kept one).
    -- Node-local advisory state, so INSERT/DELETE is honest — the events all remain logged.
    SELECT pl.state, pl.content_address, el.attester_key IS NOT NULL
      INTO v_win_state, v_win_ca, v_win_attested
      FROM patient_link pl
      JOIN event_log el ON el.content_address = pl.content_address
      WHERE pl.low = lo AND pl.high = hi;

    IF v_win_state = 'link' AND NOT v_win_attested AND cairn_has_hard_veto(lo, hi) THEN
        INSERT INTO link_veto_flag (low, high, content_address)
        VALUES (lo, hi, v_win_ca)
        ON CONFLICT (low, high) DO UPDATE SET content_address = EXCLUDED.content_address;
    ELSE
        DELETE FROM link_veto_flag WHERE low = lo AND high = hi;
    END IF;

    -- Recompute the touched component(s). Recomputing BOTH endpoints is always
    -- correct: a link merges (both endpoints reach the same union); an unlink splits
    -- into at most the piece containing `lo` and the piece containing `hi`, and every
    -- previously-connected node is reachable from one of them.
    PERFORM cairn_recompute_component(lo);
    PERFORM cairn_recompute_component(hi);
    RETURN;
END;
$$;

-- A trigger fn could never be called directly; a plain fn gets PUBLIC EXECUTE by
-- default. Same discipline as every privileged fn in db/005 (Task-1 review finding).
REVOKE EXECUTE ON FUNCTION patient_link_apply(event_log) FROM PUBLIC;

-- 6. Demonstrated unified-read VIEW (§5.1 "the unified chart unions the event streams
--    of all member UUIDs"). Thin by design: every patient_chart row is tagged with its
--    person_id — its component representative, or its own patient_id when unknown to the
--    link graph. Selecting WHERE person_id = X returns all member charts. The REAL
--    unified-chart read surface (ordering, dedup, trust states) is the API/UI tier,
--    above the foundation line — deliberately out of scope for C1.
-- Upgrade heal (issue #207): on a database created before the #115 widening, this view
-- exists WITHOUT demo_content_address (pc.* expanded at CREATE time, against the then-
-- narrow patient_chart). db/002's additive ALTER has just re-added the column mid-table,
-- so the CREATE OR REPLACE below would try to splice a new column into the MIDDLE of the
-- existing view's column list — which Postgres refuses (a replaced view may only append
-- columns at the end), aborting node boot. Drop the stale-shaped view ONCE; CASCADE takes
-- the person_chart_trust composition (db/023) with it, and both are recreated in this same
-- schema load. Steady-state loads (shape already current) never drop, so any out-of-schema
-- dependent views are not disturbed.
DO $$
BEGIN
    IF to_regclass('public.person_chart') IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM information_schema.columns
           WHERE table_schema = 'public' AND table_name = 'person_chart'
             AND column_name = 'demo_content_address') THEN
        DROP VIEW person_chart CASCADE;
    END IF;
END $$;

CREATE OR REPLACE VIEW person_chart AS
    SELECT COALESCE(pm.person_id, pc.patient_id) AS person_id, pc.*
    FROM patient_chart pc
    LEFT JOIN person_member pm ON pm.patient_id = pc.patient_id;

GRANT SELECT ON person_chart TO cairn_agent;

-- Registered apply fn for the #208/ADR-0057 generic dispatcher (db/005) + cairn_reproject
-- heal/rebuild (db/039). Both verbs share the ONE fn and the SAME projection_tables list
-- (link and unlink fold into the same overlay + component recompute + worklist tables).
-- #214 + steady-state discipline: converge these rows to the migration text on every
-- connect, but stay write-free once already converged (no dead tuples, no validate-
-- trigger fire).
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('identity.link.asserted',   'patient_link_apply',
     ARRAY['patient_link', 'person_member', 'link_veto_flag', 'identity_projection_flag'], 10, TRUE),
    ('identity.unlink.asserted', 'patient_link_apply',
     ARRAY['patient_link', 'person_member', 'link_veto_flag', 'identity_projection_flag'], 10, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
