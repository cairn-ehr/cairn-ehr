-- 034_medication_attestation.sql — slice 4 of the clinical.medication surface.
--
-- One additive verb: clinical.medication-attestation.asserted. A human takes
-- clinical responsibility (principle 10, ADR-0007) for one medication_id thread,
-- pinning a convergent commitment of the thread's content-event SET it reviewed.
-- Responsibility is enforced entirely by the db/005 attestation gate (the payload
-- carries a responsibility-bearing contributor -> the 3-arg door demands a valid
-- human token). This migration is purely structural floor + a set-commitment helper
-- + an overlay/projection (part 2). db/031, db/032, db/033 are UNTOUCHED, and the
-- current-list views are NOT widened (replay rule). See ADR-0049.
BEGIN;

-- 1. Register the verb (fail-closed registry, ADR-0010). Additive: an attestation
--    adds accountability and forecloses on nothing, so ADR-0043's owner-gate does
--    NOT apply and a clinician may vouch for a thread another author recorded.
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.medication-attestation.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. Structural floor. Culture-neutral, OFFLINE-FIRST (no check the thread exists
--    locally — set-union sync may deliver the attestation before the thread). The
--    twin requirement is enforced by the db/005 dispatcher via twin_required_msg
--    (step 3), NOT here. Mirrors cairn_check_medication_reconciliation.
CREATE OR REPLACE FUNCTION cairn_check_medication_attestation(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'medication attestation: missing payload';
    END IF;
    IF jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication attestation: medication_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (p ->> 'medication_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication attestation: medication_id must be a valid uuid';
    END;
    IF jsonb_typeof(b -> 'patient_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication attestation: patient_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (b ->> 'patient_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication attestation: patient_id must be a valid uuid';
    END;
    -- reviewed_commitment: a non-empty hex string (the pinned set commitment).
    IF jsonb_typeof(p -> 'reviewed_commitment') IS DISTINCT FROM 'string'
       OR (p ->> 'reviewed_commitment') !~ '^[0-9a-fA-F]+$' THEN
        RAISE EXCEPTION 'medication attestation: reviewed_commitment must be a non-empty hex string';
    END IF;
    -- reviewed_count: a non-negative integer legibility hint.
    IF jsonb_typeof(p -> 'reviewed_count') IS DISTINCT FROM 'number'
       OR (p ->> 'reviewed_count')::numeric < 0
       OR (p ->> 'reviewed_count')::numeric <> floor((p ->> 'reviewed_count')::numeric) THEN
        RAISE EXCEPTION 'medication attestation: reviewed_count must be a non-negative integer';
    END IF;
END;
$$;

-- 3. Register the verb's floor + HARD twin requirement in the #173/ADR-0048 registry
--    (the single db/005 dispatcher reads these rows). Placed AFTER the check fn above
--    so the fail-closed registry trigger sees cairn_check_medication_attestation(text,
--    jsonb) declared at load time (an implementer catch from #173: registry INSERT must
--    follow the CREATE, or a fresh load rolls back).
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.medication-attestation.asserted', 'cairn_check_medication_attestation',
     'medication attestation requires a non-empty authored twin (§3.13/§3.15)')
ON CONFLICT (event_type) DO NOTHING;

-- 4. The set-commitment SINGLE SOURCE. Sorted-concat-hash of the thread's content-event
--    content_addresses (byte order -> order-independent, collation-free; mirrors
--    event_set_commitment in medium.rs). Called at BOTH author time (the orchestrator
--    pins this value) and read time (the staleness view recomputes it) -> byte-identity
--    guaranteed, no Rust<->SQL drift. NULL when the thread has no local content events
--    (orphan): the orchestrator bails, the projection reads NULL -> stale. Content
--    events EXCLUDE reconciliation/separation/attestation (not thread content).
CREATE OR REPLACE FUNCTION cairn_medication_thread_commitment(p_medication_id uuid)
RETURNS bytea LANGUAGE sql STABLE AS $$
    SELECT CASE WHEN count(*) = 0 THEN NULL
                ELSE digest(string_agg(content_address, ''::bytea ORDER BY content_address), 'sha256')
           END
    FROM event_log
    WHERE event_type IN (
            'clinical.medication.asserted',
            'clinical.medication-cessation.asserted',
            'clinical.medication-dose-change.asserted',
            'clinical.medication-dose-correction.asserted')
      AND (body ->> 'medication_id')::uuid = p_medication_id;
$$;

COMMIT;
