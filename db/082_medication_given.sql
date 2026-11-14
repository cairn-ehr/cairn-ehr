-- 082_medication_given.sql — charting slice 2: a stat dose, ordered then given.
-- Canonical home: data-model §3.3; the design is spec 2026-11-10 §3.6.
--
-- A STAT ORDER is an ordinary `clinical.medication.asserted` line carrying the top-level key
-- `"stat": true` (db/077 judges and projects it; builder: cairn-event's medication/charting.rs).
-- It is a one-off dose, ordered now and given by someone else — on paper, a line in the "stat"
-- box of the chart that the nurse initials and times when it is given.
--
-- §1 is the GIVEN: `clinical.medication-given.asserted`, schema
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
-- the line is on THIS chart, `completes` names this line's stop signed in the same transaction,
-- the line is a stat order — refuse at the AUTHORING door only: a peer's given may arrive before
-- its line or its stop has synced (an ORPHAN, held and shown nowhere, because readers join
-- medication_given to the line on medication_id AND patient_id), and a peer's signed words must
-- never wedge sync.
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
-- terminals at once is two stops, and striking one give leaves the other's stop standing (§2).
-- §2 is "ENTERED IN ERROR": `clinical.medication-given-retraction.asserted`, schema
-- `clinical.medication-given-retraction/1`, payload exactly `{medication_id, target, voids, text}`.
-- `target` is the struck given; `voids` is the stop that given completed (null for an ordinary
-- give); `text` is the optional why. On paper: a line through the nurse's initials, "entered in
-- error", and the stat box is open again. §2's own header (below) says how the stop is voided.
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
    -- completes names THIS line's stop on THIS chart, signed in THIS transaction (the window signs
    -- the stop first, then the given). WHY the same transaction: a given may only complete the
    -- stop signed with it. A stop from an earlier act is someone's real decision ("patient
    -- refused"); a given naming it would let a later "entered in error" strike void that real stop
    -- (§2's retraction voids exactly what `completes` names) and show the line current again.
    -- `xmin = pg_current_xact_id()` is db/065's idiom for "inserted by this transaction". Read from
    -- the log, not from medication_cessation, because that table keeps only one winning stop per
    -- thread. A stop's medication_id is a valid uuid (db/031's floor checks it at both doors);
    -- the CASE keeps the cast off any other event `completes` might name, whose payload need not
    -- have one.
    IF NOT EXISTS (SELECT 1 FROM event_log l
                    WHERE l.event_id = v_stop
                      AND l.patient_id = v_patient
                      AND l.xmin = pg_current_xact_id()::xid
                      AND CASE WHEN l.event_type = 'clinical.medication-cessation.asserted'
                               THEN (cairn_clear_payload(l) ->> 'medication_id')::uuid = v_med
                               ELSE false END) THEN
        RAISE EXCEPTION 'medication given: completes % is not a stop of this line on this chart signed in this transaction (sign the stop first, with the given)', v_stop;
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

-- §2. Entered in error: the struck given, and the stop it voids.
--
-- WHY THE RETRACTION CARRIES `voids` ITSELF (Review Focus 3). Striking a give must re-open the
-- stat line, so the stop the give caused must stop counting. The void keys on the RETRACTION
-- ALONE — `voids` on its own row — never on the given's `completes`. So shredding the given (its
-- row goes) cannot bring the stop back; shredding the retraction can, and should: the strike is
-- gone, so "given" stands again. Shredding the voided stop itself changes nothing visible.
--
-- HOW A STOP IS VOIDED, IN EITHER ARRIVAL ORDER.
--   * The stop arrives AFTER the retraction: db/031's medication_cessation_apply and db/060's
--     medication_cessation_reason_apply ask cairn_medication_cessation_voided first and write
--     nothing for a voided stop. It simply never wins.
--   * The stop arrived FIRST, and may hold the thread's one winning medication_cessation row:
--     this applier deletes that stop's rows (by its content_address) and re-folds the thread
--     through cairn_refold_after_shred (db/037, #863), so the next stop in line — a REAL stop of
--     the same line — wins again, or none does and the line reads current (Review Focus 1).
-- Both paths end in the same rows, so the outcome is arrival-order-independent, as set-union sync
-- requires. A heal changes nothing (the stop's applier skips it, and the delete finds nothing).
--
-- WHY THE RE-FOLD RUNS ONLY WHEN THE DELETE TOOK THE WINNER (a departure from the plan's text,
-- found while building it). The retraction type is registered against medication_cessation, so
-- cairn_refold_after_shred replays retractions of the thread too — including this one. Run
-- unconditionally, that replay calls this applier, which re-folds again, without end (stack depth
-- exceeded). Run only when a medication_cessation row was deleted, the nested replay finds
-- nothing to delete and stops. That is also the only case that needs it: a voided stop that was
-- NOT the winner left the winner (another stop) standing, and a stop that never arrived left
-- nothing. Each retraction can delete at most once after its own row exists (its stop is skipped
-- from then on), so nesting is bounded by the thread's retraction count (a rebuild's worst case).
--
-- THE THREE FINDINGS THE PLAN ASKED FOR.
--   (a) LOCKS AND POSTURE. cairn_refold_after_shred needs no shred to call it: it saves the
--       caller's cairn.remote_apply, replays lenient and puts it back, so nested inside
--       cairn_reproject's heal (posture already 'on') it restores 'on', and at the authoring door
--       it restores the strict posture for the rest of the transaction. This applier takes
--       cairn_lock_shred_refold() before its delete anyway. Without it, a fresh strike and a
--       concurrent shred of a stop of the same thread can interleave: the shred's re-fold, not
--       seeing the uncommitted strike, re-upserts the voided stop on the row the strike just
--       deleted (or the two deadlock on that row and a custody lock). With it, whichever comes
--       second waits and then reads the first's committed rows. The lock is re-entrant, so inside
--       a shred's own re-fold it costs nothing, and a heal holds cairn_lock_erasure_heal
--       EXCLUSIVE, so no shred is open to contend. The door already holds its own custody lock
--       when it takes this (the inverse of a shred's order), but a shred never waits on an
--       uncommitted event's custody (it cannot see it); only a LATE key for a committed strike
--       can meet a shred in a cycle, which Postgres aborts with 40P01 (db/005's accepted shape).
--   (b) A NARROW REBUILD. db/039 refuses to rebuild a prefix whose tables another type outside it
--       also feeds. This type lists medication_cessation (it deletes from it), which the cessation
--       type feeds, so a rebuild of 'clinical.medication-given-retraction' — or of
--       'clinical.medication-given', which matches both new types — RAISES before any TRUNCATE:
--       it can never truncate the stops and replay only retractions. Pinned by
--       a_narrow_rebuild_of_the_retraction_type_keeps_every_stop. A rebuild of the given type
--       alone needs the trailing dot ('clinical.medication-given.'). 'clinical.medication' (or
--       '') rebuilds both stops and strikes: stops replay first (by type name) and every voided
--       one is inserted, then each strike deletes its stop and re-folds — the same rows.
--   (c) A SHRED OF THE RETRACTION. db/037's scrub deletes its medication_given_retraction row
--       (added there), then cairn_refold_after_shred replays every heal-safe type sharing a
--       non-flag table with this one: the cessation type shares medication_cessation and
--       medication_cessation_reason, and the retraction's clear payload names its thread
--       (`medication_id`, read by cairn_medication_threads_named). So the struck give's stop,
--       no longer voided, is re-applied and wins on its HLC: the line reads stopped again.
--
-- TRUST AT THE REMOTE DOOR. `voids` is checked against the given's `completes` at the authoring
-- door only (ADR-0056): a peer's strike may arrive before its given. A peer's signed retraction is
-- therefore trusted for its own `voids`, which its origin's authoring door held to that rule. The
-- void is confined to the retraction's own thread (medication_id), so it can never void another
-- line's stop.
--
-- THE TWO TIERS. Structural — the payload's shape, no §5.9 claim, no charting key — at BOTH doors.
-- References — the schema tag, the target is a given of THIS line on THIS chart, `voids` is what
-- that given completes — at the authoring door only. A second strike of one given is admitted and
-- harmless (its delete finds nothing); the window refuses it before the key (GIVE_GONE).

INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.medication-given-retraction.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

CREATE OR REPLACE FUNCTION cairn_check_medication_given_retraction(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
    k text;
    v_completes uuid;
BEGIN
    -- ---- Structural: both doors ----
    IF jsonb_typeof(p) IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'medication given retraction: missing payload';
    END IF;
    FOREACH k IN ARRAY ARRAY['medication_id', 'target'] LOOP
        IF jsonb_typeof(p -> k) IS DISTINCT FROM 'string' OR NOT pg_input_is_valid(p ->> k, 'uuid') THEN
            RAISE EXCEPTION 'medication given retraction: % must be a uuid string', k;
        END IF;
    END LOOP;
    -- voids: a uuid string, or JSON null / absent when the struck give stopped nothing.
    IF coalesce(jsonb_typeof(p -> 'voids'), 'null') <> 'null'
       AND (jsonb_typeof(p -> 'voids') <> 'string' OR NOT pg_input_is_valid(p ->> 'voids', 'uuid')) THEN
        RAISE EXCEPTION 'medication given retraction: voids must be a uuid string (the stop it voids) or null';
    END IF;
    IF coalesce(jsonb_typeof(p -> 'text'), 'null') NOT IN ('string', 'null') THEN
        RAISE EXCEPTION 'medication given retraction: text must be a string or null';
    END IF;
    FOREACH k IN ARRAY ARRAY['charted_as', 'warning_override', 'restores', 'stat'] LOOP
        IF p ? k THEN
            RAISE EXCEPTION 'medication given retraction: % must be absent (a strike is not a medication line)', k;
        END IF;
    END LOOP;
    IF jsonb_typeof(b -> 'safety') IS NOT NULL AND jsonb_typeof(b -> 'safety') <> 'null' THEN
        RAISE EXCEPTION 'medication given retraction: a strike carries no safety claim';
    END IF;

    -- ---- References: the authoring door only (ADR-0056) ----
    IF current_setting('cairn.remote_apply', true) = 'on' THEN
        RETURN;
    END IF;
    IF b ->> 'schema_version' IS DISTINCT FROM 'clinical.medication-given-retraction/1' THEN
        RAISE EXCEPTION 'medication given retraction: schema_version must be clinical.medication-given-retraction/1, got %', b ->> 'schema_version';
    END IF;
    -- The struck given is a projected given of THIS line on THIS chart (a given row exists only
    -- once its stop and line checks passed, §1).
    SELECT g.completes INTO v_completes
      FROM medication_given g
     WHERE g.event_id = (p ->> 'target')::uuid
       AND g.patient_id = (b ->> 'patient_id')::uuid
       AND g.medication_id = (p ->> 'medication_id')::uuid;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'medication given retraction: target % is not a given dose of this line on this chart', p ->> 'target';
    END IF;
    -- It voids exactly the stop that given completed: never a real stop, never one it left alone.
    IF (p ->> 'voids')::uuid IS DISTINCT FROM v_completes THEN
        RAISE EXCEPTION 'medication given retraction: voids % must be the stop the struck given completes (%)', p ->> 'voids', v_completes;
    END IF;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_medication_given_retraction(text, jsonb) FROM PUBLIC;

INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.medication-given-retraction.asserted', 'cairn_check_medication_given_retraction',
     'medication given retraction requires a non-empty authored twin (§3.13/§3.3)')
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (cairn_event_twin_check.check_fn, cairn_event_twin_check.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);

-- One row per strike, keyed by its content address (what a shred scrubs by). Columns are final.
CREATE TABLE IF NOT EXISTS medication_given_retraction (
    content_address bytea PRIMARY KEY,
    event_id        uuid NOT NULL,
    patient_id      uuid NOT NULL,
    medication_id   uuid NOT NULL,
    target          uuid NOT NULL,     -- the struck given
    voids           uuid,              -- the stop it voids; NULL when the give stopped nothing
    text            text,
    hlc_wall        bigint NOT NULL
);
GRANT SELECT ON medication_given_retraction TO cairn_agent;
CREATE INDEX IF NOT EXISTS medication_given_retraction_patient_idx ON medication_given_retraction (patient_id);
CREATE INDEX IF NOT EXISTS medication_given_retraction_target_idx ON medication_given_retraction (target);
-- Every stop's apply asks cairn_medication_cessation_voided, so this one is on the hot path.
CREATE INDEX IF NOT EXISTS medication_given_retraction_voids_idx ON medication_given_retraction (voids);

-- Is this stop voided by a strike of its own thread? Called by db/031's and db/060's appliers
-- (plpgsql binds at execution, so the later file is no obstacle).
CREATE OR REPLACE FUNCTION cairn_medication_cessation_voided(p_cessation uuid, p_med uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (SELECT 1 FROM medication_given_retraction
                    WHERE voids = p_cessation AND medication_id = p_med)
$$;
REVOKE EXECUTE ON FUNCTION cairn_medication_cessation_voided(uuid, uuid) FROM PUBLIC;

-- The applier (the header's "HOW A STOP IS VOIDED"). Clear payload; NULL = no custody here, and
-- the late-custody path re-runs it. A shape it cannot represent is SKIPPED, never raised.
CREATE OR REPLACE FUNCTION medication_given_retraction_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p     jsonb := cairn_clear_payload(e);
    v_med uuid;
    v_ca  bytea;
BEGIN
    IF p IS NULL
       OR jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string'
       OR NOT pg_input_is_valid(p ->> 'medication_id', 'uuid')
       OR jsonb_typeof(p -> 'target') IS DISTINCT FROM 'string'
       OR NOT pg_input_is_valid(p ->> 'target', 'uuid')
       OR (coalesce(jsonb_typeof(p -> 'voids'), 'null') <> 'null'
           AND (jsonb_typeof(p -> 'voids') <> 'string'
                OR NOT pg_input_is_valid(p ->> 'voids', 'uuid')))
       OR coalesce(jsonb_typeof(p -> 'text'), 'null') NOT IN ('string', 'null') THEN
        RETURN;
    END IF;
    v_med := (p ->> 'medication_id')::uuid;
    INSERT INTO medication_given_retraction
        (content_address, event_id, patient_id, medication_id, target, voids, text, hlc_wall)
    VALUES (e.content_address, e.event_id, e.patient_id, v_med, (p ->> 'target')::uuid,
            (p ->> 'voids')::uuid, p ->> 'text', e.hlc_wall)
    ON CONFLICT (content_address) DO NOTHING;
    IF coalesce(jsonb_typeof(p -> 'voids'), 'null') = 'null' THEN
        RETURN;
    END IF;
    -- Finding (a): serialize with shreds before touching the stop's rows.
    PERFORM cairn_lock_shred_refold();
    SELECT content_address INTO v_ca FROM event_log WHERE event_id = (p ->> 'voids')::uuid;
    IF v_ca IS NULL THEN
        RETURN;   -- the stop has not arrived; its own applier will skip it (voided)
    END IF;
    -- Every reason row of that stop (one per event), then its winner row if it holds one. The
    -- medication_id keeps both deletes inside the strike's own thread.
    DELETE FROM medication_cessation_reason WHERE content_address = v_ca AND medication_id = v_med;
    DELETE FROM medication_cessation WHERE content_address = v_ca AND medication_id = v_med;
    IF FOUND THEN
        -- The stop held the thread's winner: hand it to the next stop in line, if any (header).
        PERFORM cairn_refold_after_shred((p ->> 'voids')::uuid,
                                         'clinical.medication-cessation.asserted',
                                         ARRAY[v_med::text]);
    END IF;
END;
$$;
REVOKE EXECUTE ON FUNCTION medication_given_retraction_apply(event_log) FROM PUBLIC;

-- Registered against all three tables it writes, so db/039's narrow rebuild and db/037's
-- re-fold both see that it feeds the stops (findings b and c). heal_safe: insert-or-nothing,
-- and its delete is idempotent. #214 idiom.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('clinical.medication-given-retraction.asserted', 'medication_given_retraction_apply',
     ARRAY['medication_given_retraction', 'medication_cessation', 'medication_cessation_reason'], 30, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
