-- 079_medication_versions.sql — batch charting slice 2a/2b: "From:", the "why", and (later in this
-- file) the set-aside version of a medication line. Canonical home: data-model §3.3.
--
-- §1 (this section) is the FLOOR for two optional free-text keys on a
-- `clinical.medication.asserted` payload (the pure builder is crates/cairn-event/src/medication/
-- source_note.rs):
--   source_detail  where the claim came from, finer than `info_source` ("GP letter", "daughter,
--                  from the packet")
--   why            why the clinician charted the line this way
-- They are TWO keys because a provenance and a rationale are different facts (see source_note.rs).
-- The projection columns live on `medication_charting` (db/077 §2).
--
-- HOW IT FITS. db/031's cairn_check_medication_assertion PERFORMs the function below. plpgsql
-- resolves the callee at run time, so db/031 may call a function this later file defines; and a
-- floor change lives in a NEW file rather than an edit to db/031/077 so it ships as its own
-- schema generation (#188, as db/077's header explains).
--
-- THE TWO TIERS (ADR-0056, the db/075 idiom). STRUCTURAL — a present key must be a JSON string or
-- JSON null (the projection reads it with `->>` and must never be handed an object) — refuses at
-- BOTH doors. An explicit JSON null reads as ABSENT at both doors, as db/077 treats every optional
-- key (a peer that emits nulls for absent optionals is not refused; `->>` reads it as SQL NULL).
-- Everything else — non-blank, no control character — refuses at the AUTHORING door only
-- (`cairn.remote_apply` <> 'on'): a peer's signed words must never wedge sync.
BEGIN;

CREATE OR REPLACE FUNCTION cairn_check_medication_source_note(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
    k text;
BEGIN
    FOREACH k IN ARRAY ARRAY['source_detail', 'why'] LOOP
        -- ---- Structural: both doors. Absent and JSON null are fine (null = absent). ----
        IF p ? k AND jsonb_typeof(p -> k) NOT IN ('string', 'null') THEN
            RAISE EXCEPTION 'medication assertion: % must be a string', k;
        END IF;
    END LOOP;

    -- ---- Authoring door only (ADR-0056): the one line below is the only thing that
    --      distinguishes the two doors. ----
    IF current_setting('cairn.remote_apply', true) = 'on' THEN
        RETURN;
    END IF;
    FOREACH k IN ARRAY ARRAY['source_detail', 'why'] LOOP
        -- A JSON null is absent: skipped explicitly, not by relying on btrim(NULL).
        IF p ? k AND jsonb_typeof(p -> k) = 'string' THEN
            IF btrim(p ->> k) = '' THEN
                RAISE EXCEPTION 'medication assertion: % must not be blank (omit the key instead)', k;
            END IF;
            -- Newlines and tabs included: these are one-line labels in a list row.
            IF (p ->> k) ~ '[[:cntrl:]]' THEN
                RAISE EXCEPTION 'medication assertion: % must not contain control characters', k;
            END IF;
        END IF;
    END LOOP;
END;
$$;
-- The cairn_check_* family is revoked uniformly (#382; db/005).
REVOKE EXECUTE ON FUNCTION cairn_check_medication_source_note(jsonb) FROM PUBLIC;

COMMIT;

-- ===========================================================================================
-- §2 — the SET-ASIDE VERSION (`clinical.medication-set-aside.asserted`, schema
-- `clinical.medication-set-aside/1`). Builder: crates/cairn-event/src/medication/set_aside.rs.
--
-- WHAT IT IS. A version of a drug that a source reported (a GP letter's old dose, a daughter's
-- recollection) and the clinician did NOT believe. It is signed into the record as what the
-- source said — never erased (principle 2: never erase, always overlay) — but it is NEVER a
-- current medication: it is not a thread (no `medication_id`), it is written to its own table,
-- and none of the readers of current lines (medication_statement, patient_medication*) ever see
-- it. It hangs off a real thread by `alternative_to`, which a reader joins on.
--
-- THE TWO TIERS (ADR-0056). STRUCTURAL rules — the shape of every key — refuse at BOTH doors.
-- The REFERENCE rules — "the companion thread is on this chart and still current", "the version
-- id is new", the schema tag, no §5.9 claim — refuse at the AUTHORING door only: a peer may send a
-- version before its companion has synced (an ORPHAN, held and shown nowhere, because every reader
-- joins on alternative_to), and a peer's signed words must never wedge sync. A companion that
-- turns out to be on ANOTHER chart is not refused on remote apply but FLAGGED, exactly as a
-- cessation's is (cairn_guard_medication_patient, db/031).
--
-- WHY charted_as / warning_override ARE REFUSED. A set-aside version was never charted and never
-- overrode a warning, so no writer produces those keys; refusing them at both doors wedges no
-- peer, and it stops a version posing as a charted line.
--
-- THE §5.9 CLAIM. There is no per-type safety rule in db/049: the signal is a field on the
-- envelope that db/020 writes into event_log.safety for EVERY type, and cairn_patient_safety
-- reports any non-null one. So this floor refuses a non-null `safety` at BOTH doors (no honest
-- writer emits one; the builder carries none).
-- ===========================================================================================
BEGIN;

INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.medication-set-aside.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

CREATE OR REPLACE FUNCTION cairn_check_medication_set_aside(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
    k text;
    v_alt uuid;
BEGIN
    -- ---- Structural: both doors ----
    IF jsonb_typeof(p) IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'medication set-aside: missing payload';
    END IF;
    FOREACH k IN ARRAY ARRAY['version_id', 'alternative_to'] LOOP
        IF jsonb_typeof(p -> k) IS DISTINCT FROM 'string'
           OR NOT pg_input_is_valid(p ->> k, 'uuid') THEN
            RAISE EXCEPTION 'medication set-aside: % must be a uuid string', k;
        END IF;
    END LOOP;
    IF jsonb_typeof(p #> '{substance,term}') IS DISTINCT FROM 'string'
       OR NOT cairn_twin_is_present(p #>> '{substance,term}') THEN
        RAISE EXCEPTION 'medication set-aside: substance.term must be a non-blank string';
    END IF;
    IF jsonb_typeof(p -> 'info_source') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication set-aside: info_source must be a string';
    END IF;
    -- Never charted, never overrode a warning: refused even as a JSON null.
    FOREACH k IN ARRAY ARRAY['charted_as', 'warning_override'] LOOP
        IF p ? k THEN
            RAISE EXCEPTION 'medication set-aside: % must be absent (a set-aside version was never charted)', k;
        END IF;
    END LOOP;
    -- The drug's coding and the From:/why notes are judged by the same functions as a charted line.
    PERFORM cairn_check_medication_coding(p);
    PERFORM cairn_check_medication_source_note(b);
    -- route / substance.strength / components: the projection reads them with `->>`, so a peer's
    -- object must be refused here. With charted_as absent (refused above) its closed-vocabulary
    -- arm never fires; it also handles both doors itself.
    PERFORM cairn_check_medication_charting(p);
    -- No §5.9 claim, at BOTH doors: db/020 writes b->'safety' into event_log.safety for every
    -- type and cairn_patient_safety reports any non-null one, so a peer's claim on a set-aside
    -- version would become a live safety signal on the chart. No honest writer emits one, so
    -- refusing it pens nothing legitimate. A JSON null reads as absent.
    IF jsonb_typeof(b -> 'safety') IS NOT NULL AND jsonb_typeof(b -> 'safety') <> 'null' THEN
        RAISE EXCEPTION 'medication set-aside: a set-aside version carries no safety claim';
    END IF;

    -- ---- References: the authoring door only (ADR-0056) ----
    IF current_setting('cairn.remote_apply', true) = 'on' THEN
        RETURN;
    END IF;
    IF b ->> 'schema_version' IS DISTINCT FROM 'clinical.medication-set-aside/1' THEN
        RAISE EXCEPTION 'medication set-aside: schema_version must be clinical.medication-set-aside/1, got %', b ->> 'schema_version';
    END IF;
    v_alt := (p ->> 'alternative_to')::uuid;
    -- The companion must be a thread on THIS chart (the body's patient_id; the db/075 idiom). A
    -- companion submitted earlier in the same transaction is already projected, so it counts.
    IF cairn_medication_thread_patient(v_alt) IS DISTINCT FROM (b ->> 'patient_id')::uuid THEN
        RAISE EXCEPTION 'medication set-aside: alternative_to % is not a medication on this chart', v_alt;
    END IF;
    -- ...and still current: a stopped line is not one a clinician is choosing between versions of.
    IF NOT EXISTS (SELECT 1 FROM medication_statement WHERE medication_id = v_alt)
       OR EXISTS (SELECT 1 FROM medication_cessation WHERE medication_id = v_alt) THEN
        RAISE EXCEPTION 'medication set-aside: alternative_to % is not a current medication', v_alt;
    END IF;
    IF EXISTS (SELECT 1 FROM medication_set_aside WHERE version_id = (p ->> 'version_id')::uuid) THEN
        RAISE EXCEPTION 'medication set-aside: version_id % already exists', p ->> 'version_id';
    END IF;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_medication_set_aside(text, jsonb) FROM PUBLIC;

-- Register the floor + the hard twin requirement (#173). DO UPDATE converges on replay (#214).
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.medication-set-aside.asserted', 'cairn_check_medication_set_aside',
     'medication set-aside requires a non-empty authored twin (§3.13/§3.3)')
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (cairn_event_twin_check.check_fn, cairn_event_twin_check.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);

-- The projection: one row per version event, keyed by the event's content address (what a shred
-- scrubs by, db/037). Columns are final: a widened CREATE TABLE never reaches an existing table.
CREATE TABLE IF NOT EXISTS medication_set_aside (
    content_address BYTEA PRIMARY KEY,
    event_id        UUID NOT NULL,
    patient_id      UUID NOT NULL,
    version_id      UUID NOT NULL,
    alternative_to  UUID NOT NULL,
    term            TEXT NOT NULL,
    coding_system   TEXT,
    coding_code     TEXT,
    coding_display  TEXT,
    formulation     TEXT,
    dose_amount     TEXT,
    dose_unit       TEXT,
    sig             TEXT,
    route           TEXT,
    strength        TEXT,
    info_source     TEXT NOT NULL,
    source_detail   TEXT,
    why             TEXT,
    t_recorded      TIMESTAMPTZ NOT NULL
);
GRANT SELECT ON medication_set_aside TO cairn_agent;
CREATE INDEX IF NOT EXISTS medication_set_aside_patient_idx ON medication_set_aside (patient_id);
CREATE INDEX IF NOT EXISTS medication_set_aside_alt_idx ON medication_set_aside (alternative_to);

-- The applier reads the CLEAR payload (ADR-0052): NULL = no custody here, nothing to project (the
-- late-custody path re-runs it). A body it cannot represent is SKIPPED, never raised — a sealed
-- peer body admitted without custody never met the structural floor, and a raise would fail the
-- late-custody heal. The guard FLAGS a companion on another chart on remote apply (it raises only
-- at the local door, where the floor has already refused it).
CREATE OR REPLACE FUNCTION medication_set_aside_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := cairn_clear_payload(e);
BEGIN
    IF p IS NULL
       OR jsonb_typeof(p -> 'version_id') IS DISTINCT FROM 'string'
       OR NOT pg_input_is_valid(p ->> 'version_id', 'uuid')
       OR jsonb_typeof(p -> 'alternative_to') IS DISTINCT FROM 'string'
       OR NOT pg_input_is_valid(p ->> 'alternative_to', 'uuid')
       OR jsonb_typeof(p #> '{substance,term}') IS DISTINCT FROM 'string'
       OR jsonb_typeof(p -> 'info_source') IS DISTINCT FROM 'string' THEN
        RETURN;
    END IF;
    PERFORM cairn_guard_medication_patient(
        (p ->> 'alternative_to')::uuid, e.patient_id, e.content_address);
    INSERT INTO medication_set_aside
        (content_address, event_id, patient_id, version_id, alternative_to, term,
         coding_system, coding_code, coding_display, formulation, dose_amount, dose_unit, sig,
         route, strength, info_source, source_detail, why, t_recorded)
    VALUES (
        e.content_address, e.event_id, e.patient_id,
        (p ->> 'version_id')::uuid, (p ->> 'alternative_to')::uuid,
        p #>> '{substance,term}',
        p #>> '{substance,coding,system}', p #>> '{substance,coding,code}',
        p #>> '{substance,coding,display}',
        p #>> '{substance,formulation}',
        p #>> '{dose,amount}', p #>> '{dose,unit}', p ->> 'sig',
        p ->> 'route', p #>> '{substance,strength}',
        p ->> 'info_source', p ->> 'source_detail', p ->> 'why',
        to_timestamp(e.hlc_wall / 1000.0))
    ON CONFLICT (content_address) DO NOTHING;
END;
$$;
-- An applier WRITES a projection; callable by PUBLIC it would let any role forge rows.
REVOKE EXECUTE ON FUNCTION medication_set_aside_apply(event_log) FROM PUBLIC;

-- Register the applier (db/005, ADR-0057). heal_safe: insert-or-nothing per event. #214 idiom.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('clinical.medication-set-aside.asserted', 'medication_set_aside_apply',
     ARRAY['medication_set_aside'], 20, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
