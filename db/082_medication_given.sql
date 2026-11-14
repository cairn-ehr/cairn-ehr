-- 082_medication_given.sql — charting slice 2: a stat dose, ordered then given.
-- Canonical home: data-model §3.3; the design is spec 2026-11-10 §3.6.
--
-- A STAT ORDER is an ordinary `clinical.medication.asserted` line carrying the top-level key
-- `"stat": true` (db/077 judges and projects it; builder: cairn-event's medication/charting.rs).
-- It is a one-off dose, ordered now and given by someone else — on paper, a line in the "stat"
-- box of the chart that the nurse initials and times when it is given.
--
-- §1 (this section) is the GIVEN: `clinical.medication-given.asserted`, schema
-- `clinical.medication-given/1`, payload exactly `{medication_id, completes}` (builder:
-- cairn-event's medication/given.rs). The envelope's `t_effective` IS the time given.
--   * The type is GENERAL: it says only "a dose of this thread was given", so a future
--     medication-administration record reuses it unchanged.
--   * `completes` is the one stat tie: it names the stop (a cessation, reason "stat dose
--     given") that the give completes, so a stat line reads stopped once given and a reader
--     pairs the give with its stop without guessing. It is JSON null for an ordinary give.
--   * Giving a stat dose is therefore TWO events signed in ONE transaction: the stop first,
--     then the given naming it. The given never edits the line (principle 2).
--
-- THE TWO TIERS (ADR-0056, db/079's idiom). STRUCTURAL rules — the payload's shape, no §5.9
-- claim, no charting key — refuse at BOTH doors. REFERENCE rules — the schema tag, a time given,
-- the line is on THIS chart, `completes` names this line's stop and the line is a stat order —
-- refuse at the AUTHORING door only: a peer's given may arrive before its line or its stop has
-- synced (an ORPHAN, held and shown nowhere, because readers join medication_given to the line on
-- medication_id AND patient_id), and a peer's signed words must never wedge sync.
--
-- WHY THE CHARTING KEYS ARE REFUSED. A given is not a medication line: it was never charted,
-- overrode no warning, restores nothing and is not itself a stat order. No writer produces those
-- keys on it, so refusing them at both doors wedges no peer and stops a given posing as a line.
--
-- THE TIME GIVEN, AND THE t_effective CEILING (found first, as the plan asked). The strict door
-- (db/005 step 1b', ADR-0058, classifier in db/040) compares t_effective with t_recorded (the HLC
-- wall). A time AFTER t_recorded is NOT refused: at the only grades a node may author today
-- (unknown / self-asserted) the ceiling is open above, so the event is ADMITTED and FLAGGED (a
-- row in t_effective_ceiling_flag) — principle 4, a slow clock must not force fabrication. The
-- classifier's 'reject' arm is unreachable at that door (the mint gate refuses every grade that
-- could reach it), and the remote door never rejects on the ceiling. So this floor does not
-- refuse a time given in the future either: the WINDOW refuses "later than now" before the key
-- (GIVEN_LATER_THAN_NOW), and a raw-SQL client's forward-dated given lands with a ceiling flag.
-- A time given EARLIER than the order's t_recorded is legal (backdating is legal) and is shown,
-- worded "before it was charted" — never refused, never reordered (spec §10 Q3).
--
-- WHAT THIS FLOOR DOES NOT JUDGE: whether the line is current, or already given. The window
-- re-reads both before the key (a stale press is refused there); a stat line given twice by two
-- terminals at once is the arrival-order case Task 3's voiding logic must keep honest.
BEGIN;

-- §1. The given.

INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.medication-given.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- Two tiers (header). FLOW: shape (both doors) → the door test → the references.
CREATE OR REPLACE FUNCTION cairn_check_medication_given(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
    k text;
    v_med      uuid;
    v_patient  uuid;
    v_stop     uuid;
BEGIN
    -- ---- Structural: both doors ----
    IF jsonb_typeof(p) IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'medication given: missing payload';
    END IF;
    IF jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string'
       OR NOT pg_input_is_valid(p ->> 'medication_id', 'uuid') THEN
        RAISE EXCEPTION 'medication given: medication_id must be a uuid string';
    END IF;
    -- completes: a uuid string, or JSON null / absent for an ordinary give.
    IF coalesce(jsonb_typeof(p -> 'completes'), 'null') <> 'null'
       AND (jsonb_typeof(p -> 'completes') <> 'string'
            OR NOT pg_input_is_valid(p ->> 'completes', 'uuid')) THEN
        RAISE EXCEPTION 'medication given: completes must be a uuid string (the stop it completes) or null';
    END IF;
    -- A given is not a medication line: refused even as a JSON null.
    FOREACH k IN ARRAY ARRAY['charted_as', 'warning_override', 'restores', 'stat'] LOOP
        IF p ? k THEN
            RAISE EXCEPTION 'medication given: % must be absent (a given dose is not a medication line)', k;
        END IF;
    END LOOP;
    -- No §5.9 claim at BOTH doors (db/020 writes b->'safety' into event_log.safety and
    -- cairn_patient_safety reports any non-null one). A JSON null reads as absent.
    IF jsonb_typeof(b -> 'safety') IS NOT NULL AND jsonb_typeof(b -> 'safety') <> 'null' THEN
        RAISE EXCEPTION 'medication given: a given dose carries no safety claim';
    END IF;

    -- ---- References: the authoring door only (ADR-0056) ----
    IF current_setting('cairn.remote_apply', true) = 'on' THEN
        RETURN;
    END IF;
    IF b ->> 'schema_version' IS DISTINCT FROM 'clinical.medication-given/1' THEN
        RAISE EXCEPTION 'medication given: schema_version must be clinical.medication-given/1, got %', b ->> 'schema_version';
    END IF;
    -- The time given is the event's t_effective; the HLC wall is no stand-in (db/059's rule).
    IF jsonb_typeof(b -> 't_effective') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication given: must carry t_effective (the time the dose was given)';
    END IF;
    v_med     := (p ->> 'medication_id')::uuid;
    v_patient := (b ->> 'patient_id')::uuid;
    -- The line must be on THIS chart (the body's patient_id, the db/075 idiom).
    IF NOT EXISTS (SELECT 1 FROM medication_statement
                    WHERE medication_id = v_med AND patient_id = v_patient) THEN
        RAISE EXCEPTION 'medication given: medication_id % is not a medication on this chart', v_med;
    END IF;
    IF coalesce(jsonb_typeof(p -> 'completes'), 'null') = 'null' THEN
        RETURN;
    END IF;
    v_stop := (p ->> 'completes')::uuid;
    -- completes names THIS line's stop on THIS chart, already in the log: the window signs the
    -- stop first, in the same transaction, so it is there by now. Read from the log, not from
    -- medication_cessation, because that table keeps only one winning stop per thread. A stop's
    -- medication_id is a valid uuid (db/031's floor checks it at both doors); the CASE keeps the
    -- cast off any other event `completes` might name, whose payload need not have one.
    IF NOT EXISTS (SELECT 1 FROM event_log l
                    WHERE l.event_id = v_stop
                      AND l.patient_id = v_patient
                      AND CASE WHEN l.event_type = 'clinical.medication-cessation.asserted'
                               THEN (cairn_clear_payload(l) ->> 'medication_id')::uuid = v_med
                               ELSE false END) THEN
        RAISE EXCEPTION 'medication given: completes % is not a stop of this line on this chart (sign the stop first)', v_stop;
    END IF;
    -- Only a stat order is completed by being given; any other line is given without a stop.
    IF NOT EXISTS (SELECT 1 FROM medication_charting WHERE medication_id = v_med AND stat) THEN
        RAISE EXCEPTION 'medication given: completes is carried only by a stat order, and % is not one', v_med;
    END IF;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_medication_given(text, jsonb) FROM PUBLIC;

-- Register the floor + the hard twin requirement (#173). DO UPDATE converges on replay (#214).
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.medication-given.asserted', 'cairn_check_medication_given',
     'medication given requires a non-empty authored twin (§3.13/§3.3)')
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (cairn_event_twin_check.check_fn, cairn_event_twin_check.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);

-- The projection: one row per given event, keyed by its content address (what a shred scrubs by,
-- db/037), never overlaid — two gives are two rows. given_at is the envelope's t_effective (the
-- time given); hlc_wall is t_recorded (when it was charted). Columns are final: a widened CREATE
-- TABLE never reaches an existing table.
CREATE TABLE IF NOT EXISTS medication_given (
    content_address bytea PRIMARY KEY,
    event_id        uuid NOT NULL,
    patient_id      uuid NOT NULL,
    medication_id   uuid NOT NULL,
    completes       uuid,              -- the stop it completes; NULL for an ordinary give
    given_at        timestamptz NOT NULL,
    hlc_wall        bigint NOT NULL
);
GRANT SELECT ON medication_given TO cairn_agent;
CREATE INDEX IF NOT EXISTS medication_given_patient_idx ON medication_given (patient_id);
CREATE INDEX IF NOT EXISTS medication_given_medication_idx ON medication_given (medication_id);

-- The applier reads the CLEAR payload (ADR-0052): NULL = no custody here, nothing to project (the
-- late-custody path re-runs it). A shape it cannot represent — or a given with no time given,
-- which a peer's sealed body admitted without custody may be — is SKIPPED, never raised: a raise
-- would fail the late-custody heal. Insert-or-nothing, so arrival order never matters.
CREATE OR REPLACE FUNCTION medication_given_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := cairn_clear_payload(e);
BEGIN
    IF p IS NULL
       OR e.t_effective IS NULL
       OR jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string'
       OR NOT pg_input_is_valid(p ->> 'medication_id', 'uuid')
       OR (coalesce(jsonb_typeof(p -> 'completes'), 'null') <> 'null'
           AND (jsonb_typeof(p -> 'completes') <> 'string'
                OR NOT pg_input_is_valid(p ->> 'completes', 'uuid'))) THEN
        RETURN;
    END IF;
    INSERT INTO medication_given
        (content_address, event_id, patient_id, medication_id, completes, given_at, hlc_wall)
    VALUES (e.content_address, e.event_id, e.patient_id, (p ->> 'medication_id')::uuid,
            (p ->> 'completes')::uuid, e.t_effective, e.hlc_wall)
    ON CONFLICT (content_address) DO NOTHING;
END;
$$;
-- An applier WRITES a projection; callable by PUBLIC it would let any role forge rows.
REVOKE EXECUTE ON FUNCTION medication_given_apply(event_log) FROM PUBLIC;

-- Register the applier (db/005, ADR-0057). heal_safe: insert-or-nothing per event. #214 idiom.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('clinical.medication-given.asserted', 'medication_given_apply', ARRAY['medication_given'], 20, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
