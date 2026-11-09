-- 075_allergy_overlays.sql — the allergy overlays (allergy slice 2; canonical home data-model §3.3):
-- a DE-LABEL by challenge (`clinical.allergy-challenge.asserted`) and an ENTERED-IN-ERROR
-- withdrawal (`clinical.allergy-retraction.asserted`). The third overlay, a CORRECTION, is no new
-- type: it is an ordinary `clinical.allergy.asserted` whose `refs` names the line it replaces, and
-- its floor rule lives in db/074 step 3.
--
-- WHAT IS ON THE WIRE. Two signed, born-sealed event types (ADR-0052). The pure builders live in
-- crates/cairn-event/src/allergy_overlay.rs.
--   clinical.allergy-challenge/1   {target, tolerated: {term, coding}, date: {precision, value}, text}
--   clinical.allergy-retraction/1  {target, text}
-- `target` names the event overlaid; `date` has the shape of an entry's `onset` (ADR-0096), and is
-- checked by the same helper (db/074's cairn_allergy_onset_problem). No t_effective, no safety
-- signal (ADR-0096 decision 21). Nothing is edited or deleted: the read folds overlays over their
-- targets (cairn_node::allergy).
--
-- THE TWO TIERS (ADR-0051, the db/041 idiom, as db/074). STRUCTURAL — a key's JSON type, a uuid
-- target, a blank tolerated term, a date value that contradicts its precision (it must be null
-- exactly when the precision says not-known) — refuses at BOTH doors. CLOSED VOCABULARIES and REFERENCES — the schema tag, the date's words,
-- shape and window, a filled coding, what the target IS and which chart it sits on — refuse at the
-- AUTHORING door only (`cairn.remote_apply` <> 'on'): a peer may overlay a target this node has not
-- received yet, and a newer peer's word must never wedge sync (ADR-0056).
BEGIN;

-- 1. Register both types: additive claims that never target another author (a nurse may strike
--    a doctor's line, as on paper; ADR-0043's owner gate would refuse that).
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.allergy-challenge.asserted', 'additive', FALSE),
    ('clinical.allergy-retraction.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. The shared target rule, structural: a uuid string. Returns the refusal reason or NULL, so
--    each floor raises under its own name. Pure.
CREATE OR REPLACE FUNCTION cairn_allergy_target_problem(p jsonb)
RETURNS text LANGUAGE sql STABLE AS $$
    SELECT CASE WHEN jsonb_typeof(p -> 'target') IS DISTINCT FROM 'string'
                  OR NOT pg_input_is_valid(p ->> 'target', 'uuid')
                THEN 'target must be a uuid string' END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_allergy_target_problem(jsonb) FROM PUBLIC;

-- 3. The target's event type, when it is held here ON THIS CHART (the body's patient_id: clear in
--    the envelope even for a sealed target); NULL otherwise. Authoring door only. Filed under the
--    target's own chart (plan D2, the vouch precedent ADR-0092): an overlay recorded against the
--    wrong patient is the #689 failure.
CREATE OR REPLACE FUNCTION cairn_allergy_target_type(p jsonb, b jsonb)
RETURNS text LANGUAGE sql STABLE AS $$
    SELECT event_type FROM event_log
     WHERE event_id = (p ->> 'target')::uuid AND patient_id = (b ->> 'patient_id')::uuid;
$$;
REVOKE EXECUTE ON FUNCTION cairn_allergy_target_type(jsonb, jsonb) FROM PUBLIC;

-- 4. The de-label floor. Each rule its own message; structural first, then the authoring door.
CREATE OR REPLACE FUNCTION cairn_check_allergy_challenge(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p          jsonb := b -> 'payload';
    v_problem  text;
    v_date_why text;
BEGIN
    -- ---- Structural: both doors ----
    IF jsonb_typeof(p) IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'allergy challenge: missing payload';
    END IF;
    v_problem := cairn_allergy_target_problem(p);
    IF v_problem IS NOT NULL THEN
        RAISE EXCEPTION 'allergy challenge: %', v_problem;
    END IF;
    IF jsonb_typeof(p -> 'tolerated') IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'allergy challenge: tolerated must be an object {term, coding}';
    END IF;
    -- "Blank" is the twin's own Unicode-exact test (db/005), as for an entry's substance.term.
    IF jsonb_typeof(p #> '{tolerated,term}') IS DISTINCT FROM 'string'
       OR NOT cairn_twin_is_present(p #>> '{tolerated,term}') THEN
        RAISE EXCEPTION 'allergy challenge: tolerated.term must be a non-blank string';
    END IF;
    IF NOT (p -> 'tolerated' ? 'coding') THEN
        RAISE EXCEPTION 'allergy challenge: tolerated.coding must be present (null when uncoded)';
    END IF;
    -- One definition of a valid coding object (db/041): null passes; an object needs three strings.
    PERFORM cairn_check_coding_object(p #> '{tolerated,coding}', 'allergy challenge: tolerated.coding');
    IF jsonb_typeof(p -> 'date') IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'allergy challenge: date must be an object {precision, value}';
    END IF;
    IF jsonb_typeof(p #> '{date,precision}') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'allergy challenge: date.precision must be a string';
    END IF;
    -- Principle 4: "not known" is an answer, and it carries no value — exactly then.
    IF jsonb_typeof(p #> '{date,value}') IS DISTINCT FROM
       (CASE WHEN p #>> '{date,precision}' = 'not-known' THEN 'null' ELSE 'string' END) THEN
        RAISE EXCEPTION 'allergy challenge: date.value must be null exactly when date.precision is not-known, else a string';
    END IF;
    IF jsonb_typeof(p -> 'text') IS NULL OR jsonb_typeof(p -> 'text') NOT IN ('string', 'null') THEN
        RAISE EXCEPTION 'allergy challenge: text must be present, a string or null';
    END IF;

    -- ---- Closed vocabularies and references: the authoring door only (ADR-0056) ----
    IF current_setting('cairn.remote_apply', true) = 'on' THEN
        RETURN;
    END IF;
    IF b ->> 'schema_version' IS DISTINCT FROM 'clinical.allergy-challenge/1' THEN
        RAISE EXCEPTION 'allergy challenge: schema_version must be clinical.allergy-challenge/1, got %', b ->> 'schema_version';
    END IF;
    IF p #>> '{date,precision}' NOT IN ('day', 'month', 'year', 'not-known') THEN
        RAISE EXCEPTION 'allergy challenge: date.precision must be day, month, year or not-known, got %', p #>> '{date,precision}';
    END IF;
    v_date_why := cairn_allergy_onset_problem(p #>> '{date,precision}', p #>> '{date,value}');
    IF v_date_why IS NOT NULL THEN
        RAISE EXCEPTION 'allergy challenge: date.value %', v_date_why;
    END IF;
    -- The reserved slot: coding opens in slice 3 (drugref).
    IF jsonb_typeof(p #> '{tolerated,coding}') IS DISTINCT FROM 'null' THEN
        RAISE EXCEPTION 'allergy challenge: tolerated.coding must be null at the authoring door (coding is not built yet)';
    END IF;
    -- A de-label lifts an allergy ENTRY (never a "none known", a de-label or a withdrawal).
    IF cairn_allergy_target_type(p, b) IS DISTINCT FROM 'clinical.allergy.asserted' THEN
        RAISE EXCEPTION 'allergy challenge: target % is not an allergy entry on this chart', p ->> 'target';
    END IF;
END;
$$;
-- The cairn_check_* family is revoked uniformly (#382; db/005).
REVOKE EXECUTE ON FUNCTION cairn_check_allergy_challenge(text, jsonb) FROM PUBLIC;

-- 5. The withdrawal floor.
CREATE OR REPLACE FUNCTION cairn_check_allergy_retraction(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p             jsonb := b -> 'payload';
    v_problem     text;
    v_target_type text;   -- the target's event type, when it is on this chart
BEGIN
    -- ---- Structural: both doors ----
    IF jsonb_typeof(p) IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'allergy retraction: missing payload';
    END IF;
    v_problem := cairn_allergy_target_problem(p);
    IF v_problem IS NOT NULL THEN
        RAISE EXCEPTION 'allergy retraction: %', v_problem;
    END IF;
    IF jsonb_typeof(p -> 'text') IS NULL OR jsonb_typeof(p -> 'text') NOT IN ('string', 'null') THEN
        RAISE EXCEPTION 'allergy retraction: text must be present, a string or null';
    END IF;

    -- ---- Closed vocabulary and reference: the authoring door only ----
    IF current_setting('cairn.remote_apply', true) = 'on' THEN
        RETURN;
    END IF;
    IF b ->> 'schema_version' IS DISTINCT FROM 'clinical.allergy-retraction/1' THEN
        RAISE EXCEPTION 'allergy retraction: schema_version must be clinical.allergy-retraction/1, got %', b ->> 'schema_version';
    END IF;
    v_target_type := cairn_allergy_target_type(p, b);
    -- Plan D3: a wrong withdrawal is repaired by recording the entry again, as on paper — never
    -- by withdrawing the withdrawal (an undo chain no reader could fold safely).
    IF v_target_type = 'clinical.allergy-retraction.asserted' THEN
        RAISE EXCEPTION 'allergy retraction: a withdrawal is never itself withdrawn (record the entry again instead): target %', p ->> 'target';
    END IF;
    -- A de-label is withdrawable (plan M3: that puts the allergy back in force).
    IF v_target_type IS NULL OR v_target_type NOT IN
       ('clinical.allergy.asserted', 'clinical.allergy-none.asserted', 'clinical.allergy-challenge.asserted') THEN
        RAISE EXCEPTION 'allergy retraction: target % is not an allergy entry, none statement or de-label on this chart', p ->> 'target';
    END IF;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_allergy_retraction(text, jsonb) FROM PUBLIC;

-- 6. Register both floors + the hard twin requirement (#173). DO UPDATE converges on replay (#214).
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.allergy-challenge.asserted', 'cairn_check_allergy_challenge',
     'allergy challenge requires a non-empty authored twin (§3.13/§3.3)'),
    ('clinical.allergy-retraction.asserted', 'cairn_check_allergy_retraction',
     'allergy retraction requires a non-empty authored twin (§3.13/§3.3)')
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (cairn_event_twin_check.check_fn, cairn_event_twin_check.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);

-- 7. The projections: one row per overlay event, keyed by the event itself (an overlay is never
--    itself edited in place). content_address is what a shred scrubs by (db/037); patient_id is
--    how a chart reads its list. These columns are final (a widened CREATE TABLE never reaches an
--    existing table on replay).
CREATE TABLE IF NOT EXISTS clinical_allergy_challenge (
    event_id         UUID PRIMARY KEY,
    patient_id       UUID NOT NULL,
    target           UUID NOT NULL,
    tolerated_term   TEXT,
    tolerated_coding JSONB,
    date_precision   TEXT,
    date_value       TEXT,
    text             TEXT,
    hlc_wall         BIGINT,
    hlc_counter      INTEGER,
    origin           TEXT,
    content_address  BYTEA NOT NULL
);
CREATE TABLE IF NOT EXISTS clinical_allergy_retraction (
    event_id        UUID PRIMARY KEY,
    patient_id      UUID NOT NULL,
    target          UUID NOT NULL,
    text            TEXT,
    hlc_wall        BIGINT,
    hlc_counter     INTEGER,
    origin          TEXT,
    content_address BYTEA NOT NULL
);
GRANT SELECT ON clinical_allergy_challenge, clinical_allergy_retraction TO cairn_agent;
CREATE INDEX IF NOT EXISTS clinical_allergy_challenge_patient_idx ON clinical_allergy_challenge (patient_id);
CREATE INDEX IF NOT EXISTS clinical_allergy_challenge_content_address_idx ON clinical_allergy_challenge (content_address);
CREATE INDEX IF NOT EXISTS clinical_allergy_retraction_patient_idx ON clinical_allergy_retraction (patient_id);
CREATE INDEX IF NOT EXISTS clinical_allergy_retraction_content_address_idx ON clinical_allergy_retraction (content_address);

-- 8. The appliers read the CLEAR payload (ADR-0052): NULL means no custody here — nothing to
--    project; the late-custody path re-runs it when a key lands. No schema_version filter (a
--    peer's /2 projects under /1's keys; ADR-0096 decision 8's reasoning). A body the read could
--    not represent — a target that is not a uuid; for a de-label, a tolerated term or a date
--    precision that is not a string (the read takes both as text) — is SKIPPED, never raised: a
--    sealed peer body admitted without custody never met the structural floor, a raise here would
--    fail the late-custody heal (the db/071 idiom), and a NULL there would fail the whole list's
--    read. A skipped overlay is held but unprojected, so the read counts it unreadable and its
--    target stays live (over-warns).
CREATE OR REPLACE FUNCTION clinical_allergy_challenge_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := cairn_clear_payload(e);
BEGIN
    IF p IS NULL OR cairn_allergy_target_problem(p) IS NOT NULL
       OR jsonb_typeof(p #> '{tolerated,term}') IS DISTINCT FROM 'string'
       OR jsonb_typeof(p #> '{date,precision}') IS DISTINCT FROM 'string' THEN
        RETURN;
    END IF;
    INSERT INTO clinical_allergy_challenge
        (event_id, patient_id, target, tolerated_term, tolerated_coding, date_precision, date_value,
         text, hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        e.event_id, e.patient_id, (p ->> 'target')::uuid,
        p #>> '{tolerated,term}',
        NULLIF(p #> '{tolerated,coding}', 'null'::jsonb),
        p #>> '{date,precision}',
        p #>> '{date,value}',
        p ->> 'text',
        e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    ON CONFLICT (event_id) DO NOTHING;
END;
$$;
-- An applier WRITES a projection; callable by PUBLIC it would let any role forge rows.
REVOKE EXECUTE ON FUNCTION clinical_allergy_challenge_apply(event_log) FROM PUBLIC;

CREATE OR REPLACE FUNCTION clinical_allergy_retraction_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := cairn_clear_payload(e);
BEGIN
    IF p IS NULL OR cairn_allergy_target_problem(p) IS NOT NULL THEN RETURN; END IF;
    INSERT INTO clinical_allergy_retraction
        (event_id, patient_id, target, text, hlc_wall, hlc_counter, origin, content_address)
    VALUES (e.event_id, e.patient_id, (p ->> 'target')::uuid, p ->> 'text',
            e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    ON CONFLICT (event_id) DO NOTHING;
END;
$$;
REVOKE EXECUTE ON FUNCTION clinical_allergy_retraction_apply(event_log) FROM PUBLIC;

-- 9. Register the appliers (db/005, ADR-0057). heal_safe: insert-or-nothing per event. #214 idiom.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('clinical.allergy-challenge.asserted', 'clinical_allergy_challenge_apply',
     ARRAY['clinical_allergy_challenge'], 20, TRUE),
    ('clinical.allergy-retraction.asserted', 'clinical_allergy_retraction_apply',
     ARRAY['clinical_allergy_retraction'], 20, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
