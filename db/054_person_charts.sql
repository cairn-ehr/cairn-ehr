-- db/054 — the person's chart set, and a duplicate flag over it (ADR-0076 decision 1).
--
-- WHY IN THE DATABASE. Every combined read — the medication list now, allergies when they
-- exist, the duplicate banner (R5) — must agree on which charts are "this person". One
-- function answers it, over db/018's person_member projection, so no two readers can
-- disagree about the set they are combining.
--
-- REPLAY-SAFE: CREATE OR REPLACE only; nothing here changes a table or a view.
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

-- 2. Un-reconciled duplicates ACROSS a chart set. patient_medication_reconciliation_flag
--    (db/033) groups by patient_id, so the same drug recorded on two LINKED charts is two
--    groups on two patients and is never flagged — on a combined list that is two unflagged
--    lines for one drug, a double-dose reading hazard. This is the same rule over the SET:
--    active threads on any chart in p_charts sharing a dup_key and spanning more than one
--    group. Returns the flagged GROUP ids (every group those threads display under).
--
--    DRIFT: the `coalesce('code:' || …)` dup_key expression below must stay byte-identical
--    to db/033's (and db/031's) view — medication_dup_key_drift.rs pins it. Note it is
--    written over the UNQUALIFIED column names `coding_system` / `coding_code` / `term`,
--    matching db/033's own shape exactly (db/033 groups the same inner projection, so its
--    dup_key never needed to qualify them) — an inner CTE first produces that same bare
--    shape here so the expression can be copied verbatim rather than adapted.
CREATE OR REPLACE FUNCTION cairn_medication_duplicate_groups(p_charts uuid[])
RETURNS SETOF uuid
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    WITH base AS (
        -- Bare column names on purpose (db/033's own inner-subquery shape): the dup_key
        -- expression below reads them unqualified, byte-identical to db/033's copies.
        SELECT s.patient_id, s.medication_id, mc.coding_system, mc.coding_code, s.term,
               COALESCE(gm.group_id, s.medication_id) AS group_id
        FROM medication_statement s
        LEFT JOIN medication_group_member gm ON gm.medication_id = s.medication_id
        LEFT JOIN medication_coding mc ON mc.medication_id = s.medication_id
        WHERE s.patient_id = ANY(p_charts)
          AND NOT EXISTS (SELECT 1 FROM medication_cessation c WHERE c.medication_id = s.medication_id)
    )
    SELECT DISTINCT t.group_id
    FROM (
        SELECT group_id,
               coalesce('code:' || (coding_system COLLATE "C") || '|' || (coding_code COLLATE "C"),
                        'term:' || lower(btrim(term) COLLATE "C")) AS dup_key
        FROM base
    ) t
    WHERE t.dup_key IN (
        SELECT dup_key FROM (
            SELECT group_id,
                   coalesce('code:' || (coding_system COLLATE "C") || '|' || (coding_code COLLATE "C"),
                            'term:' || lower(btrim(term) COLLATE "C")) AS dup_key
            FROM base
        ) u
        GROUP BY dup_key
        HAVING count(DISTINCT group_id) > 1
    )
$$;
GRANT EXECUTE ON FUNCTION cairn_medication_duplicate_groups(uuid[]) TO cairn_agent;

COMMIT;
