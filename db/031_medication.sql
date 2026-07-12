-- 031_medication.sql — the first clinical-content surface (§3.15/§3.16).
--
-- Two append-only verbs over an immortal medication_id thread:
--   clinical.medication.asserted            — patient takes/took a substance (mints the thread)
--   clinical.medication-cessation.asserted  — the thread is no longer taken (references it)
--
-- Safety floor (the only hard invariants): an assertion must carry a non-empty
-- substance.term and a non-empty info_source; both verbs must carry a valid
-- medication_id uuid. Everything else is honest-unknown (principle 4) — the floor
-- never blocks a medication write beyond these. Duplicates are ALLOWED (two
-- statements do exist); duplicate *detection* is the advisory projection's job.
--
-- event_log.body IS the payload (submit_event inserts body = b->'payload'); patient_id
-- is a top-level column. So the floor check (sees the full body b) reads b->'payload',
-- while the projection triggers (see NEW.body = the payload) read NEW.body directly.
BEGIN;

-- 1. Register both types in the fail-closed classification registry. Additive,
--    never targeting another author.
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.medication.asserted',           'additive', FALSE),
    ('clinical.medication-cessation.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. The structural floor for both verbs. RAISE EXCEPTION per violation.
CREATE OR REPLACE FUNCTION cairn_check_medication_assertion(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'medication assertion: missing payload';
    END IF;
    -- medication_id is the thread key on BOTH verbs.
    IF jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication assertion: medication_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (p ->> 'medication_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication assertion: medication_id must be a valid uuid';
    END;
    -- The start verb carries the clinical floor: a non-empty term + present info_source.
    IF p_type = 'clinical.medication.asserted' THEN
        IF jsonb_typeof(p -> 'substance' -> 'term') IS DISTINCT FROM 'string'
           OR length(btrim(p -> 'substance' ->> 'term')) = 0 THEN
            RAISE EXCEPTION 'medication assertion: substance.term must be a non-empty string (principle 4 floor)';
        END IF;
        IF jsonb_typeof(p -> 'info_source') IS DISTINCT FROM 'string'
           OR length(btrim(p ->> 'info_source')) = 0 THEN
            RAISE EXCEPTION 'medication assertion: info_source must be a non-empty string';
        END IF;
    END IF;
    -- The cessation verb carries only medication_id (+ optional stopped/reason) — done.
END;
$$;

-- 3. Extend the shared twin hook. This CREATE OR REPLACE PRESERVES every existing
--    branch (db/010 demographics, db/018 link, db/023 dispute, db/024 identity-state,
--    db/025 repudiate, db/015 honest-degrade fallback) verbatim from db/025's live
--    body, and adds ONLY the medication branch below — submit_event itself is never
--    re-declared. Required because this function is CREATE-OR-REPLACE and db/031
--    loads last: copying a stale body would silently drop the later floor branches.
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

-- 4. Projection table: one row per asserted thread. Overlay columns (hlc/origin/
--    content_address) let a replayed/duplicate assert converge deterministically.
CREATE TABLE IF NOT EXISTS medication_statement (
    medication_id     UUID PRIMARY KEY,
    patient_id        UUID NOT NULL,
    term              TEXT NOT NULL,
    inn_code          TEXT,
    formulation       TEXT,
    dose_amount       TEXT,
    dose_unit         TEXT,
    sig               TEXT,
    info_source       TEXT NOT NULL,
    started_value     TEXT,
    started_precision TEXT,
    hlc_wall          BIGINT NOT NULL,
    hlc_counter       INTEGER NOT NULL,
    origin            TEXT NOT NULL,
    content_address   BYTEA NOT NULL,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_statement TO cairn_agent;
CREATE INDEX IF NOT EXISTS medication_statement_patient_idx ON medication_statement (patient_id);

-- 5. Fold clinical.medication.asserted into medication_statement. NEW.body is the
--    payload; patient_id is a column. Overlay-winner keeps set-union convergence.
CREATE OR REPLACE FUNCTION medication_statement_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := NEW.body;
BEGIN
    INSERT INTO medication_statement
        (medication_id, patient_id, term, inn_code, formulation,
         dose_amount, dose_unit, sig, info_source, started_value, started_precision,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        (p ->> 'medication_id')::uuid, NEW.patient_id,
        p -> 'substance' ->> 'term',
        p -> 'substance' ->> 'inn_code',
        p -> 'substance' ->> 'formulation',
        p -> 'dose' ->> 'amount',
        p -> 'dose' ->> 'unit',
        p ->> 'sig',
        p ->> 'info_source',
        p -> 'started' ->> 'value',
        p -> 'started' ->> 'precision',
        NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (medication_id) DO UPDATE SET
        patient_id        = EXCLUDED.patient_id,
        term              = EXCLUDED.term,
        inn_code          = EXCLUDED.inn_code,
        formulation       = EXCLUDED.formulation,
        dose_amount       = EXCLUDED.dose_amount,
        dose_unit         = EXCLUDED.dose_unit,
        sig               = EXCLUDED.sig,
        info_source       = EXCLUDED.info_source,
        started_value     = EXCLUDED.started_value,
        started_precision = EXCLUDED.started_precision,
        hlc_wall          = EXCLUDED.hlc_wall,
        hlc_counter       = EXCLUDED.hlc_counter,
        origin            = EXCLUDED.origin,
        content_address   = EXCLUDED.content_address,
        updated_at        = clock_timestamp()
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        medication_statement.hlc_wall, medication_statement.hlc_counter,
        medication_statement.origin, medication_statement.content_address);
    RETURN NULL;  -- AFTER trigger
END;
$$;
DROP TRIGGER IF EXISTS medication_statement_apply_trg ON event_log;
CREATE TRIGGER medication_statement_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'clinical.medication.asserted')
    EXECUTE FUNCTION medication_statement_apply();

-- 7. Cessation projection. A SEPARATE table (not an UPDATE of medication_statement)
--    makes the fold arrival-order-independent: an orphan cessation (assert not yet
--    local) lands here and the join lights up as 'past' only once the assert arrives.
CREATE TABLE IF NOT EXISTS medication_cessation (
    medication_id     UUID PRIMARY KEY,
    patient_id        UUID NOT NULL,
    stopped_value     TEXT,
    stopped_precision TEXT,
    reason            TEXT,
    hlc_wall          BIGINT NOT NULL,
    hlc_counter       INTEGER NOT NULL,
    origin            TEXT NOT NULL,
    content_address   BYTEA NOT NULL,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_cessation TO cairn_agent;

CREATE OR REPLACE FUNCTION medication_cessation_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := NEW.body;
BEGIN
    INSERT INTO medication_cessation
        (medication_id, patient_id, stopped_value, stopped_precision, reason,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        (p ->> 'medication_id')::uuid, NEW.patient_id,
        p -> 'stopped' ->> 'value',
        p -> 'stopped' ->> 'precision',
        p ->> 'reason',
        NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (medication_id) DO UPDATE SET
        patient_id        = EXCLUDED.patient_id,
        stopped_value     = EXCLUDED.stopped_value,
        stopped_precision = EXCLUDED.stopped_precision,
        reason            = EXCLUDED.reason,
        hlc_wall          = EXCLUDED.hlc_wall,
        hlc_counter       = EXCLUDED.hlc_counter,
        origin            = EXCLUDED.origin,
        content_address   = EXCLUDED.content_address,
        updated_at        = clock_timestamp()
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        medication_cessation.hlc_wall, medication_cessation.hlc_counter,
        medication_cessation.origin, medication_cessation.content_address);
    RETURN NULL;
END;
$$;
DROP TRIGGER IF EXISTS medication_cessation_apply_trg ON event_log;
CREATE TRIGGER medication_cessation_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'clinical.medication-cessation.asserted')
    EXECUTE FUNCTION medication_cessation_apply();

-- 8. Unified list: statement LEFT JOIN cessation → status derived. An orphan
--    cessation (no matching statement) yields NO row here (nothing to render);
--    when the statement arrives, ceased flips true. Combines each statement
--    with its cessation (if any) into one list; every asserted thread appears
--    regardless of who asserted it.
--
--    `asserted_at` is derived from the assert event's HLC wall component
--    (`hlc_wall`, t_recorded in ms — db/001), NOT the local `updated_at`. This is
--    the *convergent* recording time: the same on every node that holds the event,
--    so the staleness signal (§3.15/§9-B — a med asserted years ago shows its age)
--    is honest even on a node that only just replicated an old assert. `updated_at`
--    is a local-clock fold marker (reset on every overlay apply) and would make a
--    freshly-synced old med look new and diverge between nodes — wrong for display.
CREATE OR REPLACE VIEW patient_medication AS
SELECT s.medication_id, s.patient_id, s.term, s.inn_code, s.formulation,
       s.dose_amount, s.dose_unit, s.sig, s.info_source,
       s.started_value, s.started_precision,
       to_timestamp(s.hlc_wall / 1000.0) AS asserted_at,
       (c.medication_id IS NOT NULL) AS ceased,
       c.stopped_value, c.stopped_precision, c.reason
FROM medication_statement s
LEFT JOIN medication_cessation c USING (medication_id);
GRANT SELECT ON patient_medication TO cairn_agent;

CREATE OR REPLACE VIEW patient_medication_current AS
SELECT medication_id, patient_id, term, inn_code, formulation,
       dose_amount, dose_unit, sig, info_source, started_value, started_precision, asserted_at
FROM patient_medication WHERE NOT ceased;
GRANT SELECT ON patient_medication_current TO cairn_agent;

CREATE OR REPLACE VIEW patient_medication_past AS
SELECT medication_id, patient_id, term, inn_code, formulation,
       dose_amount, dose_unit, sig, info_source, started_value, started_precision,
       asserted_at, stopped_value, stopped_precision, reason
FROM patient_medication WHERE ceased;
GRANT SELECT ON patient_medication_past TO cairn_agent;

-- 9. E1 reconciliation flag (advisory, never auto-merges). >=2 ACTIVE threads for
--    one patient sharing coalesce(inn_code, normalized term). Deterministic — no
--    fuzzy matching (brand<->generic/typos are deferred to the Tier-A drug matcher).
--    COLLATE "C" pins the normalized-term key for cross-node determinism (ADR-0045).
--    Resolution is ceasing the redundant thread (no new event type).
--    Known blind spot (deferred, not a bug): the key prefers inn_code when present,
--    so the SAME substance asserted once coded and once uncoded lands under two
--    different keys and is NOT flagged. Cross-coding-state matching waits on the
--    Tier-A dictionary, same as brand<->generic.
CREATE OR REPLACE VIEW patient_medication_reconciliation_flag AS
SELECT patient_id,
       coalesce(inn_code, lower(btrim(term) COLLATE "C")) AS dup_key,
       count(*)                                           AS thread_count,
       array_agg(medication_id ORDER BY medication_id)    AS medication_ids
FROM patient_medication_current
GROUP BY patient_id, coalesce(inn_code, lower(btrim(term) COLLATE "C"))
HAVING count(*) > 1;
GRANT SELECT ON patient_medication_reconciliation_flag TO cairn_agent;

COMMIT;
