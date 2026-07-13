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
--    MAX(... COLLATE "C") forces byte-order max (ADR-0045).
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
            WHEN ae.eff >= ce.eff THEN 'active'
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
CREATE OR REPLACE VIEW medication_group_display AS
SELECT DISTINCT ON (g.group_id)
    g.group_id, g.patient_id, s.term, s.inn_code, s.formulation, s.sig, s.info_source,
    s.started_value, s.started_precision,
    to_timestamp(s.hlc_wall / 1000.0) AS asserted_at
FROM medication_statement s
JOIN medication_thread_group g ON g.medication_id = s.medication_id
ORDER BY g.group_id, (s.medication_id = g.group_id) DESC, s.medication_id;
GRANT SELECT ON medication_group_display TO cairn_agent;

-- 12. Rework the current/past views to emit ONE row per group. CRITICAL: keep the
--     EXACT SAME COLUMN SET as db/032 (replay safety — db/031/032 replay on every
--     connect; widening/renaming breaks reconnect). Only rows + dose/status source
--     change. medication_id = group_id (the stable group key; = the thread itself
--     for an un-reconciled thread, so slice-1/2 behavior is preserved).
--     NOTE (intentional, not an oversight): unlike db/032, the dose here is sourced
--     ONLY from the timeline (medication_group_current_dose / _last_dose), with no
--     COALESCE to the as-asserted statement dose. That fallback is unnecessary because
--     db/032's medication_dose_seed_initial trigger seeds a point-0 dose event on EVERY
--     clinical.medication.asserted (local or replicated), so any thread with a statement
--     has >= 1 dose point — the seedless-assert case db/032's fallback guarded cannot
--     arise once db/032 is loaded (which connect_and_load_schema guarantees before any
--     assert is applied).
CREATE OR REPLACE VIEW patient_medication_current AS
SELECT d.group_id AS medication_id, d.patient_id, d.term, d.inn_code, d.formulation,
       cd.amount AS dose_amount, cd.unit AS dose_unit,
       d.sig, d.info_source, d.started_value, d.started_precision, d.asserted_at
FROM medication_group_display d
JOIN medication_group_status st ON st.group_id = d.group_id
LEFT JOIN medication_group_current_dose cd ON cd.group_id = d.group_id
WHERE st.status = 'active';
GRANT SELECT ON patient_medication_current TO cairn_agent;

CREATE OR REPLACE VIEW patient_medication_past AS
SELECT d.group_id AS medication_id, d.patient_id, d.term, d.inn_code, d.formulation,
       ld.amount AS dose_amount, ld.unit AS dose_unit,
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

COMMIT;
