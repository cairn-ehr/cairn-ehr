-- db/054 — the person's chart set, a duplicate flag over it, and whether it holds a doubted
-- link (ADR-0076 decision 1).
--
-- WHY IN THE DATABASE. Every combined read — the medication list now, allergies when they
-- exist, the duplicate banner (R5) — must agree on which charts are "this person". One
-- function answers it, over db/018's person_member projection, so no two readers can
-- disagree about the set they are combining.
--
-- REPLAY-SAFE: CREATE OR REPLACE and CREATE INDEX IF NOT EXISTS only. The one change to an
-- existing table is an index (section 1); no column, constraint or view changes.
BEGIN;

-- 1. The chart set: every chart in p_patient's link component, or p_patient alone when it
--    has never been linked (person_member has no row for a chart no linkage event touched;
--    an unlinked chart that once had an edge maps to itself). Ordered for stable reads;
--    callers canonicalise in Rust (ChartSet) regardless.
CREATE OR REPLACE FUNCTION cairn_person_charts(p_patient uuid)
RETURNS SETOF uuid
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT m.patient_id
    FROM person_member m
    WHERE m.person_id = (SELECT person_id FROM person_member WHERE patient_id = p_patient)
    UNION
    SELECT p_patient
    ORDER BY 1
$$;
GRANT EXECUTE ON FUNCTION cairn_person_charts(uuid) TO cairn_agent;

-- The component lookup above filters person_member by person_id, which db/018 does not
-- index (its only key is patient_id). Without this, every chart open scans the whole link
-- projection — three times per window sign-off — against db/018's own stated discipline of
-- keeping chart reads bounded by the touched component. Idempotent on replay.
CREATE INDEX IF NOT EXISTS person_member_person_id_idx ON person_member (person_id);

-- 2. Un-reconciled duplicates ACROSS a chart set. patient_medication_reconciliation_flag
--    (db/033) groups by patient_id, so the same drug recorded on two LINKED charts is two
--    groups on two patients and is never flagged — on a combined list that is two unflagged
--    lines for one drug, a double-dose reading hazard. This is the same rule over the SET:
--    active threads on any chart in p_charts sharing a dup_key and spanning more than one
--    group. Returns the flagged GROUP ids (every group those threads display under).
--
--    DRIFT: the `coalesce('code:' || …)` dup_key expression below must stay identical (up
--    to whitespace — the guard normalises it) to db/033's LIVE view
--    (`patient_medication_reconciliation_flag`); medication_dup_key_drift.rs pins db/033
--    against THIS file. db/031 originally defined that view too, but db/033's `CREATE OR
--    REPLACE VIEW` supersedes it on every schema replay (connect_and_load_schema always
--    replays db/031 first), so db/031's copy is executed and then immediately overridden —
--    never the definition a read sees, so not one the guard needs to track (see db/033's
--    comment on that view, section 13, for why its column name stayed `thread_count`). Note the expression is written over
--    the UNQUALIFIED column names `coding_system` / `coding_code` / `term`, matching
--    db/033's own shape exactly (its inner subquery groups the same bare projection, so its
--    dup_key never needed to qualify them) — the `base`/`keyed` CTEs below produce that same
--    bare shape here so the expression can be copied verbatim rather than adapted.
CREATE OR REPLACE FUNCTION cairn_medication_duplicate_groups(p_charts uuid[])
RETURNS SETOF uuid
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    WITH base AS (
        -- Bare column names on purpose (db/033's own inner-subquery shape): `keyed` below
        -- reads them unqualified, exactly as db/033's dup_key copy does.
        SELECT s.patient_id, s.medication_id, mc.coding_system, mc.coding_code, s.term,
               COALESCE(gm.group_id, s.medication_id) AS group_id
        FROM medication_statement s
        LEFT JOIN medication_group_member gm ON gm.medication_id = s.medication_id
        LEFT JOIN medication_coding mc ON mc.medication_id = s.medication_id
        WHERE s.patient_id = ANY(p_charts)
          AND NOT EXISTS (SELECT 1 FROM medication_cessation c WHERE c.medication_id = s.medication_id)
    ),
    keyed AS (
        -- The ONE copy of the dup_key expression in this file: both the outer SELECT and
        -- the HAVING-grouped subquery below read it from here instead of each repeating it,
        -- so there is exactly one place in db/054 that can drift from db/033.
        SELECT group_id,
               coalesce('code:' || (coding_system COLLATE "C") || '|' || (coding_code COLLATE "C"),
                        'term:' || lower(btrim(term) COLLATE "C")) AS dup_key
        FROM base
    )
    SELECT DISTINCT group_id
    FROM keyed
    WHERE dup_key IN (
        SELECT dup_key FROM keyed GROUP BY dup_key HAVING count(DISTINCT group_id) > 1
    )
$$;
GRANT EXECUTE ON FUNCTION cairn_medication_duplicate_groups(uuid[]) TO cairn_agent;

-- 3. Whether a chart set holds a link this node DOUBTS — an input to the medication read's
--    wrong-chart hazard rule (cairn-node medication/read.rs, is_wrong_chart_hazard).
--
--    ADR-0076 decision 1 combines every standing link, including an un-attested one the
--    node's hard veto (db/016) would refuse at its own door. Such a pair may be two people,
--    so a medication group spanning it must stay withheld from sign-off, as it was before
--    the combined read. Two ways a link is doubted:
--      (a) db/018 flagged it on arrival (link_veto_flag), or
--      (b) it is an un-attested standing link that trips cairn_has_hard_veto NOW. db/018
--          evaluates the veto only when the link arrives, so demographics arriving later
--          (a peer's link syncing ahead of the clashing DOB) never raise the flag — issue
--          #220. Evaluating it here at read time closes that gap for this read, whatever
--          #220's fix to the flag itself turns out to be.
--    "Attested" is read from the STORED patient_link.attested column (#701; R2a's one
--    definition: an attester key is present AND cairn_attestation_vouched held when the
--    winner was applied). A human-attested link is the human decision the veto exists to
--    force, so it is never doubted here.
--
--    SECURITY DEFINER is no longer strictly required: the old reasons
--    (cairn_attestation_vouched locked away by db/001; event_log attester columns under the
--    #405 column floor) no longer apply, and every remaining callee is granted directly to
--    cairn_agent. It is kept so this slice makes no privilege change (issue #718 decides
--    whether to drop it); the answer is one boolean about a set the caller already holds,
--    and search_path is pinned.
CREATE OR REPLACE FUNCTION cairn_chart_set_has_doubted_link(p_charts uuid[])
RETURNS boolean
LANGUAGE sql STABLE
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    SELECT EXISTS (
        SELECT 1 FROM link_veto_flag f
        WHERE f.low = ANY(p_charts) AND f.high = ANY(p_charts)
    ) OR EXISTS (
        -- #701: the STORED winner attestation (R2a, ADR-0076 decision 5 - one definition,
        -- evaluated when the winner was applied). Never re-derive it through event_log: that
        -- is a second spelling, and the join dropped a legacy row whose content_address is
        -- NULL (pre-#115).
        SELECT 1
        FROM patient_link pl
        WHERE pl.state = 'link'
          AND pl.low = ANY(p_charts) AND pl.high = ANY(p_charts)
          AND NOT pl.attested
          AND cairn_has_hard_veto(pl.low, pl.high)
    )
$$;
REVOKE EXECUTE ON FUNCTION cairn_chart_set_has_doubted_link(uuid[]) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_chart_set_has_doubted_link(uuid[]) TO cairn_agent;

COMMIT;
