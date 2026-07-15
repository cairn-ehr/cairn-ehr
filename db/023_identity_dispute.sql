-- db/023_identity_dispute.sql
-- Cairn — §5.7 `dispute` + the chart trust-state projection (identity piece C3).
--
-- WHAT: the patient-initiated "I was never there" front door (§5.5(b) identity theft).
-- A dispute flags a chart *under-review* and enters a triage worklist; a later
-- resolution clears it. This slice also builds the §5.7 "projection-side contract" —
-- the chart trust state (confirmed / under-review) — that every remaining identity
-- event (identify, reattribute, and the §5.2 coherence check) will compose into.
--
-- The safety-critical write door submit_event (db/005) is REUSED verbatim: the two new
-- types register in event_type_class and add a branch to the cairn_event_twin hook. A
-- dispute is ADDITIVE (it annotates trust; it never erases, moves, or blocks anything),
-- so no attestation is forced unless a responsibility-bearing contributor is named — the
-- same low-ceremony path as the C1 link (db/018). Additive DDL only; no submit_event
-- re-declaration, no floor change, no SCHEMA-version bump.

BEGIN;

-- 1. Register the two additive dispute event types (fail-closed registry, ADR-0010).
--    additive + targets_other_author=FALSE: a dispute neither suppresses nor targets
--    another author's event, so the db/005 gate requires NO attestation for a clerk /
--    patient-portal dispute; a clinician who takes responsibility simply includes a
--    responsibility-bearing contributor, which the gate already attests.
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('identity.dispute.asserted', 'additive', FALSE),
    ('identity.dispute.resolved', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. The §5.7 structural floor. Culture-neutral: validates STRUCTURE only — a valid
--    dispute_id, a valid subject uuid, and a required non-empty descriptive field whose
--    key differs by type (`reason` opens a dispute, `resolution` closes one). Each
--    violation is a distinct legible exception (the cairn_check_link_assertion pattern).
CREATE OR REPLACE FUNCTION cairn_check_dispute_assertion(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p         jsonb := b -> 'payload';
    v_field   text;   -- the descriptive-field key required for THIS type
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'dispute assertion: missing payload';
    END IF;
    -- dispute_id: present, string, valid uuid (its own identity — a chart may carry
    -- several concurrent, independently-resolvable disputes).
    IF jsonb_typeof(p -> 'dispute_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'dispute assertion: dispute_id must be a uuid string (§5.7)';
    END IF;
    BEGIN
        PERFORM (p ->> 'dispute_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'dispute assertion: dispute_id must be a valid uuid (§5.7)';
    END;
    -- subject: present, string, valid uuid (the patient chart under dispute). No
    -- cross-existence check — a dispute may arrive before or independently of the chart
    -- it names (offline-first, set-union; the safety signal must exist without the body).
    IF jsonb_typeof(p -> 'subject') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'dispute assertion: subject must be a uuid string (§5.7)';
    END IF;
    BEGIN
        PERFORM (p ->> 'subject')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'dispute assertion: subject must be a valid uuid (§5.7)';
    END;
    -- The descriptive field: `reason` opens, `resolution` closes. Present + non-empty
    -- (§4.1 ladder; value-open — "unknown" is honest, "" is fabrication-only; principle 4).
    v_field := CASE WHEN p_type = 'identity.dispute.resolved' THEN 'resolution' ELSE 'reason' END;
    IF jsonb_typeof(p -> v_field) IS DISTINCT FROM 'string'
       OR length(trim(p ->> v_field)) = 0 THEN
        RAISE EXCEPTION 'dispute assertion: % must be a non-empty string (§4.1)', v_field;
    END IF;
END;
$$;

-- 3. Register both dispute verbs' structural floor + hard twin requirement in the #173
--    registry (replaces the copied cairn_event_twin dispatch chain; the single db/005
--    dispatcher reads these rows). Placed after the floor fn above so the fail-closed
--    registry trigger (db/005) sees cairn_check_dispute_assertion(text, jsonb) declared.
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('identity.dispute.asserted', 'cairn_check_dispute_assertion', 'identity dispute assertion requires a non-empty authored twin (§5.7)'),
    ('identity.dispute.resolved', 'cairn_check_dispute_assertion', 'identity dispute assertion requires a non-empty authored twin (§5.7)')
ON CONFLICT (event_type) DO NOTHING;

-- 4. chart_dispute: the standing-dispute overlay (same overlay discipline as db/018's
--    patient_link, but keyed by the dispute's own id). One row per dispute_id; the
--    latest-HLC assertion wins the `state`. Unlike patient_link there is NO connected-
--    component recompute — a dispute is a single-row standing fact — so the trigger is a
--    plain HLC-guarded upsert (cheaper than C1; no BFS, no oversize guard).
CREATE TABLE IF NOT EXISTS chart_dispute (
    dispute_id  UUID    PRIMARY KEY,
    subject     UUID    NOT NULL,
    state       TEXT    NOT NULL CHECK (state IN ('open', 'resolved')),
    detail      TEXT,                       -- the winning assertion's descriptive text: its `reason` (open) or `resolution` (resolved)
    hlc_wall    BIGINT  NOT NULL,
    hlc_counter INTEGER NOT NULL,
    origin      TEXT    NOT NULL,
    content_address BYTEA NOT NULL,   -- winning event's content address; the #115 tiebreak
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
-- Additive widening (#115 → issue #207): see db/018's patient_link note — the CREATE
-- no-ops on a pre-widening DB; nullable ALTER, new upserts always write it.
-- Guarded by migration_replay_widening.rs.
ALTER TABLE chart_dispute ADD COLUMN IF NOT EXISTS content_address BYTEA;
GRANT SELECT ON chart_dispute TO cairn_agent;
-- The chart_trust VIEW's hot lookup AND the triage worklist are both "standing OPEN
-- disputes for a subject"; index exactly that partial set so neither cliffs as the
-- dispute history grows.
CREATE INDEX IF NOT EXISTS chart_dispute_open_subject_idx
    ON chart_dispute (subject) WHERE state = 'open';

-- Incremental maintenance: fold exactly the one new dispute event into the overlay. The
-- whole row overlays atomically only when the incoming HLC is strictly greater than the
-- stored one (ON CONFLICT ... WHERE) — so out-of-order arrival (a resolution landing
-- before the open it closes) converges to the highest-HLC assertion.
CREATE OR REPLACE FUNCTION chart_dispute_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p        jsonb := NEW.body;
    v_state  text  := CASE WHEN NEW.event_type = 'identity.dispute.resolved' THEN 'resolved' ELSE 'open' END;
    -- The descriptive field's key differs by type (reason opens, resolution closes);
    -- store whichever one this assertion carries in the neutrally-named `detail` column
    -- so the overlay row is self-describing without a misleading column name.
    v_detail text  := CASE WHEN NEW.event_type = 'identity.dispute.resolved'
                           THEN p ->> 'resolution' ELSE p ->> 'reason' END;
    v_cur record;
BEGIN
    -- #157: detect a Byzantine HLC-triple collision against the current dispute state and record
    -- an advisory signal before overlaying. Placed before the subject-consistency guard so the
    -- collision is observed regardless of which door (local submit / remote apply) we are on.
    SELECT hlc_wall, hlc_counter, origin, content_address
      INTO v_cur
      FROM chart_dispute WHERE dispute_id = (p ->> 'dispute_id')::uuid;
    IF FOUND AND cairn_hlc_triple_collision(
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
            v_cur.hlc_wall, v_cur.hlc_counter, v_cur.origin, v_cur.content_address) THEN
        PERFORM cairn_record_hlc_collision(
            'chart_dispute', p ->> 'dispute_id',
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
            NEW.content_address, v_cur.content_address);
    END IF;

    -- Subject-consistency guard, split exactly like C1's oversize guard (db/018):
    -- FAIL LOUD locally, CONVERGE on sync. A dispute_id names ONE chart for its whole
    -- life, so a second assertion binding the SAME dispute_id to a DIFFERENT subject is
    -- malformed. On the LOCAL submit door (cairn.remote_apply unset) nothing has been
    -- accepted yet, so we refuse it — catching a caller bug at source and losing no data.
    -- On the SYNC-APPLY path (apply_remote_event sets cairn.remote_apply='on', db/020) we
    -- must NOT raise: peers already hold this validly-signed event and this is a node-local
    -- check, so vetoing would fork the event set between honest nodes. There the subject
    -- simply converges to the highest-HLC assertion via the upsert below — deterministic
    -- and arrival-order-independent (never "pin first-seen", which IS order-dependent and
    -- would diverge under set-union sync).
    IF current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on'
       AND EXISTS (SELECT 1 FROM chart_dispute
                   WHERE dispute_id = (p ->> 'dispute_id')::uuid
                     AND subject   <> (p ->> 'subject')::uuid) THEN
        RAISE EXCEPTION
            'dispute %: subject cannot change — a dispute_id names one chart for life (§5.7)',
            p ->> 'dispute_id';
    END IF;

    INSERT INTO chart_dispute
        (dispute_id, subject, state, detail, hlc_wall, hlc_counter, origin, content_address)
    VALUES
        ((p ->> 'dispute_id')::uuid, (p ->> 'subject')::uuid, v_state, v_detail,
         NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (dispute_id) DO UPDATE SET
        subject     = EXCLUDED.subject,
        state       = EXCLUDED.state,
        detail      = EXCLUDED.detail,
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        content_address = EXCLUDED.content_address,
        updated_at  = clock_timestamp()
    -- content_address is the deterministic final tiebreaker (#115) so an HLC-triple collision
    -- settles open-vs-resolved identically on every node.
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        chart_dispute.hlc_wall, chart_dispute.hlc_counter, chart_dispute.origin,
        chart_dispute.content_address);
    RETURN NULL;  -- AFTER trigger
END;
$$;

DROP TRIGGER IF EXISTS chart_dispute_apply_trg ON event_log;
CREATE TRIGGER chart_dispute_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type IN ('identity.dispute.asserted', 'identity.dispute.resolved'))
    EXECUTE FUNCTION chart_dispute_apply();

-- 5. chart_trust: the §5.7 effective trust-state projection (confirmed / under-review).
--    Delivered as a thin VIEW — consistent with db/018's person_chart being a VIEW and
--    with the ADR-0001/Bet-B discipline: a chart's trust is a BOUNDED, INDEXED lookup
--    over chart_dispute (the partial index above), not a full-projection recompute.
--
--    Precedence is built so LATER slices ADD a branch, never rewrite this VIEW:
--      under-review  <- any standing OPEN dispute                 (THIS slice)
--      [under-review <- pending reattribution]                    (C4, §5.5 — future)
--      [under-review <- coherence-check demoted link]             (§5.2 feedback — future)
--      [unconfirmed  <- identity-pending registration]            (C4/C5, §5.4 — future)
--      confirmed     <- default (a subject with no row here; the read below coalesces)
--
--    A subject in the DEFAULT (confirmed) state has NO row here — that keeps the VIEW
--    tiny and makes the triage worklist a trivial `chart_dispute WHERE state='open'`.
CREATE OR REPLACE VIEW chart_trust AS
    SELECT subject AS patient_id, 'under-review'::text AS trust_state
    FROM chart_dispute
    WHERE state = 'open'
    GROUP BY subject;

GRANT SELECT ON chart_trust TO cairn_agent;

-- 6. Surface trust on the unified read. person_chart_trust COMPOSES on top of db/018's
--    person_chart (reusing its person_member union — no re-join) and tags every member
--    row with its OWN chart's trust, coalescing to 'confirmed' when unknown to chart_trust.
--    Trust attaches to the patient_id (the chart a dispute names), NOT the aggregated
--    person_id — whether an under-review member taints the whole person view is a
--    read-surface (API/UI) judgment above the foundation line.
--
--    Deliberately a SEPARATE view, not an extension of person_chart. person_chart is the
--    C1 read surface the API/UI tier builds on, so it must stay droppable-free: a later
--    migration extending it in place would force a DROP+CREATE (CREATE OR REPLACE cannot
--    add/shrink columns idempotently across the connect_and_load_schema reload), and a
--    bare DROP would abort node boot the moment any dependent view sits on person_chart.
--    Composing a new view sidesteps that entirely — CREATE OR REPLACE here is column-stable
--    across reloads, and this view is the one future trust-source slices (identify /
--    reattribute / §5.2 coherence) extend, keeping person_chart itself untouched.
--
--    NOTE: like person_chart, this lists a subject only once its patient_chart row exists.
--    A dispute that arrives before the disputed body still reports under-review via
--    chart_trust (the authoritative identity safety signal, queried directly); this view
--    is the convenience join for charts that have synced, NOT the complete safety surface.
-- Upgrade heal (issue #207): symmetric with db/018's person_chart heal — if this view
-- still has the pre-#115 narrow shape (no demo_content_address, via pc.*), the REPLACE
-- below would splice a column mid-list and abort boot. Normally db/018's CASCADE already
-- took this view down in the same load; this guard covers a partially-healed history
-- (e.g. an earlier load that committed db/018 but failed before this file). Steady-state
-- loads never drop.
DO $$
BEGIN
    IF to_regclass('public.person_chart_trust') IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM information_schema.columns
           WHERE table_schema = 'public' AND table_name = 'person_chart_trust'
             AND column_name = 'demo_content_address') THEN
        DROP VIEW person_chart_trust CASCADE;
    END IF;
END $$;

CREATE OR REPLACE VIEW person_chart_trust AS
    SELECT pc.*,
           COALESCE(ct.trust_state, 'confirmed') AS trust_state
    FROM person_chart pc
    LEFT JOIN chart_trust ct ON ct.patient_id = pc.patient_id;

GRANT SELECT ON person_chart_trust TO cairn_agent;

COMMIT;
