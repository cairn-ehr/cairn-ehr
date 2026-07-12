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
