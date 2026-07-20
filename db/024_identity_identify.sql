-- db/024_identity_identify.sql
-- Cairn — §5.4/§5.7 identity-pending + `identify` + the *unconfirmed* trust state
-- (identity piece C4).
--
-- WHAT: the third and final state of the §5.7 chart trust-state contract. C3 (db/023)
-- built the projection with *confirmed* (default) and *under-review* (open dispute) and
-- reserved a compose-slot for *unconfirmed* — an identity-pending "John Doe" registration
-- (§5.4). This slice supplies that state and the `identify` event (§5.7 "who, method")
-- that clears it back to *confirmed*.
--
-- Two additive event types flow through the reused submit_event door (db/005): they
-- register in event_type_class and add a branch to the cairn_event_twin hook, exactly
-- as demographics (db/010), C1 link (db/018), and C3 dispute (db/023) did. An
-- identity-pending marker / identify is ADDITIVE (it annotates the trust state; it never
-- erases, moves, or blocks anything), so no attestation is forced unless a responsibility-
-- bearing contributor is named — the same low-ceremony path as the C1 link and C3 dispute.
--
-- Additive DDL only; no submit_event re-declaration, no floor change, no SCHEMA-version
-- bump. db/023 is left UNTOUCHED — this migration CREATE-OR-REPLACEs the shared
-- cairn_event_twin hook and the chart_trust view from a LATER migration step (the
-- established slice pattern), never editing the earlier file.

BEGIN;

-- 1. Register the two additive identity-state event types (fail-closed registry, ADR-0010).
--    additive + targets_other_author=FALSE: neither a pending marker nor an identify
--    suppresses or targets another author's event, so the db/005 gate requires NO
--    attestation for a clerk / registration-desk actor. §5.7's "Human; method recorded"
--    composes two ways: `method` is structurally required (the floor below), and the
--    "human vouches" requirement rides the existing attestation gate whenever a
--    responsibility-bearing contributor is named — workflow-tier policy, not a floor rule.
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('identity.pending.asserted',  'additive', FALSE),
    ('identity.identify.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. The §5.7 structural floor. Culture-neutral: validates STRUCTURE only — a valid
--    subject uuid and a required non-empty descriptive field whose key differs by type
--    (`basis` opens the unconfirmed state, `method` records the identification). Each
--    violation is a distinct legible exception (the cairn_check_dispute_assertion pattern).
CREATE OR REPLACE FUNCTION cairn_check_identity_state_assertion(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p         jsonb := b -> 'payload';
    v_field   text;   -- the descriptive-field key required for THIS type
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'identity-state assertion: missing payload';
    END IF;
    -- subject: present, string, valid uuid (the patient chart whose identity state this
    -- asserts). No cross-existence check — a pending marker or identify may arrive before
    -- or independently of the chart it names (offline-first, set-union; the safety signal
    -- must exist without the body, mirroring §5.9 and the C3 dispute floor).
    IF jsonb_typeof(p -> 'subject') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'identity-state assertion: subject must be a uuid string (§5.7)';
    END IF;
    BEGIN
        PERFORM (p ->> 'subject')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'identity-state assertion: subject must be a valid uuid (§5.7)';
    END;
    -- The descriptive field: `basis` opens the unconfirmed state (§5.4), `method` records
    -- the identification (§5.7 "method recorded"). Present + non-empty (§4.1 ladder;
    -- value-open — "unknown" is honest, "" is fabrication-only; principle 4).
    v_field := CASE WHEN p_type = 'identity.pending.asserted' THEN 'basis' ELSE 'method' END;
    IF jsonb_typeof(p -> v_field) IS DISTINCT FROM 'string'
       OR length(trim(p ->> v_field)) = 0 THEN
        RAISE EXCEPTION 'identity-state assertion: % must be a non-empty string (§4.1)', v_field;
    END IF;
END;
$$;

-- 3. Register both identity-state verbs' structural floor + hard twin requirement in the
--    #173 registry (replaces the copied cairn_event_twin dispatch chain; the single db/005
--    dispatcher reads these rows). Placed after the floor fn above so the fail-closed
--    registry trigger (db/005) sees cairn_check_identity_state_assertion(text, jsonb)
--    declared at load time.
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('identity.pending.asserted',  'cairn_check_identity_state_assertion', 'identity-state assertion requires a non-empty authored twin (§5.7)'),
    ('identity.identify.asserted', 'cairn_check_identity_state_assertion', 'identity-state assertion requires a non-empty authored twin (§5.7)')
ON CONFLICT (event_type) DO NOTHING;

-- 4. chart_identity_state: the standing identity-status overlay (same overlay discipline
--    as db/023's chart_dispute, but keyed by the SUBJECT itself — a chart carries exactly
--    ONE identity status at a time, so the subject is the natural key). One row per
--    subject; the latest-HLC assertion wins the `state`. No connected-component recompute
--    (single-row standing fact, cheaper than C1) and — because the key IS the subject —
--    NO subject-consistency guard is possible or needed (contrast db/023's chart_dispute,
--    whose separate dispute_id made a rebind hazard the guard closes).
CREATE TABLE IF NOT EXISTS chart_identity_state (
    subject     UUID    PRIMARY KEY,
    state       TEXT    NOT NULL CHECK (state IN ('pending', 'identified')),
    detail      TEXT,                       -- the winning assertion's text: its `basis` (pending) or `method` (identified)
    hlc_wall    BIGINT  NOT NULL,
    hlc_counter INTEGER NOT NULL,
    origin      TEXT    NOT NULL,
    content_address BYTEA NOT NULL,   -- winning event's content address; the #115 tiebreak
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
-- Additive widening (#115 → issue #207): see db/018's patient_link note — the CREATE
-- no-ops on a pre-widening DB; nullable ALTER, new upserts always write it.
-- Guarded by migration_replay_widening.rs.
ALTER TABLE chart_identity_state ADD COLUMN IF NOT EXISTS content_address BYTEA;
GRANT SELECT ON chart_identity_state TO cairn_agent;
-- The chart_trust VIEW's unconfirmed source AND the "still-John-Doe" worklist are both
-- "standing PENDING charts"; index exactly that partial set so neither cliffs as the
-- identity-state history grows.
CREATE INDEX IF NOT EXISTS chart_identity_state_pending_idx
    ON chart_identity_state (subject) WHERE state = 'pending';

-- Incremental maintenance: fold exactly the one new identity-state event into the overlay.
-- The whole row overlays atomically only when the incoming HLC is strictly greater than
-- the stored one (ON CONFLICT ... WHERE) — so out-of-order arrival (an identify landing
-- before the pending it clears, or a re-registration pending landing after an identify)
-- converges to the highest-HLC assertion, arrival-order-independent.
-- The per-type trigger is superseded by cairn_projection_dispatch_trg (db/005,
-- ADR-0057); this fn is now registered in cairn_projection_apply below.
DROP TRIGGER IF EXISTS chart_identity_state_apply_trg ON event_log;
-- The old zero-arg trigger-function signature is superseded by the (event_log)
-- apply-fn signature below; CREATE OR REPLACE cannot change a function's arg
-- list (it would overload, not replace), so drop the old signature explicitly
-- (same idiom as db/005's `DROP FUNCTION IF EXISTS submit_event(bytea, bytea, bytea);`).
DROP FUNCTION IF EXISTS chart_identity_state_apply();

CREATE OR REPLACE FUNCTION chart_identity_state_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p        jsonb := e.body;
    v_state  text  := CASE WHEN e.event_type = 'identity.identify.asserted'
                           THEN 'identified' ELSE 'pending' END;
    -- The descriptive field's key differs by type (basis opens, method identifies); store
    -- whichever one this assertion carries in the neutrally-named `detail` column so the
    -- overlay row is self-describing without a misleading column name.
    v_detail text  := CASE WHEN e.event_type = 'identity.identify.asserted'
                           THEN p ->> 'method' ELSE p ->> 'basis' END;
    v_cur    record;
BEGIN
    -- ADR-0052 §2 seal-robustness (#10): a wrongly-sealed NON-clinical row holds CIPHERTEXT
    -- in e.body (refused at submit; admitted lenient at apply for lossless sync). Reading it
    -- below would drive NULLs into this projection and freeze the sync watermark — so a sealed
    -- row projects NOTHING (harmless ciphertext noise; no custody, no leak).
    IF e.sealed THEN RETURN; END IF;
    -- #157: detect a Byzantine HLC-triple collision against the current identity state and record
    -- an advisory signal before overlaying pending-vs-identified.
    SELECT hlc_wall, hlc_counter, origin, content_address
      INTO v_cur
      FROM chart_identity_state WHERE subject = (p ->> 'subject')::uuid;
    IF FOUND AND cairn_hlc_triple_collision(
            e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address,
            v_cur.hlc_wall, v_cur.hlc_counter, v_cur.origin, v_cur.content_address) THEN
        PERFORM cairn_record_hlc_collision(
            'chart_identity_state', p ->> 'subject',
            e.hlc_wall, e.hlc_counter, e.node_origin,
            e.content_address, v_cur.content_address);
    END IF;

    INSERT INTO chart_identity_state
        (subject, state, detail, hlc_wall, hlc_counter, origin, content_address)
    VALUES
        ((p ->> 'subject')::uuid, v_state, v_detail,
         e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    ON CONFLICT (subject) DO UPDATE SET
        state       = EXCLUDED.state,
        detail      = EXCLUDED.detail,
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        content_address = EXCLUDED.content_address,
        updated_at  = clock_timestamp()
    -- content_address is the deterministic final tiebreaker (#115) so an HLC-triple collision
    -- converges pending-vs-identified identically on every node.
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        chart_identity_state.hlc_wall, chart_identity_state.hlc_counter,
        chart_identity_state.origin, chart_identity_state.content_address);
    RETURN;
END;
$$;

-- A trigger fn could never be called directly; a plain fn gets PUBLIC EXECUTE by
-- default. Same discipline as every privileged fn in db/005 (Task-1 review finding).
REVOKE EXECUTE ON FUNCTION chart_identity_state_apply(event_log) FROM PUBLIC;

-- 5. chart_trust: rework the §5.7 effective trust-state projection to COMPOSE two sources
--    by highest severity. C3 delivered under-review from one source (open dispute); C4
--    adds unconfirmed (a standing identity-pending chart) and turns the VIEW into a
--    highest-severity-wins overlay — the same "effective grade is the highest standing
--    assertion" discipline as the §5.9 sensitivity projection.
--
--    Precedence: under-review (2) > unconfirmed (1) > confirmed (default). When a chart is
--    BOTH identity-pending AND has an open dispute, the single displayed state is
--    under-review: *unconfirmed* means "we don't yet know WHO this is" (the data present
--    genuinely belongs to this unnamed person — the caution is about ABSENT history),
--    whereas *under-review* means "the ATTRIBUTION of events on this chart is actively
--    challenged" (the data PRESENT may not belong here at all). The sharper, more actively-
--    dangerous caution wins the display.
--
--    Built so LATER slices ADD a branch, never rewrite: a future *under-review* source
--    (pending reattribution §5.5, a coherence-check demoted link §5.2) emits severity 2 and
--    needs NO CASE change; a source introducing a NEW label adds both its `SELECT ... n`
--    branch AND its `WHEN n` arm (the one invariant a future editor must hold — every
--    emitted severity must have a matching WHEN, or trust_state would be NULL).
--
--    The column contract is UNCHANGED (patient_id uuid, trust_state text), so this
--    CREATE OR REPLACE is reload-idempotent across connect_and_load_schema and db/023's
--    person_chart_trust (which LEFT JOINs this view) is untouched. A subject in the DEFAULT
--    (confirmed) state appears in NEITHER source ⇒ has no row here ⇒ the read coalesces to
--    'confirmed', keeping the VIEW tiny.
CREATE OR REPLACE VIEW chart_trust AS
    WITH trust_source(patient_id, severity) AS (
        -- under-review (2): any standing OPEN dispute                 (C3, §5.5(b))
        SELECT subject, 2 FROM chart_dispute        WHERE state = 'open'
        UNION ALL
        -- unconfirmed  (1): a standing identity-pending chart         (C4, §5.4)  <-- THIS slice
        SELECT subject, 1 FROM chart_identity_state WHERE state = 'pending'
        UNION ALL
        -- under-review (2): an un-attested link admitted on sync that trips the local
        -- db/016 hard veto (#190) — the merge converges, but BOTH charts read
        -- under-review until a human resolves the pair (unlink / attested re-link)
        SELECT low,  2 FROM link_veto_flag
        UNION ALL
        SELECT high, 2 FROM link_veto_flag
        -- future sources ADD a branch here (both a SELECT and, for a new label, a WHEN):
        --   under-review (2) <- pending reattribution                 (§5.5 — future)
        --   under-review (2) <- coherence-check demoted link          (§5.2 feedback — future)
    )
    SELECT patient_id,
           (CASE max(severity) WHEN 2 THEN 'under-review'
                               WHEN 1 THEN 'unconfirmed'
                               -- FAIL-SAFE, not dead code: today severity is only ever 1 or 2,
                               -- so this ELSE is unreachable. But if a future editor adds a
                               -- `SELECT ... n` source branch and forgets its matching `WHEN n`,
                               -- an un-elsed CASE would yield NULL — and person_chart_trust's
                               -- COALESCE(trust_state,'confirmed') would then render a genuinely
                               -- trust-flagged chart as *confirmed*: a silent fail-OPEN on a
                               -- safety signal. Degrade to the most cautious state instead, so a
                               -- missing WHEN can only ever OVER-warn, never under-warn.
                               ELSE 'under-review' END)::text AS trust_state
    FROM trust_source
    GROUP BY patient_id;

GRANT SELECT ON chart_trust TO cairn_agent;

-- Registered apply fn for the #208/ADR-0057 generic dispatcher (db/005) + cairn_reproject
-- heal/rebuild (db/039). Both verbs share the ONE fn and the SAME projection_tables list
-- (pending and identify both fold into the same chart_identity_state overlay). #214 +
-- steady-state discipline: converge these rows to the migration text on every connect,
-- but stay write-free once already converged (no dead tuples, no validate-trigger fire).
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('identity.pending.asserted',  'chart_identity_state_apply', ARRAY['chart_identity_state'], 10, TRUE),
    ('identity.identify.asserted', 'chart_identity_state_apply', ARRAY['chart_identity_state'], 10, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
