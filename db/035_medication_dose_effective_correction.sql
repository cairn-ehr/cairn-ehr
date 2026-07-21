-- 035_medication_dose_effective_correction.sql — slice 5 of clinical.medication (data-model §3.3).
--
-- Extends the slice-2 dose-correction overlay (db/032) so a correction PATCHES a targeted
-- dose point per-field: dose (amount+unit), effective (value+precision), and the point's
-- clinical reason. Omit a group = keep; name it = set; list it in `strike` = set-unknown.
-- The CORRECTED effective date drives current-dose winner selection (bitemporal repair,
-- not a display label). db/031-034 UNTOUCHED — all slice-5 SQL is here (ADR-0050).
--
-- Convergence: the overlay stays ONE row per corrected point, highest-HLC-wins WHOLESALE
-- (cairn_hlc_overlay_wins), so set-union sync stays convergent. Per-field patch therefore
-- applies WITHIN one correction (vs the original point); a later correction supersedes an
-- earlier one rather than field-merging (documented boundary; field-merge would need
-- per-field HLC tracking).
BEGIN;

-- 1. Extend the correction overlay with effective + note + per-group touched-flags. The
--    flags disambiguate "struck to NULL" from "untouched" — a nullable value column
--    cannot. Added nullable so a pre-035 row (flags NULL) is distinguishable; backfilled
--    in step 2.
ALTER TABLE medication_dose_correction ADD COLUMN IF NOT EXISTS effective_value     TEXT;
ALTER TABLE medication_dose_correction ADD COLUMN IF NOT EXISTS effective_precision TEXT;
ALTER TABLE medication_dose_correction ADD COLUMN IF NOT EXISTS note                TEXT;
ALTER TABLE medication_dose_correction ADD COLUMN IF NOT EXISTS dose_corrected      BOOLEAN;
ALTER TABLE medication_dose_correction ADD COLUMN IF NOT EXISTS effective_corrected BOOLEAN;
ALTER TABLE medication_dose_correction ADD COLUMN IF NOT EXISTS reason_corrected    BOOLEAN;

-- 2. Backfill pre-035 rows (idempotent; guarded on dose_corrected IS NULL). A slice-2
--    correction row was a whole-row DOSE correction, and `reason` held the correction-why
--    (now `note`). New rows always set the flags, so this touches a legacy row at most
--    once and never re-clobbers a new one.
UPDATE medication_dose_correction
   SET dose_corrected      = TRUE,
       effective_corrected = FALSE,
       reason_corrected    = FALSE,
       note                = reason,
       reason              = NULL
 WHERE dose_corrected IS NULL;

-- 3. Extend the correction floor (whole fn re-created; the dose-change branch is
--    byte-identical to db/032, the correction branch gains strike/patch validation).
--    Registry mapping (db/032) is untouched — same fn name/signature.
CREATE OR REPLACE FUNCTION cairn_check_medication_dose(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'medication dose: missing payload';
    END IF;
    IF jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication dose: medication_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (p ->> 'medication_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication dose: medication_id must be a valid uuid';
    END;

    IF p_type = 'clinical.medication-dose-change.asserted' THEN
        IF jsonb_typeof(p -> 'info_source') IS DISTINCT FROM 'string'
           OR length(btrim(p ->> 'info_source')) = 0 THEN
            RAISE EXCEPTION 'medication dose-change: info_source must be a non-empty string';
        END IF;
        IF NOT (
            (p -> 'dose' ->> 'amount') IS NOT NULL OR (p -> 'dose' ->> 'unit') IS NOT NULL
            OR (p -> 'effective' ->> 'value') IS NOT NULL
            OR COALESCE(jsonb_typeof(p -> 'reason') = 'string' AND length(btrim(p ->> 'reason')) > 0, FALSE)
        ) THEN
            RAISE EXCEPTION 'medication dose-change: must carry a dose, an effective date, or a reason (principle 4 floor)';
        END IF;
    ELSIF p_type = 'clinical.medication-dose-correction.asserted' THEN
        IF jsonb_typeof(p -> 'corrects') IS DISTINCT FROM 'string' THEN
            RAISE EXCEPTION 'medication dose-correction: corrects must be a uuid string';
        END IF;
        BEGIN
            PERFORM (p ->> 'corrects')::uuid;
        EXCEPTION WHEN others THEN
            RAISE EXCEPTION 'medication dose-correction: corrects must be a valid uuid';
        END;
        -- `strike`, if present, is a JSON array over the closed group set.
        IF p ? 'strike' THEN
            IF jsonb_typeof(p -> 'strike') IS DISTINCT FROM 'array' THEN
                RAISE EXCEPTION 'medication dose-correction: strike must be a JSON array of group names';
            END IF;
            IF EXISTS (
                SELECT 1 FROM jsonb_array_elements_text(p -> 'strike') g
                WHERE g NOT IN ('dose', 'effective', 'reason')
            ) THEN
                RAISE EXCEPTION 'medication dose-correction: strike may only contain dose|effective|reason';
            END IF;
        END IF;
        -- A group cannot be both set and struck.
        IF (p ? 'dose'      AND COALESCE(p -> 'strike' ? 'dose', FALSE))
           OR (p ? 'effective' AND COALESCE(p -> 'strike' ? 'effective', FALSE))
           OR (p ? 'reason'    AND COALESCE(p -> 'strike' ? 'reason', FALSE)) THEN
            RAISE EXCEPTION 'medication dose-correction: a group cannot be both set and struck';
        END IF;
        -- A SET group must carry a value — keeps `strike` the one canonical way to unknown.
        IF p ? 'dose' AND NOT ((p -> 'dose' ->> 'amount') IS NOT NULL OR (p -> 'dose' ->> 'unit') IS NOT NULL) THEN
            RAISE EXCEPTION 'medication dose-correction: a set dose must carry amount and/or unit (use strike to set unknown)';
        END IF;
        IF p ? 'effective' AND (p -> 'effective' ->> 'value') IS NULL THEN
            RAISE EXCEPTION 'medication dose-correction: a set effective must carry a value (use strike to set unknown)';
        END IF;
        -- A set reason must be a STRING, not merely non-empty after ->> — a raw-SQL
        -- client could submit `"reason": {...}` or a number, and `->>` on a non-scalar
        -- jsonb value returns its non-empty stringified text, silently passing a
        -- length-only check. The dose-CHANGE branch above already gates on
        -- jsonb_typeof = 'string'; this closes the same gap on the correction branch
        -- (Task 3 review finding; principle 12 — the in-DB floor is the complete
        -- defense, not just the Rust path, which only ever offers a &str).
        IF p ? 'reason' AND (jsonb_typeof(p -> 'reason') IS DISTINCT FROM 'string'
                             OR length(btrim(p ->> 'reason')) = 0) THEN
            RAISE EXCEPTION 'medication dose-correction: a set reason must be a non-empty string (use strike to set unknown)';
        END IF;
        -- `note` and `info_source` are audit annotations, but the SAME non-scalar
        -- stringification trap the reason guard closes applies: `->>` on a jsonb
        -- object/array/number returns its non-empty stringified text, so a length-only
        -- check would let a raw-SQL client (bypassing the Rust builder, which only ever
        -- offers a &str) land e.g. `"note": {...}`'s JSON text verbatim in the column.
        -- Guarded here too so the in-DB floor is the COMPLETE defense (principle 12),
        -- uniform with reason. Both are honest-omit: absent is fine, present ⇒ non-empty
        -- string (omit rather than send an empty annotation).
        IF p ? 'note' AND (jsonb_typeof(p -> 'note') IS DISTINCT FROM 'string'
                           OR length(btrim(p ->> 'note')) = 0) THEN
            RAISE EXCEPTION 'medication dose-correction: note, when present, must be a non-empty string';
        END IF;
        IF p ? 'info_source' AND (jsonb_typeof(p -> 'info_source') IS DISTINCT FROM 'string'
                                  OR length(btrim(p ->> 'info_source')) = 0) THEN
            RAISE EXCEPTION 'medication dose-correction: info_source, when present, must be a non-empty string';
        END IF;
        -- Not a no-op: must set or strike at least one group.
        IF NOT (
            p ? 'dose' OR p ? 'effective' OR p ? 'reason'
            OR (p ? 'strike' AND jsonb_array_length(p -> 'strike') > 0)
        ) THEN
            RAISE EXCEPTION 'medication dose-correction: must set or strike at least one of dose|effective|reason (principle 4 floor)';
        END IF;
    END IF;
END;
$$;

-- 4. Fold a correction as a per-field patch overlay keyed by the TARGET dose event.
--    Each group's touched-flag = (set OR struck); its value = the set value, or NULL when
--    struck / untouched (the view uses the flag, never the raw NULL, to decide). HLC-wins
--    wholesale on a re-correction of the same point (ON CONFLICT). Offline-first: the
--    target need not exist locally.
--
-- #208/ADR-0057: this redefines ONLY the (event_log)-signature body db/032 first
-- defined — db/032 owns the DROP TRIGGER/DROP FUNCTION preamble, the REVOKE, and the
-- cairn_projection_apply registration row (same db/013-style redefinition convention:
-- the fn's FIRST definer owns that scaffolding, a later redefiner touches only the body).
CREATE OR REPLACE FUNCTION medication_dose_correction_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    -- ADR-0052: sealed rows carry ciphertext in body; the clear payload lives
    -- in event_clear (populated by the door BEFORE this row, same txn). NULL =
    -- sealed without custody here: nothing to project — honest degradation.
    p jsonb := cairn_clear_payload(e);
    v_dose_set     boolean := p ? 'dose';
    v_eff_set      boolean := p ? 'effective';
    v_reason_set   boolean := p ? 'reason';
    v_dose_struck   boolean := COALESCE(p -> 'strike' ? 'dose', FALSE);
    v_eff_struck    boolean := COALESCE(p -> 'strike' ? 'effective', FALSE);
    v_reason_struck boolean := COALESCE(p -> 'strike' ? 'reason', FALSE);
BEGIN
    IF p IS NULL THEN RETURN; END IF;
    -- #192 thread patient-consistency (shared guard, db/031) — same contract as the
    -- assert/cessation/dose-change verbs: fail loud locally, converge-and-flag on
    -- sync. Restored by #273: this redefinition had silently shadowed the guard call
    -- #192 added to db/032's body (file replay order), leaving the one verb whose
    -- corrected value drives current-dose winner selection (ADR-0050) unguarded.
    PERFORM cairn_guard_medication_patient(
        (p ->> 'medication_id')::uuid, e.patient_id, e.content_address);
    INSERT INTO medication_dose_correction
        (corrected_dose_event_id, medication_id, patient_id,
         amount, unit, effective_value, effective_precision, reason, note, info_source,
         dose_corrected, effective_corrected, reason_corrected,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        (p ->> 'corrects')::uuid, (p ->> 'medication_id')::uuid, e.patient_id,
        CASE WHEN v_dose_set THEN p -> 'dose' ->> 'amount' END,
        CASE WHEN v_dose_set THEN p -> 'dose' ->> 'unit'   END,
        CASE WHEN v_eff_set  THEN p -> 'effective' ->> 'value'     END,
        CASE WHEN v_eff_set  THEN p -> 'effective' ->> 'precision' END,
        CASE WHEN v_reason_set THEN p ->> 'reason' END,
        p ->> 'note',
        p ->> 'info_source',
        (v_dose_set OR v_dose_struck),
        (v_eff_set OR v_eff_struck),
        (v_reason_set OR v_reason_struck),
        e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    ON CONFLICT (corrected_dose_event_id) DO UPDATE SET
        medication_id       = EXCLUDED.medication_id,
        patient_id          = EXCLUDED.patient_id,
        amount              = EXCLUDED.amount,
        unit                = EXCLUDED.unit,
        effective_value     = EXCLUDED.effective_value,
        effective_precision = EXCLUDED.effective_precision,
        reason              = EXCLUDED.reason,
        note                = EXCLUDED.note,
        info_source         = EXCLUDED.info_source,
        dose_corrected      = EXCLUDED.dose_corrected,
        effective_corrected = EXCLUDED.effective_corrected,
        reason_corrected    = EXCLUDED.reason_corrected,
        hlc_wall            = EXCLUDED.hlc_wall,
        hlc_counter         = EXCLUDED.hlc_counter,
        origin              = EXCLUDED.origin,
        content_address     = EXCLUDED.content_address,
        updated_at          = clock_timestamp()
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        medication_dose_correction.hlc_wall, medication_dose_correction.hlc_counter,
        medication_dose_correction.origin, medication_dose_correction.content_address);
    RETURN;
END;
$$;
-- (db/032 already created the (event_log)-signature fn + its REVOKE + its
--  cairn_projection_apply registration row; no scaffolding is needed here — the
--  dispatcher picks up this replaced body via the same registered fn name.)
--
-- NOTE (history): this redefinition originally shipped (2026-07-15) WITHOUT the #192
-- patient-consistency guard — #192 landed a day later and patched db/032's body, not
-- knowing this file replaces that fn later in replay order, so the guard was silently
-- shadowed. Discovered during the #208 zero-drift conversion audit; restored by #273
-- (the PERFORM cairn_guard_medication_patient call in the body above).

-- 5. Rework the two dose-timeline views to read corrected effective/reason via the
--    touched-flags. SAME column sets as db/032 (no widening — replay-safe). The effective
--    SORT KEY uses the corrected value, so winner selection + trail order reflect the fix.
CREATE OR REPLACE VIEW medication_current_dose AS
SELECT DISTINCT ON (de.medication_id)
    de.medication_id, de.patient_id, de.dose_event_id,
    CASE WHEN corr.dose_corrected THEN corr.amount ELSE de.amount END AS amount,
    CASE WHEN corr.dose_corrected THEN corr.unit   ELSE de.unit   END AS unit,
    CASE WHEN corr.effective_corrected THEN corr.effective_value     ELSE de.effective_value     END AS effective_value,
    CASE WHEN corr.effective_corrected THEN corr.effective_precision ELSE de.effective_precision END AS effective_precision,
    (corr.corrected_dose_event_id IS NOT NULL) AS corrected
FROM medication_dose_event de
LEFT JOIN medication_dose_correction corr
    ON corr.corrected_dose_event_id = de.dose_event_id
   AND corr.medication_id = de.medication_id
ORDER BY de.medication_id,
         cairn_dose_effective_sort_key(
             CASE WHEN corr.effective_corrected THEN corr.effective_value ELSE de.effective_value END,
             de.hlc_wall) COLLATE "C" DESC,
         de.hlc_wall DESC, de.hlc_counter DESC, de.origin COLLATE "C" DESC, de.content_address DESC;
GRANT SELECT ON medication_current_dose TO cairn_agent;

CREATE OR REPLACE VIEW patient_medication_dose_history AS
SELECT de.medication_id, de.patient_id, de.dose_event_id, de.is_initial,
       CASE WHEN corr.dose_corrected THEN corr.amount ELSE de.amount END AS amount,
       CASE WHEN corr.dose_corrected THEN corr.unit   ELSE de.unit   END AS unit,
       CASE WHEN corr.effective_corrected THEN corr.effective_value     ELSE de.effective_value     END AS effective_value,
       CASE WHEN corr.effective_corrected THEN corr.effective_precision ELSE de.effective_precision END AS effective_precision,
       de.info_source,
       CASE WHEN corr.reason_corrected THEN corr.reason ELSE de.reason END AS reason,
       (corr.corrected_dose_event_id IS NOT NULL) AS corrected,
       to_timestamp(de.hlc_wall / 1000.0) AS recorded_at
FROM medication_dose_event de
LEFT JOIN medication_dose_correction corr
    ON corr.corrected_dose_event_id = de.dose_event_id
   AND corr.medication_id = de.medication_id
ORDER BY de.medication_id,
         cairn_dose_effective_sort_key(
             CASE WHEN corr.effective_corrected THEN corr.effective_value ELSE de.effective_value END,
             de.hlc_wall) COLLATE "C" ASC,
         de.hlc_wall ASC, de.hlc_counter ASC, de.origin COLLATE "C" ASC, de.content_address ASC;
GRANT SELECT ON patient_medication_dose_history TO cairn_agent;

-- 6. db/033 (medication reconciliation, already merged) reworked patient_medication_current/
--    patient_medication_past to route dose through db/033's OWN group-rollup views
--    (medication_group_current_dose / medication_group_last_dose), NOT step 5's single-thread
--    medication_current_dose. Left unfixed, those group views would keep the OLD whole-row
--    correction semantics (corr.corrected_dose_event_id IS NOT NULL) and the UNCORRECTED
--    effective sort key — silently defeating the headline behavior (a corrected effective
--    flips the winner) for every consumer that actually renders a patient's medication list.
--    SAME column sets as db/033 (replay-safe); only the CASE source + sort key change, same
--    pattern as step 5.
CREATE OR REPLACE VIEW medication_group_current_dose AS
SELECT DISTINCT ON (g.group_id)
    g.group_id, g.patient_id, de.dose_event_id,
    CASE WHEN corr.dose_corrected THEN corr.amount ELSE de.amount END AS amount,
    CASE WHEN corr.dose_corrected THEN corr.unit   ELSE de.unit   END AS unit,
    CASE WHEN corr.effective_corrected THEN corr.effective_value     ELSE de.effective_value     END AS effective_value,
    CASE WHEN corr.effective_corrected THEN corr.effective_precision ELSE de.effective_precision END AS effective_precision
FROM medication_dose_event de
JOIN medication_thread_group g ON g.medication_id = de.medication_id
LEFT JOIN medication_dose_correction corr
    ON corr.corrected_dose_event_id = de.dose_event_id AND corr.medication_id = de.medication_id
WHERE NOT EXISTS (SELECT 1 FROM medication_cessation cc WHERE cc.medication_id = de.medication_id)
ORDER BY g.group_id,
         cairn_dose_effective_sort_key(
             CASE WHEN corr.effective_corrected THEN corr.effective_value ELSE de.effective_value END,
             de.hlc_wall) COLLATE "C" DESC,
         de.hlc_wall DESC, de.hlc_counter DESC, de.origin COLLATE "C" DESC, de.content_address DESC;
GRANT SELECT ON medication_group_current_dose TO cairn_agent;

CREATE OR REPLACE VIEW medication_group_last_dose AS
SELECT DISTINCT ON (g.group_id)
    g.group_id, g.patient_id,
    CASE WHEN corr.dose_corrected THEN corr.amount ELSE de.amount END AS amount,
    CASE WHEN corr.dose_corrected THEN corr.unit   ELSE de.unit   END AS unit
FROM medication_dose_event de
JOIN medication_thread_group g ON g.medication_id = de.medication_id
LEFT JOIN medication_dose_correction corr
    ON corr.corrected_dose_event_id = de.dose_event_id AND corr.medication_id = de.medication_id
ORDER BY g.group_id,
         cairn_dose_effective_sort_key(
             CASE WHEN corr.effective_corrected THEN corr.effective_value ELSE de.effective_value END,
             de.hlc_wall) COLLATE "C" DESC,
         de.hlc_wall DESC, de.hlc_counter DESC, de.origin COLLATE "C" DESC, de.content_address DESC;
GRANT SELECT ON medication_group_last_dose TO cairn_agent;

-- 7. db/033's medication_group_status classifies a group active/ceased by comparing the
--    MAX effective across active members' dose points vs ceased members' cessations
--    (ADR-0047 ruling 3). Its active_eff CTE read de.effective_value directly (uncorrected) —
--    same class of bug as step 6: a correction that moves a dose point's effective date could
--    legitimately flip which side of the active/ceased boundary a group falls on, and the
--    uncorrected read would silently ignore that. SAME column set (group_id, patient_id,
--    status) — only the CTE's effective source changes.
CREATE OR REPLACE VIEW medication_group_status AS
WITH active_eff AS (
    SELECT g.group_id,
           MAX(cairn_dose_effective_sort_key(
               CASE WHEN corr.effective_corrected THEN corr.effective_value ELSE de.effective_value END,
               de.hlc_wall) COLLATE "C") AS eff
    FROM medication_dose_event de
    JOIN medication_thread_group g ON g.medication_id = de.medication_id
    LEFT JOIN medication_dose_correction corr
        ON corr.corrected_dose_event_id = de.dose_event_id AND corr.medication_id = de.medication_id
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

COMMIT;
