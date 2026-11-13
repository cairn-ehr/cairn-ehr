-- 080_medication_version_settle.sql — batch charting slice 2b: settling a set-aside version.
-- Canonical home: data-model §3.3; the design is spec 2026-11-13 §3.3–§3.4.
--
-- A set-aside version (db/079 §2) is settled either way in one signed act. §1 (this section) is
-- the RESTORE — "use this version instead". A restore is TWO events, signed in ONE transaction:
--   1. a cessation of the companion line (the thread the version was set aside beside), and
--   2. a NEW `clinical.medication.asserted` thread carrying the optional top-level key
--      `restores` = the version's `version_id` (builder: cairn-event's medication/charting.rs).
-- The version event itself is never edited (principle 2: never erase, always overlay).
--
-- THE SAFETY RULE. Without the stop, the chart would show two CURRENT lines of one drug — the
-- old companion and the restored line. The window always signs both, but the floor must hold
-- for a raw-SQL client too (ADR-0021: the floor is in the database). So at the AUTHORING door a
-- restore must (a) name a set-aside version of its own chart, checked at submit, and (b) leave
-- that version's companion stopped by COMMIT — in the same transaction, or before.
--
-- WHY (b) IS CHECKED AT COMMIT. The stop and the line are two events and may be submitted in
-- either order, so at the moment the line is submitted the stop may not exist yet. The check
-- is a CONSTRAINT TRIGGER, DEFERRABLE INITIALLY DEFERRED (the db/065 note_draft_spent shape):
-- it runs at COMMIT, when both have landed, and a failure rolls back every write of the
-- transaction. (A client that sets the constraint IMMEDIATE only makes it stricter: the check
-- then runs as the line is submitted, so the stop must come first.)
--
-- WHY A PENDING TABLE, NOT THE cairn.remote_apply SETTING. Only the authoring door owes the
-- pairing: a peer's restore may arrive before its version has synced, and a peer's signed words
-- must never wedge sync (ADR-0056). The usual marker of "which door" is the transaction-local
-- `cairn.remote_apply` setting, but the strict door sets it to '' only for its OWN duration
-- (`SET cairn.remote_apply = ''` on submit_event, db/005). At COMMIT that has gone out of scope,
-- and the setting reads whatever else the transaction last set (cairn_reproject, for one, leaves
-- it 'on') — not the door the restore came through. So the floor records the debt instead: the
-- authoring-door branch of cairn_check_medication_restores INSERTs one row into
-- medication_restore_pending, and that row's existence IS "this restore came through the
-- authoring door". The deferred trigger never reads the setting.
--
-- WHO RUNS THE CHECK. db/031's cairn_check_medication_assertion PERFORMs it (beside db/079's
-- source-note call); plpgsql resolves the callee at run time. The per-type checks run on three
-- paths only: submit_event (the strict door, remote_apply = ''), apply_remote_event and db/043's
-- deferred re-adjudication (both 'on'). A reprojection, a late-custody heal and a shred re-fold
-- run the APPLIERS only, never a check — so no re-fold can write a pending row. The insert is
-- still ON CONFLICT (event_id) DO NOTHING, so a re-submit of the same event in one transaction
-- owes the pairing once.
--
-- THE PENDING TABLE is empty outside a transaction (the trigger deletes its row at COMMIT, and
-- a failed transaction rolls the row back), REVOKE ALL FROM PUBLIC — the check reaches it from
-- inside the SECURITY DEFINER strict door, which runs as the owner — and, like db/078's draft
-- tables, never exported or backed up: a backup carries the event log, and this table holds no
-- fact the log does not.
BEGIN;

-- §1. The restore.
CREATE TABLE IF NOT EXISTS medication_restore_pending (
    event_id   uuid PRIMARY KEY,  -- the restore line's event
    version_id uuid NOT NULL,     -- the set-aside version it restores
    patient_id uuid NOT NULL      -- the chart both are on
);
REVOKE ALL ON medication_restore_pending FROM PUBLIC;

-- The floor for `restores`. Two tiers (ADR-0056, db/079's idiom):
--   STRUCTURAL, both doors: when present, a JSON string that is a valid uuid, or JSON null
--     (= absent, as db/077 treats every optional key). The projection casts it to uuid.
--   REFERENCE, the authoring door only: it names a set-aside version of THIS chart (the body's
--     patient_id, the db/075 idiom); then the pairing is recorded for the COMMIT-time check.
CREATE OR REPLACE FUNCTION cairn_check_medication_restores(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
    v_version uuid;
    v_patient uuid;
BEGIN
    -- ---- Structural: both doors. Absent and JSON null are fine (null = absent). ----
    IF jsonb_typeof(p -> 'restores') IS NULL OR jsonb_typeof(p -> 'restores') = 'null' THEN
        RETURN;
    END IF;
    IF jsonb_typeof(p -> 'restores') <> 'string'
       OR NOT pg_input_is_valid(p ->> 'restores', 'uuid') THEN
        RAISE EXCEPTION 'medication assertion: restores must be a uuid string (a set-aside version_id)';
    END IF;

    -- ---- References: the authoring door only (ADR-0056) ----
    IF current_setting('cairn.remote_apply', true) = 'on' THEN
        RETURN;
    END IF;
    v_version := (p ->> 'restores')::uuid;
    v_patient := (b ->> 'patient_id')::uuid;
    -- A version submitted earlier in the same transaction is already projected, so it counts.
    IF NOT EXISTS (SELECT 1 FROM medication_set_aside
                    WHERE version_id = v_version AND patient_id = v_patient) THEN
        RAISE EXCEPTION 'medication assertion: restores % is not a set-aside version on this chart', v_version;
    END IF;
    -- The debt the COMMIT-time trigger below settles: this restore came through this door.
    INSERT INTO medication_restore_pending (event_id, version_id, patient_id)
    VALUES ((b ->> 'event_id')::uuid, v_version, v_patient)
    ON CONFLICT (event_id) DO NOTHING;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_medication_restores(jsonb) FROM PUBLIC;

-- At COMMIT: every companion of the restored version (every set-aside row of that version_id on
-- that chart — normally one) is stopped, else the whole transaction fails. Then the debt is
-- paid: the pending row is deleted, so the table is empty again. SECURITY DEFINER with a pinned
-- search_path and qualified tables (the db/065 precedent): the trigger runs at COMMIT as the
-- committing role, which holds no privilege on the pending table, and a caller must not be able
-- to shadow medication_cessation with a TEMP table and fake the stop.
CREATE OR REPLACE FUNCTION medication_restore_pairs() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM public.medication_set_aside s
                WHERE s.version_id = NEW.version_id
                  AND s.patient_id = NEW.patient_id
                  AND NOT EXISTS (SELECT 1 FROM public.medication_cessation c
                                   WHERE c.medication_id = s.alternative_to)) THEN
        RAISE EXCEPTION 'a restore must stop the line it replaces in the same transaction (version %)',
            NEW.version_id;
    END IF;
    DELETE FROM public.medication_restore_pending WHERE event_id = NEW.event_id;
    RETURN NULL;
END;
$$;
REVOKE EXECUTE ON FUNCTION medication_restore_pairs() FROM PUBLIC;

DROP TRIGGER IF EXISTS medication_restore_pairs ON medication_restore_pending;
CREATE CONSTRAINT TRIGGER medication_restore_pairs
    AFTER INSERT ON medication_restore_pending
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION medication_restore_pairs();

COMMIT;
