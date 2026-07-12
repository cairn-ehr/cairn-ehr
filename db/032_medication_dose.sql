-- 032_medication_dose.sql — slice 2 of the clinical.medication surface (§3.15/§3.16).
--
-- Two append-only verbs over the existing medication_id thread:
--   clinical.medication-dose-change.asserted     — the dose changed (additive; both
--                                                   doses true over effective time)
--   clinical.medication-dose-correction.asserted — a recorded dose was wrong; carries
--                                                   `corrects` = the dose event it fixes
--
-- Floor: structural only (principle 4 — never block a dose write beyond integrity).
-- Projection (added below): a dose timeline (point 0 seeded from the assert + one
-- point per change) with corrections overlaying a targeted point; current dose is the
-- latest-EFFECTIVE point (bitemporal §5.1). db/031 is UNTOUCHED — all slice-2 SQL is here.
BEGIN;

-- 1. Register both verbs. Additive, never targeting another author: a correction does
--    NOT foreclose the corrected event (kept verbatim in event_log + flagged in the
--    history); it only wins the current-dose projection. So ADR-0043's suppression
--    owner-gate does not apply and cross-author dose correction is allowed (as with a
--    corrected DOB in demographics).
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.medication-dose-change.asserted',     'additive', FALSE),
    ('clinical.medication-dose-correction.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. Structural floor for both verbs. RAISE EXCEPTION per violation.
CREATE OR REPLACE FUNCTION cairn_check_medication_dose(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'medication dose: missing payload';
    END IF;
    -- medication_id is the thread key on BOTH verbs.
    IF jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication dose: medication_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (p ->> 'medication_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication dose: medication_id must be a valid uuid';
    END;

    IF p_type = 'clinical.medication-dose-change.asserted' THEN
        -- A change is a new clinical claim: it carries its provenance.
        IF jsonb_typeof(p -> 'info_source') IS DISTINCT FROM 'string'
           OR length(btrim(p ->> 'info_source')) = 0 THEN
            RAISE EXCEPTION 'medication dose-change: info_source must be a non-empty string';
        END IF;
        -- Not a pure no-op: it must state a dose, an effective date, or a reason.
        -- This is a CONTENT check, not a key-presence check: a raw-SQL client can
        -- submit a present-but-empty `"dose":{}` (or `"effective":{}`), which would
        -- satisfy `p ? 'dose'` while carrying nothing — the no-op floor must not be
        -- bypassable that way. The first three disjuncts are 3VL-safe by
        -- construction: `->> ... IS NOT NULL` always yields a definite TRUE/FALSE,
        -- never NULL, regardless of whether the key or its parent object exists.
        -- The `reason` disjunct is NOT of that shape (it's a bare
        -- `typeof(...) = 'string' AND length(...) > 0`, which itself evaluates to
        -- SQL NULL when 'reason' is absent) — so it is wrapped in
        -- COALESCE(..., FALSE), same as pre-fix, to keep it solid. Without that
        -- COALESCE, "FALSE OR FALSE OR FALSE OR NULL" is NULL (not FALSE) under
        -- three-valued logic, and PL/pgSQL's `IF NULL THEN` silently skips —
        -- exactly the bug this guard exists to close.
        IF NOT (
            (p -> 'dose' ->> 'amount') IS NOT NULL OR (p -> 'dose' ->> 'unit') IS NOT NULL
            OR (p -> 'effective' ->> 'value') IS NOT NULL
            OR COALESCE(jsonb_typeof(p -> 'reason') = 'string' AND length(btrim(p ->> 'reason')) > 0, FALSE)
        ) THEN
            RAISE EXCEPTION 'medication dose-change: must carry a dose, an effective date, or a reason (principle 4 floor)';
        END IF;
    ELSIF p_type = 'clinical.medication-dose-correction.asserted' THEN
        -- `corrects` names the dose event being fixed. Existence is NOT required —
        -- offline-first: the target may replicate after the correction.
        IF jsonb_typeof(p -> 'corrects') IS DISTINCT FROM 'string' THEN
            RAISE EXCEPTION 'medication dose-correction: corrects must be a uuid string';
        END IF;
        BEGIN
            PERFORM (p ->> 'corrects')::uuid;
        EXCEPTION WHEN others THEN
            RAISE EXCEPTION 'medication dose-correction: corrects must be a valid uuid';
        END;
        -- dose optional (correct-to-unknown); info_source optional (a record fix).
    END IF;
END;
$$;

-- 3. Extend the shared twin hook. PRESERVES every existing branch from db/025+db/031
--    verbatim and adds ONLY the two dose branches. submit_event itself is never
--    re-declared.
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

-- 4. Deterministic effective sort key: the ISO-ish effective string sorts
--    chronologically as bytes; a NULL effective falls back to the recording time
--    (hlc_wall → ISO string), an honest lower bound. Format mask is numeric-only so
--    it is locale-independent and identical on every node (§5.1). STABLE (to_char).
CREATE OR REPLACE FUNCTION cairn_dose_effective_sort_key(p_effective text, p_hlc_wall bigint)
RETURNS text LANGUAGE sql STABLE AS $$
    SELECT COALESCE(
        p_effective,
        to_char(to_timestamp(p_hlc_wall / 1000.0) AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS'));
$$;

-- 5. One row per dose POINT: point 0 (seeded from the assert) + one per change. PK =
--    the event's own event_id (immutable content), so a replayed event is idempotent.
CREATE TABLE IF NOT EXISTS medication_dose_event (
    dose_event_id       UUID PRIMARY KEY,
    medication_id       UUID NOT NULL,
    patient_id          UUID NOT NULL,
    amount               TEXT,
    unit                TEXT,
    effective_value     TEXT,
    effective_precision TEXT,
    is_initial          BOOLEAN NOT NULL,
    info_source         TEXT,
    reason               TEXT,
    hlc_wall            BIGINT NOT NULL,
    hlc_counter         INTEGER NOT NULL,
    origin               TEXT NOT NULL,
    content_address      BYTEA NOT NULL,
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_dose_event TO cairn_agent;
CREATE INDEX IF NOT EXISTS medication_dose_event_med_idx ON medication_dose_event (medication_id);

-- 6. Corrections, keyed by the TARGET dose event they fix (a correction overlays a
--    specific point). HLC-wins if one point is corrected twice; converges if the
--    correction arrives before its target (orphan). Correction TABLE only here; its
--    trigger is added in the correction task.
CREATE TABLE IF NOT EXISTS medication_dose_correction (
    corrected_dose_event_id UUID PRIMARY KEY,
    medication_id           UUID NOT NULL,
    patient_id              UUID NOT NULL,
    amount                  TEXT,
    unit                    TEXT,
    reason                  TEXT,
    info_source             TEXT,
    hlc_wall                BIGINT NOT NULL,
    hlc_counter             INTEGER NOT NULL,
    origin                  TEXT NOT NULL,
    content_address         BYTEA NOT NULL,
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_dose_correction TO cairn_agent;

-- 7. Seed point 0 from the assert (a SECOND, additive trigger on the assert type; the
--    slice-1 statement/cessation triggers are untouched). dose_event_id = the assert's
--    event_id; effective = the assert's `started`.
CREATE OR REPLACE FUNCTION medication_dose_seed_initial()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE p jsonb := NEW.body;
BEGIN
    INSERT INTO medication_dose_event
        (dose_event_id, medication_id, patient_id, amount, unit,
         effective_value, effective_precision, is_initial, info_source, reason,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        NEW.event_id, (p ->> 'medication_id')::uuid, NEW.patient_id,
        p -> 'dose' ->> 'amount', p -> 'dose' ->> 'unit',
        p -> 'started' ->> 'value', p -> 'started' ->> 'precision',
        TRUE, p ->> 'info_source', NULL,
        NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (dose_event_id) DO NOTHING;
    RETURN NULL;
END;
$$;
DROP TRIGGER IF EXISTS medication_dose_seed_initial_trg ON event_log;
CREATE TRIGGER medication_dose_seed_initial_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'clinical.medication.asserted')
    EXECUTE FUNCTION medication_dose_seed_initial();

-- 8. Fold a dose change into a new timeline point.
CREATE OR REPLACE FUNCTION medication_dose_change_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE p jsonb := NEW.body;
BEGIN
    INSERT INTO medication_dose_event
        (dose_event_id, medication_id, patient_id, amount, unit,
         effective_value, effective_precision, is_initial, info_source, reason,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        NEW.event_id, (p ->> 'medication_id')::uuid, NEW.patient_id,
        p -> 'dose' ->> 'amount', p -> 'dose' ->> 'unit',
        p -> 'effective' ->> 'value', p -> 'effective' ->> 'precision',
        FALSE, p ->> 'info_source', p ->> 'reason',
        NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (dose_event_id) DO NOTHING;
    RETURN NULL;
END;
$$;
DROP TRIGGER IF EXISTS medication_dose_change_apply_trg ON event_log;
CREATE TRIGGER medication_dose_change_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'clinical.medication-dose-change.asserted')
    EXECUTE FUNCTION medication_dose_change_apply();

-- 9. Effective per-point value = the correction's value IF a correction row exists,
--    ELSE the event's value. Keyed on PRESENCE (not COALESCE) so a correct-to-unknown
--    (correction row with NULL amount) shows unknown, not the stale original.
CREATE OR REPLACE VIEW medication_current_dose AS
SELECT DISTINCT ON (de.medication_id)
    de.medication_id, de.patient_id, de.dose_event_id,
    CASE WHEN corr.corrected_dose_event_id IS NOT NULL THEN corr.amount ELSE de.amount END AS amount,
    CASE WHEN corr.corrected_dose_event_id IS NOT NULL THEN corr.unit   ELSE de.unit   END AS unit,
    de.effective_value, de.effective_precision,
    (corr.corrected_dose_event_id IS NOT NULL) AS corrected
FROM medication_dose_event de
LEFT JOIN medication_dose_correction corr ON corr.corrected_dose_event_id = de.dose_event_id
ORDER BY de.medication_id,
         cairn_dose_effective_sort_key(de.effective_value, de.hlc_wall) COLLATE "C" DESC,
         de.hlc_wall DESC, de.hlc_counter DESC, de.origin COLLATE "C" DESC, de.content_address DESC;
GRANT SELECT ON medication_current_dose TO cairn_agent;

-- 10. The full titration trail, chronological by effective time. Exposes dose_event_id
--     (so a correction can target a point) and the corrected flag.
--     NOTE: the ORDER BY below is a DISPLAY CONVENIENCE ONLY. SQL does not guarantee a
--     view's internal ordering survives when the view is wrapped in an outer query (e.g.
--     filtered by a WHERE clause or joined). Any consumer that requires a guaranteed
--     chronological trail MUST add its own outer ORDER BY (e.g.
--     ORDER BY recorded_at, dose_event_id).
CREATE OR REPLACE VIEW patient_medication_dose_history AS
SELECT de.medication_id, de.patient_id, de.dose_event_id, de.is_initial,
       CASE WHEN corr.corrected_dose_event_id IS NOT NULL THEN corr.amount ELSE de.amount END AS amount,
       CASE WHEN corr.corrected_dose_event_id IS NOT NULL THEN corr.unit   ELSE de.unit   END AS unit,
       de.effective_value, de.effective_precision, de.info_source, de.reason,
       (corr.corrected_dose_event_id IS NOT NULL) AS corrected,
       to_timestamp(de.hlc_wall / 1000.0) AS recorded_at
FROM medication_dose_event de
LEFT JOIN medication_dose_correction corr ON corr.corrected_dose_event_id = de.dose_event_id
ORDER BY de.medication_id,
         cairn_dose_effective_sort_key(de.effective_value, de.hlc_wall) COLLATE "C" ASC,
         de.hlc_wall ASC, de.hlc_counter ASC, de.origin COLLATE "C" ASC, de.content_address ASC;
GRANT SELECT ON patient_medication_dose_history TO cairn_agent;

-- 11. Rework the current/past views to source the dose from the timeline winner.
--     CRITICAL: keep the EXACT SAME COLUMN SET as db/031 (do NOT append columns).
--     connect_and_load_schema REPLAYS db/031 on every connect, so if db/032 WIDENED these
--     views, db/031's narrower CREATE OR REPLACE would then fail on the next connect with
--     "cannot drop columns from view". Changing only the dose SOURCE (same names/types/order)
--     is a legal CREATE OR REPLACE both directions, so replay is safe. dose_event_id /
--     corrected are exposed via the separate medication_current_dose view (created above),
--     not here. A thread with NO timeline row (a pre-slice-2 assert) falls back to the
--     as-asserted statement dose (CASE on cd presence — self-healing, no data migration).
CREATE OR REPLACE VIEW patient_medication_current AS
SELECT pm.medication_id, pm.patient_id, pm.term, pm.inn_code, pm.formulation,
       CASE WHEN cd.medication_id IS NOT NULL THEN cd.amount ELSE pm.dose_amount END AS dose_amount,
       CASE WHEN cd.medication_id IS NOT NULL THEN cd.unit   ELSE pm.dose_unit   END AS dose_unit,
       pm.sig, pm.info_source, pm.started_value, pm.started_precision, pm.asserted_at
FROM patient_medication pm
LEFT JOIN medication_current_dose cd USING (medication_id)
WHERE NOT pm.ceased;
GRANT SELECT ON patient_medication_current TO cairn_agent;

CREATE OR REPLACE VIEW patient_medication_past AS
SELECT pm.medication_id, pm.patient_id, pm.term, pm.inn_code, pm.formulation,
       CASE WHEN cd.medication_id IS NOT NULL THEN cd.amount ELSE pm.dose_amount END AS dose_amount,
       CASE WHEN cd.medication_id IS NOT NULL THEN cd.unit   ELSE pm.dose_unit   END AS dose_unit,
       pm.sig, pm.info_source, pm.started_value, pm.started_precision,
       pm.asserted_at, pm.stopped_value, pm.stopped_precision, pm.reason
FROM patient_medication pm
LEFT JOIN medication_current_dose cd USING (medication_id)
WHERE pm.ceased;
GRANT SELECT ON patient_medication_past TO cairn_agent;

COMMIT;
