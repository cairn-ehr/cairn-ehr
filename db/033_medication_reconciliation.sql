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

-- 3. Register both reconciliation verbs' structural floor + hard twin requirement in the
--    #173 registry (replaces the copied cairn_event_twin dispatch chain; the single db/005
--    dispatcher reads these rows). Placed after the floor fn above so the fail-closed
--    registry trigger (db/005) sees cairn_check_medication_reconciliation(text, jsonb)
--    declared at load time.
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.medication-reconciliation.asserted', 'cairn_check_medication_reconciliation', 'medication reconciliation requires a non-empty authored twin (§3.13/§3.3)'),
    ('clinical.medication-separation.asserted',     'cairn_check_medication_reconciliation', 'medication reconciliation requires a non-empty authored twin (§3.13/§3.3)')
-- DO UPDATE, not DO NOTHING (#214): replay must converge the row to the migration text;
-- the IS DISTINCT FROM guard keeps the steady-state replay write-free (see db/031's
-- medication registration for the rationale).
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (cairn_event_twin_check.check_fn, cairn_event_twin_check.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);

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
    -- ADR-0052: sealed rows carry ciphertext in body; the clear payload lives
    -- in event_clear (populated by the door BEFORE this row, same txn). NULL =
    -- sealed without custody here: nothing to project — honest degradation.
    p       jsonb := cairn_clear_payload(NEW);
    a       uuid  := (p ->> 'subject_a')::uuid;
    b       uuid  := (p ->> 'subject_b')::uuid;
    lo      uuid  := LEAST(a, b);
    hi      uuid  := GREATEST(a, b);
    v_state text  := CASE WHEN NEW.event_type = 'clinical.medication-reconciliation.asserted'
                          THEN 'reconciled' ELSE 'separated' END;
    v_pa    uuid;
    v_pb    uuid;
BEGIN
    IF p IS NULL THEN RETURN NULL; END IF;
    PERFORM pg_advisory_xact_lock(x'4341524E4D52'::bigint);  -- 'CARNMR'

    -- Cross-patient reconciliation guard (#192 scenario B, resolving the #177 design
    -- decision): when BOTH subject threads' patients are KNOWN locally and differ, a
    -- reconciliation is refused at the LOCAL door — the caller is linking two charts'
    -- medications, almost certainly a wrong-chart click, and nothing has accepted the
    -- event yet. Offline-first is preserved: an unknown thread passes honestly
    -- (principle 4), and the late-arriving contradiction is surfaced read-time by the
    -- medication_group_cross_patient view below, whatever the arrival order. The
    -- sync-apply path never raises (a node-local veto would fork the event set) — the
    -- view is its surface too. SEPARATION is never guarded: it is the repair primitive
    -- for exactly this inconsistency and must always pass (never block the fix).
    IF v_state = 'reconciled'
       AND current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on' THEN
        v_pa := cairn_medication_thread_patient(lo);
        v_pb := cairn_medication_thread_patient(hi);
        IF v_pa IS NOT NULL AND v_pb IS NOT NULL AND v_pa <> v_pb THEN
            RAISE EXCEPTION
                'medication reconciliation: threads % and % belong to different patients (% vs %) — cross-patient reconciliation refused; separate/re-assert on the right chart instead (issues #192/#177)',
                lo, hi, v_pa, v_pb;
        END IF;
    END IF;

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

BEGIN;

-- 6. Helper: every asserted thread -> (patient_id, group_id). A thread with no
--    group_member row collapses to itself (COALESCE).
CREATE OR REPLACE VIEW medication_thread_group AS
SELECT s.medication_id,
       s.patient_id,
       COALESCE(gm.group_id, s.medication_id) AS group_id
FROM medication_statement s
LEFT JOIN medication_group_member gm ON gm.medication_id = s.medication_id;
GRANT SELECT ON medication_thread_group TO cairn_agent;

-- 7. Group status by LATEST-EFFECTIVE WINS (ADR-0047 ruling 3). Compare the max
--    effective sort key across active members' dose points vs ceased members'
--    cessations. Ties (equal effective keys) resolve to 'active' (documented
--    tiebreak — keep the drug visible). A single-member group can never be "mixed"
--    (all-active or all-ceased), so this reduces EXACTLY to slice-1/2 semantics.
--    MAX(... COLLATE "C") forces byte-order max, and the final active-vs-ceased
--    comparison is likewise pinned with COLLATE "C" (ADR-0045) so the winner never
--    depends on the node's database collation.
CREATE OR REPLACE VIEW medication_group_status AS
WITH active_eff AS (
    SELECT g.group_id,
           MAX(cairn_dose_effective_sort_key(de.effective_value, de.hlc_wall) COLLATE "C") AS eff
    FROM medication_dose_event de
    JOIN medication_thread_group g ON g.medication_id = de.medication_id
    WHERE NOT EXISTS (SELECT 1 FROM medication_cessation cc WHERE cc.medication_id = de.medication_id)
    GROUP BY g.group_id
),
ceased_eff AS (
    SELECT g.group_id,
           MAX(cairn_dose_effective_sort_key(c.stopped_value, c.hlc_wall) COLLATE "C") AS eff
    FROM medication_cessation c
    JOIN medication_thread_group g ON g.medication_id = c.medication_id
    GROUP BY g.group_id
)
SELECT grp.group_id, grp.patient_id,
       CASE WHEN ce.eff IS NULL THEN 'active'
            WHEN ae.eff IS NULL THEN 'ceased'
            WHEN ae.eff >= ce.eff COLLATE "C" THEN 'active'
            ELSE 'ceased' END AS status
FROM (SELECT DISTINCT group_id, patient_id FROM medication_thread_group) grp
LEFT JOIN active_eff ae ON ae.group_id = grp.group_id
LEFT JOIN ceased_eff ce ON ce.group_id = grp.group_id;
GRANT SELECT ON medication_group_status TO cairn_agent;

-- 8. Group current dose = latest-EFFECTIVE dose point across ACTIVE members only
--    (a ceased member's doses are not "current"). Correction overlay applied per
--    point (thread-scoped join, as in db/032). One row per group.
CREATE OR REPLACE VIEW medication_group_current_dose AS
SELECT DISTINCT ON (g.group_id)
    g.group_id, g.patient_id, de.dose_event_id,
    CASE WHEN corr.corrected_dose_event_id IS NOT NULL THEN corr.amount ELSE de.amount END AS amount,
    CASE WHEN corr.corrected_dose_event_id IS NOT NULL THEN corr.unit   ELSE de.unit   END AS unit,
    de.effective_value, de.effective_precision
FROM medication_dose_event de
JOIN medication_thread_group g ON g.medication_id = de.medication_id
LEFT JOIN medication_dose_correction corr
    ON corr.corrected_dose_event_id = de.dose_event_id AND corr.medication_id = de.medication_id
WHERE NOT EXISTS (SELECT 1 FROM medication_cessation cc WHERE cc.medication_id = de.medication_id)
ORDER BY g.group_id,
         cairn_dose_effective_sort_key(de.effective_value, de.hlc_wall) COLLATE "C" DESC,
         de.hlc_wall DESC, de.hlc_counter DESC, de.origin COLLATE "C" DESC, de.content_address DESC;
GRANT SELECT ON medication_group_current_dose TO cairn_agent;

-- 9. Group last dose = latest-EFFECTIVE dose point across ALL members (for the past
--    view — the last recorded dose before stopping, incl. now-ceased members).
CREATE OR REPLACE VIEW medication_group_last_dose AS
SELECT DISTINCT ON (g.group_id)
    g.group_id, g.patient_id,
    CASE WHEN corr.corrected_dose_event_id IS NOT NULL THEN corr.amount ELSE de.amount END AS amount,
    CASE WHEN corr.corrected_dose_event_id IS NOT NULL THEN corr.unit   ELSE de.unit   END AS unit
FROM medication_dose_event de
JOIN medication_thread_group g ON g.medication_id = de.medication_id
LEFT JOIN medication_dose_correction corr
    ON corr.corrected_dose_event_id = de.dose_event_id AND corr.medication_id = de.medication_id
ORDER BY g.group_id,
         cairn_dose_effective_sort_key(de.effective_value, de.hlc_wall) COLLATE "C" DESC,
         de.hlc_wall DESC, de.hlc_counter DESC, de.origin COLLATE "C" DESC, de.content_address DESC;
GRANT SELECT ON medication_group_last_dose TO cairn_agent;

-- 10. Group's latest cessation (stopped info for the past view). One row per group.
CREATE OR REPLACE VIEW medication_group_cessation AS
SELECT DISTINCT ON (g.group_id)
    g.group_id, g.patient_id, c.stopped_value, c.stopped_precision, c.reason
FROM medication_cessation c
JOIN medication_thread_group g ON g.medication_id = c.medication_id
ORDER BY g.group_id,
         cairn_dose_effective_sort_key(c.stopped_value, c.hlc_wall) COLLATE "C" DESC,
         c.hlc_wall DESC, c.hlc_counter DESC, c.origin COLLATE "C" DESC, c.content_address DESC;
GRANT SELECT ON medication_group_cessation TO cairn_agent;

-- 11. Group display fields from the canonical member's statement: prefer the exact
--     group_id member if its assert is local, else the min-UUID member present.
--     Carries the as-asserted dose (dose_amount/dose_unit) so the current/past views
--     can fall back to it for a member with a statement but NO dose timeline row
--     (see the fallback note on patient_medication_current below). The dose columns
--     are APPENDED at the end of the select list, never inserted mid-list: a later
--     CREATE OR REPLACE that repositions an existing view column fails ("cannot change
--     name of view column"), so column additions to this internal view must stay
--     append-only to survive an in-place upgrade over a prior-shaped view.
CREATE OR REPLACE VIEW medication_group_display AS
SELECT DISTINCT ON (g.group_id)
    g.group_id, g.patient_id, s.term, s.inn_code, s.formulation, s.sig, s.info_source,
    s.started_value, s.started_precision,
    to_timestamp(s.hlc_wall / 1000.0) AS asserted_at,
    s.dose_amount, s.dose_unit
FROM medication_statement s
JOIN medication_thread_group g ON g.medication_id = s.medication_id
ORDER BY g.group_id, (s.medication_id = g.group_id) DESC, s.medication_id;
GRANT SELECT ON medication_group_display TO cairn_agent;

-- 12. Rework the current/past views to emit ONE row per group. CRITICAL: keep the
--     EXACT SAME COLUMN SET as db/032 (replay safety — db/031/032 replay on every
--     connect; widening/renaming breaks reconnect). Only rows + dose/status source
--     change. medication_id = group_id (the stable group key; = the thread itself
--     for an un-reconciled thread, so slice-1/2 behavior is preserved).
--     DOSE FALLBACK (retained from db/032, legibility across time / principle 11):
--     the dose prefers the timeline winner (medication_group_current_dose / _last_dose)
--     but falls back to the canonical member's as-asserted statement dose when the group
--     has NO timeline row. That case is NOT hypothetical: db/032's medication_dose_seed_initial
--     trigger only seeds a point-0 dose event for asserts INSERTED after the trigger
--     exists — a clinical.medication.asserted already in event_log from before this node
--     first loaded db/032 (a slice-1-only history) has a statement but no dose event, and
--     re-running db/032 on connect does NOT backfill it. Dropping the fallback would make
--     such an old, still-current med render with a NULL dose on reconnect. The fallback is
--     a CASE on timeline-row PRESENCE (cd/ld.group_id IS NOT NULL), NOT a COALESCE on the
--     amount — a present timeline row with a NULL amount is a legitimate honest-unknown
--     (a correct-to-unknown / unquantified change, db/032 §9) and must stay NULL, never
--     silently revert to the stale as-asserted value.
CREATE OR REPLACE VIEW patient_medication_current AS
SELECT d.group_id AS medication_id, d.patient_id, d.term, d.inn_code, d.formulation,
       CASE WHEN cd.group_id IS NOT NULL THEN cd.amount ELSE d.dose_amount END AS dose_amount,
       CASE WHEN cd.group_id IS NOT NULL THEN cd.unit   ELSE d.dose_unit   END AS dose_unit,
       d.sig, d.info_source, d.started_value, d.started_precision, d.asserted_at
FROM medication_group_display d
JOIN medication_group_status st ON st.group_id = d.group_id
LEFT JOIN medication_group_current_dose cd ON cd.group_id = d.group_id
WHERE st.status = 'active';
GRANT SELECT ON patient_medication_current TO cairn_agent;

CREATE OR REPLACE VIEW patient_medication_past AS
SELECT d.group_id AS medication_id, d.patient_id, d.term, d.inn_code, d.formulation,
       CASE WHEN ld.group_id IS NOT NULL THEN ld.amount ELSE d.dose_amount END AS dose_amount,
       CASE WHEN ld.group_id IS NOT NULL THEN ld.unit   ELSE d.dose_unit   END AS dose_unit,
       d.sig, d.info_source, d.started_value, d.started_precision, d.asserted_at,
       gc.stopped_value, gc.stopped_precision, gc.reason
FROM medication_group_display d
JOIN medication_group_status st ON st.group_id = d.group_id
LEFT JOIN medication_group_last_dose ld ON ld.group_id = d.group_id
LEFT JOIN medication_group_cessation gc ON gc.group_id = d.group_id
WHERE st.status = 'ceased';
GRANT SELECT ON patient_medication_past TO cairn_agent;

-- 13. Rework the reconciliation flag: fire only when ACTIVE threads sharing a dup_key
--     span MORE THAN ONE distinct group (un-reconciled duplicates). Reconciling
--     collapses them to one group -> no flag; separating re-splits -> flag returns.
--     DEVIATION FROM THE DRAFT PLAN: the original plan was to RENAME `thread_count`
--     to `group_count` via DROP+CREATE, reasoning that db/033 loading after db/031
--     within one connect makes it replay-safe. Verified empirically (direct psql
--     repro) that this is FALSE: connect_and_load_schema replays over the SAME live
--     schema on every connect (never a fresh DB), and db/031 replays FIRST and
--     unconditionally re-issues `CREATE OR REPLACE VIEW ... thread_count ...`
--     against whatever the view currently is. Once a prior connect's db/033 step has
--     renamed the live column to `group_count`, db/031's OWN replay then fails with
--     "cannot change name of view column" on the very next connect — before db/033
--     ever gets a chance to run. Since db/031 is out of scope for this task, the
--     column keeps its ORIGINAL name `thread_count` (same name/type/position as
--     db/031 — the identical same-column-set replay rule already applied to
--     current/past above); only the COUNTING SEMANTICS move from per-thread to
--     per-group, via a plain CREATE OR REPLACE (no DROP needed).
CREATE OR REPLACE VIEW patient_medication_reconciliation_flag AS
SELECT patient_id,
       coalesce(inn_code, lower(btrim(term) COLLATE "C")) AS dup_key,
       count(DISTINCT group_id)                           AS thread_count,
       array_agg(DISTINCT medication_id ORDER BY medication_id) AS medication_ids
FROM (
    SELECT s.patient_id, s.medication_id, s.inn_code, s.term,
           COALESCE(gm.group_id, s.medication_id) AS group_id
    FROM medication_statement s
    LEFT JOIN medication_group_member gm ON gm.medication_id = s.medication_id
    WHERE NOT EXISTS (SELECT 1 FROM medication_cessation c WHERE c.medication_id = s.medication_id)
) t
GROUP BY patient_id, coalesce(inn_code, lower(btrim(term) COLLATE "C"))
HAVING count(DISTINCT group_id) > 1;
GRANT SELECT ON patient_medication_reconciliation_flag TO cairn_agent;

-- 9. Cross-patient group surface (#192 scenario B / #177). A reconciled group whose
--    member threads' statements span MORE THAN ONE patient is a standing wrong-chart
--    hazard: medication_group_status emits one row per (group, patient), so
--    patient_medication_current shows the group under both charts with mixed
--    attribution. Read-time and arrival-order independent (the whole point: a
--    reconciliation may legitimately pass the local door while a subject's statement
--    is still in flight — this view lights up whenever the contradiction lands,
--    whichever door and order it arrived by, and clears when a separation repairs
--    it). Advisory worklist: surface, never auto-separate (flag-never-suppress).
-- A member's patient is derived through cairn_medication_thread_patient — the SAME
-- source the write-guard reads (statement, else orphan cessation), NOT a bare join to
-- medication_statement (PR #219 review, finding 3): a thread known locally only via a
-- cessation carries a real patient the guard sees, so a group spanning it is a genuine
-- cross-patient hazard that a statement-only join would hide on the very read-time
-- surface meant to catch the late-arriving case. A thread whose patient is still
-- unknown (NULL) contributes nothing — it cannot yet evidence a contradiction.
CREATE OR REPLACE VIEW medication_group_cross_patient AS
SELECT gm.group_id,
       array_agg(DISTINCT tp.patient_id ORDER BY tp.patient_id) AS patients,
       count(DISTINCT tp.patient_id)                            AS patient_count,
       count(DISTINCT gm.medication_id)                         AS member_count
FROM medication_group_member gm
CROSS JOIN LATERAL (
    SELECT cairn_medication_thread_patient(gm.medication_id) AS patient_id
) tp
WHERE tp.patient_id IS NOT NULL
GROUP BY gm.group_id
HAVING count(DISTINCT tp.patient_id) > 1;
GRANT SELECT ON medication_group_cross_patient TO cairn_agent;

COMMIT;
