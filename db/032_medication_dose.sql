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

COMMIT;
