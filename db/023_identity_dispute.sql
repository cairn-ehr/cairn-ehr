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

-- 3. Extend the per-type twin hook. Dispute: run the floor + HARD-require an authored
--    twin (identity events are legible-critical, like link and demographics). This
--    CREATE OR REPLACE PRESERVES db/010's demographic branches, db/018's identity-link
--    branch, and db/015's honest-degrade fallback for every other type — it only adds
--    the dispute branch (submit_event itself is NEVER re-declared).
CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
    v_twin        text    := b ->> 'plaintext_twin';
    v_authored    boolean := v_twin IS NOT NULL AND length(regexp_replace(v_twin, '\s+', '', 'g')) > 0;
    v_demographic boolean := false;
    v_identity    boolean := false;
    v_dispute     boolean := false;
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
    ELSIF p_type IN ('identity.dispute.asserted', 'identity.dispute.resolved') THEN
        PERFORM cairn_check_dispute_assertion(p_type, b);
        v_dispute := true;
    END IF;

    -- Authored twin present → carry it verbatim (principle 11; the conformant path).
    IF v_authored THEN
        RETURN v_twin;
    END IF;

    -- Absent/blank twin: demographic, identity-link, AND dispute types HARD-require it;
    -- every other type degrades honestly to a flagged derived skeleton (ADR-0039).
    IF v_demographic THEN
        RAISE EXCEPTION 'submit_event: demographic assertion requires a non-empty authored twin (§4.5)';
    ELSIF v_identity THEN
        RAISE EXCEPTION 'submit_event: identity linkage assertion requires a non-empty authored twin (§5.7)';
    ELSIF v_dispute THEN
        RAISE EXCEPTION 'submit_event: identity dispute assertion requires a non-empty authored twin (§5.7)';
    END IF;
    RETURN cairn_twin_skeleton(p_type, b);
END;
$$;

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
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
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
BEGIN
    INSERT INTO chart_dispute
        (dispute_id, subject, state, detail, hlc_wall, hlc_counter, origin)
    VALUES
        ((p ->> 'dispute_id')::uuid, (p ->> 'subject')::uuid, v_state, v_detail,
         NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
    ON CONFLICT (dispute_id) DO UPDATE SET
        subject     = EXCLUDED.subject,
        state       = EXCLUDED.state,
        detail      = EXCLUDED.detail,
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        updated_at  = clock_timestamp()
    WHERE (EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin)
        > (chart_dispute.hlc_wall, chart_dispute.hlc_counter, chart_dispute.origin);
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
CREATE OR REPLACE VIEW person_chart_trust AS
    SELECT pc.*,
           COALESCE(ct.trust_state, 'confirmed') AS trust_state
    FROM person_chart pc
    LEFT JOIN chart_trust ct ON ct.patient_id = pc.patient_id;

GRANT SELECT ON person_chart_trust TO cairn_agent;

COMMIT;
