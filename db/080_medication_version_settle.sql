-- 080_medication_version_settle.sql — batch charting slice 2b: settling a set-aside version.
-- Canonical home: data-model §3.3; the design is spec 2026-11-13 §3.3–§3.4.
--
-- A set-aside version (db/079 §2) is settled either way in one signed act. §1 (this section) is
-- the RESTORE — "use this version instead". A restore is TWO events, signed in ONE transaction:
--   1. a cessation of the line that stands for the companion (below), and
--   2. a NEW `clinical.medication.asserted` thread carrying the optional top-level key
--      `restores` = the version's `version_id` (builder: cairn-event's medication/charting.rs).
-- The version event itself is never edited (principle 2: never erase, always overlay).
--
-- THE SAFETY RULE (rulings R3, R4). A restore REPLACES WHATEVER CURRENTLY STANDS FOR THE
-- COMPANION (the thread the version was set aside beside). That is the companion itself, or any
-- line descended from it through restores: a restore of a version of the companion, a restore of
-- a version of THAT line, and so on — the companion's RESTORE LINEAGE
-- (cairn_medication_restore_lineage, below). Without the stop the chart would show two CURRENT
-- lines of one drug. The window always signs the stop, but the floor must hold for a raw-SQL
-- client too (ADR-0021: the floor is in the database). So at the AUTHORING door a restore must,
-- checked at submit:
--   (a) name a set-aside version of its own chart, and
--   (b) be a NEW thread (`restores` records how a thread began; it is never added to a line
--       that already exists);
-- and, checked at COMMIT:
--   (c) no CURRENT member of the companion's lineage other than this restore's own thread —
--       unless this restore's own thread is itself stopped (a restore stopped in the same
--       transaction stands for nothing, so two restores of one companion may be signed together
--       as long as only the last stands).
-- "Current" = in medication_statement with no medication_cessation row. The stops may be signed
-- in the same transaction or before. Spec §3.3 said a restore emits no cessation when the
-- companion was "already replaced by another version's restore"; ruling R3 supersedes that,
-- because it left two current lines (a companion stopped for another reason, with no current
-- replacement, still needs no cessation).
--
-- WHY THE LINEAGE IS RECURSIVE (ruling R4). One hop is not enough: X has versions V1 and V2;
-- restoring V1 gives L1 (X stopped); V3 is set aside beside L1, and restoring it gives L3 (L1
-- stopped). Restoring V2 next must stop L3 — reached from X only through L1. The closure follows
-- `restores` from version to line as far as it goes. The window lists what a restore must stop
-- with the SAME function, so what the window offers and what the floor demands cannot drift.
--
-- WHY THE LINEAGE IS ROOTED (ruling R10). A version may be set aside beside a restore line, so a
-- restore's companion need not be the original line: X charted; L1 restored v1 (X stopped); v3
-- is set aside beside L1 while an older v2 is still beside X. Restoring v2 (A) stops L1; a later
-- restore of v3 (B) has companion L1, whose closure DOWNWARD is only {L1, B} — A, its sibling
-- under X, is not in it, and both would stand. So the function first walks UP, from a restore
-- line to the companion of the version it restored, to the original line, and returns the whole
-- tree below it. The floor's COMMIT check and the window's "what stands" read it alike, so a
-- restore anywhere in the tree must stop whatever stands anywhere in it.
--
-- A DOCUMENTED LIMIT: AN ERASED LINK (#1097). The closure walks projected rows, so a shred that
-- deletes one of them cuts the chain:
--   (1) a shredded set-aside VERSION in the middle of a chain hides the whole subtree below it
--       from the lineage;
--   (2) a shredded restore EVENT whose line stays current through a later re-assertion loses
--       its medication_restore row, so that line drops out of every lineage.
-- In both cases a later restore of a sibling version passes, leaving two current lines.
-- Accepted, not fixed: it needs a deliberate erasure; both lines stay visible on the chart (an
-- over-statement, never a silent loss); erasure is best-effort and declared (ADR-0005); and the
-- erased event was the only holder of the link, so nothing left could restore it.
--
-- WHY `restores` HAS ITS OWN PROJECTION (ruling R5), medication_restore, rather than a column on
-- medication_charting. That table holds one HLC-winner row per thread, so a sealed restore that
-- heals late (its key arrives after a newer re-assertion) would lose to the newer row, and a
-- shred, which scrubs by the producing event's content address, could miss it. Here a row is
-- the restore EVENT's fact, keyed by its content address, insert-or-nothing: arrival order never
-- matters and a shred finds it (db/037).
--
-- WHY THE PAIRING IS CHECKED AT COMMIT. The stop and the line are two events and may be
-- submitted in either order, so at the moment the line is submitted the stop may not exist yet.
-- The check is a CONSTRAINT TRIGGER, DEFERRABLE INITIALLY DEFERRED (the db/065 note_draft_spent
-- shape): it runs at COMMIT, when both have landed, and a failure rolls back every write of the
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
-- authoring-door branch of cairn_check_medication_restores INSERTs into
-- medication_restore_pending, and that row's existence IS "this restore came through the
-- authoring door". The deferred trigger never reads the setting.
--
-- WHO RUNS THE CHECK. db/031's cairn_check_medication_assertion PERFORMs it (beside db/079's
-- source-note call); plpgsql resolves the callee at run time. The per-type checks run on three
-- paths only: submit_event (the strict door, remote_apply = ''), apply_remote_event and db/043's
-- deferred re-adjudication (both 'on'). A reprojection, a late-custody heal and a shred re-fold
-- run the APPLIERS only, never a check — so no re-fold can write a pending row. A re-submit of
-- an event ALREADY IN THE LOG skips the reference rules entirely: db/005 makes that a silent
-- no-op (its pairing was settled when it first committed), and a different body reusing the
-- event_id is refused there by the substitution guard.
--
-- THE PENDING TABLE is empty outside a transaction (the trigger deletes its row at COMMIT, and
-- a failed transaction rolls the row back), REVOKE ALL FROM PUBLIC — the check reaches it from
-- inside the SECURITY DEFINER strict door, which runs as the owner — and, like db/078's draft
-- tables, never exported or backed up: a backup carries the event log, and this table holds no
-- fact the log does not.
BEGIN;

-- §1. The restore.

-- The projection: one row per restore event, keyed by its content address (what a shred scrubs
-- by, db/037). patient_id is the event's chart. Columns are final: a widened CREATE TABLE never
-- reaches an existing table.
CREATE TABLE IF NOT EXISTS medication_restore (
    content_address bytea PRIMARY KEY,
    event_id        uuid NOT NULL,
    patient_id      uuid NOT NULL,
    medication_id   uuid NOT NULL,  -- the restore line's own thread
    version_id      uuid NOT NULL   -- the set-aside version it restores
);
GRANT SELECT ON medication_restore TO cairn_agent;
CREATE INDEX IF NOT EXISTS medication_restore_patient_idx ON medication_restore (patient_id);
CREATE INDEX IF NOT EXISTS medication_restore_version_idx ON medication_restore (version_id);
CREATE INDEX IF NOT EXISTS medication_restore_medication_idx ON medication_restore (medication_id);
-- The floor and the lineage look versions up by version_id.
CREATE INDEX IF NOT EXISTS medication_set_aside_version_idx ON medication_set_aside (version_id);

-- The applier, a second one on clinical.medication.asserted (beside db/031's and db/077's). It
-- reads the CLEAR payload (ADR-0052): NULL = no custody here, nothing to project (the
-- late-custody path re-runs it). A shape it cannot represent is SKIPPED, never raised — a sealed
-- peer body admitted without custody never met the structural floor, and a raise would fail the
-- late-custody heal. Insert-or-nothing per event, so arrival order never matters.
CREATE OR REPLACE FUNCTION medication_restore_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := cairn_clear_payload(e);
BEGIN
    IF p IS NULL
       OR jsonb_typeof(p -> 'restores') IS DISTINCT FROM 'string'
       OR NOT pg_input_is_valid(p ->> 'restores', 'uuid')
       OR jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string'
       OR NOT pg_input_is_valid(p ->> 'medication_id', 'uuid') THEN
        RETURN;
    END IF;
    INSERT INTO medication_restore (content_address, event_id, patient_id, medication_id, version_id)
    VALUES (e.content_address, e.event_id, e.patient_id,
            (p ->> 'medication_id')::uuid, (p ->> 'restores')::uuid)
    ON CONFLICT (content_address) DO NOTHING;
END;
$$;
-- An applier WRITES a projection; callable by PUBLIC it would let any role forge rows.
REVOKE EXECUTE ON FUNCTION medication_restore_apply(event_log) FROM PUBLIC;

-- Register the applier (db/005, ADR-0057). heal_safe: insert-or-nothing per event. #214 idiom.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('clinical.medication.asserted', 'medication_restore_apply', ARRAY['medication_restore'], 20, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

-- The companion's RESTORE LINEAGE on one chart, ROOTED (header: why recursive, why rooted).
-- `up`: the companion, and from each line that is a restore, the companion of the version it
-- restored, up to the original line. `lineage`: everything in `up`, plus every line whose
-- restore names a set-aside version whose alternative_to is already in the set, recursively.
-- Walking down from every member of `up` gives exactly the tree below the root (each member of
-- `up` is itself reached down from the root), and still gives an answer when a forged cycle has
-- no root. UNION, not UNION ALL, in both walks, so a cycle a peer could construct still ends.
-- The ONE answer to "what stands for this companion": the COMMIT trigger below demands that no
-- current member but the restore itself survive, and the window (Task 5's node read) lists the
-- current members as what a restore must stop. STABLE; SECURITY DEFINER with a pinned
-- search_path and qualified tables so the window's role gets the answer without needing more
-- than EXECUTE, and no caller can shadow a table with a TEMP one.
CREATE OR REPLACE FUNCTION cairn_medication_restore_lineage(p_companion uuid, p_patient uuid)
RETURNS SETOF uuid LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    WITH RECURSIVE up(medication_id) AS (
        SELECT p_companion
        UNION
        SELECT s.alternative_to
          FROM up u
          JOIN public.medication_restore r
            ON r.medication_id = u.medication_id AND r.patient_id = p_patient
          JOIN public.medication_set_aside s
            ON s.version_id = r.version_id AND s.patient_id = p_patient
    ), lineage(medication_id) AS (
        SELECT medication_id FROM up
        UNION
        SELECT r.medication_id
          FROM lineage l
          JOIN public.medication_set_aside s
            ON s.alternative_to = l.medication_id AND s.patient_id = p_patient
          JOIN public.medication_restore r
            ON r.version_id = s.version_id AND r.patient_id = p_patient
    )
    SELECT medication_id FROM lineage;
$$;
REVOKE EXECUTE ON FUNCTION cairn_medication_restore_lineage(uuid, uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_medication_restore_lineage(uuid, uuid) TO cairn_window, cairn_agent;

-- The pending pairing: one row per (restore event, companion), written at SUBMIT. The companion
-- is captured there, so the COMMIT check never needs the version row to survive to COMMIT (a
-- shred deletes that row by content_address). Normally one companion per version; a peer may
-- have sent two versions sharing a version_id, and then each companion is owed.
--
-- One-off for a dev database that loaded this file's first draft (event_id-only key, no
-- medication_id / companion): the table is empty outside a transaction, so dropping it loses
-- nothing, and the CREATE below rebuilds it (the trigger, dropped with it, is recreated below).
DO $$
BEGIN
    IF to_regclass('public.medication_restore_pending') IS NOT NULL
       AND NOT EXISTS (SELECT 1 FROM information_schema.columns
                        WHERE table_schema = 'public' AND table_name = 'medication_restore_pending'
                          AND column_name = 'companion') THEN
        DROP TABLE medication_restore_pending;
    END IF;
END $$;
CREATE TABLE IF NOT EXISTS medication_restore_pending (
    event_id      uuid NOT NULL,  -- the restore line's event
    medication_id uuid NOT NULL,  -- the restore line's own (new) thread
    version_id    uuid NOT NULL,  -- the set-aside version it restores
    companion     uuid NOT NULL,  -- that version's alternative_to, read at submit
    patient_id    uuid NOT NULL,  -- the chart all of them are on
    PRIMARY KEY (event_id, companion)
);
REVOKE ALL ON medication_restore_pending FROM PUBLIC;

-- The floor for `restores`. Two tiers (ADR-0056, db/079's idiom):
--   STRUCTURAL, both doors: when present, a JSON string that is a valid uuid, or JSON null
--     (= absent, as db/077 treats every optional key). The projection casts it to uuid.
--   REFERENCE, the authoring door only, and only for an event not already in the log (header):
--     it names a set-aside version of THIS chart (the body's patient_id, the db/075 idiom), on a
--     NEW thread; then the pairing is recorded, with the companion it is owed against, for the
--     COMMIT-time check.
CREATE OR REPLACE FUNCTION cairn_check_medication_restores(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
    v_version uuid;
    v_patient uuid;
    v_med     uuid;
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
    -- An idempotent re-submit: its thread now exists, and its pairing was settled at its first
    -- COMMIT. db/005 makes it a silent no-op (or refuses a substitute body), so judge nothing.
    IF EXISTS (SELECT 1 FROM event_log WHERE event_id = (b ->> 'event_id')::uuid) THEN
        RETURN;
    END IF;
    v_version := (p ->> 'restores')::uuid;
    v_patient := (b ->> 'patient_id')::uuid;
    -- db/031 has already checked medication_id is a valid uuid.
    v_med     := (p ->> 'medication_id')::uuid;
    -- (a) A version submitted earlier in the same transaction is already projected, so it counts.
    IF NOT EXISTS (SELECT 1 FROM medication_set_aside
                    WHERE version_id = v_version AND patient_id = v_patient) THEN
        RAISE EXCEPTION 'medication assertion: restores % is not a set-aside version on this chart', v_version;
    END IF;
    -- (b) A NEW thread only: this event is not projected yet, so a known thread is an older one.
    IF cairn_medication_thread_patient(v_med) IS NOT NULL THEN
        RAISE EXCEPTION 'medication assertion: restores is carried only by a new line, and % already exists', v_med;
    END IF;
    -- The debt the COMMIT-time trigger below settles: this restore came through this door.
    INSERT INTO medication_restore_pending (event_id, medication_id, version_id, companion, patient_id)
    SELECT DISTINCT (b ->> 'event_id')::uuid, v_med, v_version, s.alternative_to, v_patient
      FROM medication_set_aside s
     WHERE s.version_id = v_version AND s.patient_id = v_patient
    ON CONFLICT DO NOTHING;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_medication_restores(jsonb) FROM PUBLIC;

-- At COMMIT, per pending row, rule (c): unless this restore's own thread is itself stopped, no
-- other CURRENT member of the companion's lineage may remain. Else the whole transaction fails.
-- Then the debt is paid: the pending row is deleted, so the table is empty again. SECURITY
-- DEFINER with a pinned search_path and qualified tables (the db/065 precedent): the trigger
-- runs at COMMIT as the committing role, which holds no privilege on the pending table, and a
-- caller must not be able to shadow medication_cessation with a TEMP table and fake a stop.
CREATE OR REPLACE FUNCTION medication_restore_pairs() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_standing uuid;
BEGIN
    -- A restore whose own thread is STOPPED stands for nothing, so it owes nothing (a later
    -- restore in the transaction replaced it). Keyed on the cessation row, NOT on "no current
    -- statement": under SET CONSTRAINTS IMMEDIATE this runs before the restore's own line is in
    -- the log, and reading its absence as "not current" would let the restore skip the check.
    IF NOT EXISTS (SELECT 1 FROM public.medication_cessation c
                    WHERE c.medication_id = NEW.medication_id) THEN
        SELECT l.medication_id INTO v_standing
          FROM public.cairn_medication_restore_lineage(NEW.companion, NEW.patient_id)
               AS l(medication_id)
         WHERE l.medication_id <> NEW.medication_id
           AND EXISTS (SELECT 1 FROM public.medication_statement m
                        WHERE m.medication_id = l.medication_id)
           AND NOT EXISTS (SELECT 1 FROM public.medication_cessation c
                            WHERE c.medication_id = l.medication_id)
         LIMIT 1;
        IF v_standing IS NOT NULL THEN
            RAISE EXCEPTION 'a restore must stop the line it replaces in the same transaction (version %: line % still stands for line %)',
                NEW.version_id, v_standing, NEW.companion;
        END IF;
    END IF;
    DELETE FROM public.medication_restore_pending
     WHERE event_id = NEW.event_id AND companion = NEW.companion;
    RETURN NULL;
END;
$$;
REVOKE EXECUTE ON FUNCTION medication_restore_pairs() FROM PUBLIC;

DROP TRIGGER IF EXISTS medication_restore_pairs ON medication_restore_pending;
CREATE CONSTRAINT TRIGGER medication_restore_pairs
    AFTER INSERT ON medication_restore_pending
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION medication_restore_pairs();

-- §2. "Checked: not current" — a set-aside version looked at again and found still not current.
-- Its own event type (ADR-0102 / spec §3.4): the type IS the verdict, so there is no flag to get
-- backwards; the opposite outcome is a restore (§1). Never a medication line, and it never starts
-- a thread; it sits on its version's companion thread (db/048 §10). It is not in
-- medication_statement, the active read or the safety projection. A second check of the same
-- version is admitted and shown. Modelled on db/079 §2's set-aside type.

INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.medication-set-aside-checked.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- Two tiers (ADR-0056):
--   STRUCTURAL, both doors: version_id a uuid string; info_source a non-blank string; the
--     current-line keys absent; no §5.9 claim; the shared From:/why note judged as a charted line's.
--   REFERENCE, the authoring door only: the schema tag, and version_id names a set-aside version
--     of THIS chart. A peer's check may arrive before its version has synced; it is held and shown
--     nowhere (readers join on version_id AND patient_id), so sync never wedges.
CREATE OR REPLACE FUNCTION cairn_check_medication_version_checked(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
    k text;
BEGIN
    -- ---- Structural: both doors ----
    IF jsonb_typeof(p) IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'medication version-checked: missing payload';
    END IF;
    IF jsonb_typeof(p -> 'version_id') IS DISTINCT FROM 'string'
       OR NOT pg_input_is_valid(p ->> 'version_id', 'uuid') THEN
        RAISE EXCEPTION 'medication version-checked: version_id must be a uuid string';
    END IF;
    IF jsonb_typeof(p -> 'info_source') IS DISTINCT FROM 'string'
       OR btrim(p ->> 'info_source') = '' THEN
        RAISE EXCEPTION 'medication version-checked: info_source must be a non-blank string';
    END IF;
    -- A check is not a charted line: refused even as a JSON null.
    FOREACH k IN ARRAY ARRAY['charted_as', 'warning_override', 'restores'] LOOP
        IF p ? k THEN
            RAISE EXCEPTION 'medication version-checked: % must be absent (a check is not a medication line)', k;
        END IF;
    END LOOP;
    PERFORM cairn_check_medication_source_note(b);
    -- No §5.9 claim at BOTH doors (db/020 writes b->'safety' into event_log.safety and
    -- cairn_patient_safety reports any non-null one). A JSON null reads as absent.
    IF jsonb_typeof(b -> 'safety') IS NOT NULL AND jsonb_typeof(b -> 'safety') <> 'null' THEN
        RAISE EXCEPTION 'medication version-checked: a check carries no safety claim';
    END IF;

    -- ---- References: the authoring door only (ADR-0056) ----
    IF current_setting('cairn.remote_apply', true) = 'on' THEN
        RETURN;
    END IF;
    IF b ->> 'schema_version' IS DISTINCT FROM 'clinical.medication-set-aside-checked/1' THEN
        RAISE EXCEPTION 'medication version-checked: schema_version must be clinical.medication-set-aside-checked/1, got %', b ->> 'schema_version';
    END IF;
    -- A version submitted earlier in the same transaction is already projected, so it counts.
    IF NOT EXISTS (SELECT 1 FROM medication_set_aside
                    WHERE version_id = (p ->> 'version_id')::uuid
                      AND patient_id = (b ->> 'patient_id')::uuid) THEN
        RAISE EXCEPTION 'medication version-checked: version_id % is not a set-aside version on this chart', p ->> 'version_id';
    END IF;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_medication_version_checked(text, jsonb) FROM PUBLIC;

-- Register the floor + the hard twin requirement (#173). DO UPDATE converges on replay (#214).
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.medication-set-aside-checked.asserted', 'cairn_check_medication_version_checked',
     'medication version-checked requires a non-empty authored twin (§3.13/§3.3)')
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (cairn_event_twin_check.check_fn, cairn_event_twin_check.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);

-- The projection: one row per check event, keyed by its content address (what a shred scrubs by,
-- db/037). Columns are final: a widened CREATE TABLE never reaches an existing table.
CREATE TABLE IF NOT EXISTS medication_set_aside_checked (
    content_address BYTEA PRIMARY KEY,
    event_id        UUID NOT NULL,
    patient_id      UUID NOT NULL,
    version_id      UUID NOT NULL,
    info_source     TEXT NOT NULL,
    source_detail   TEXT,
    why             TEXT,
    t_recorded      TIMESTAMPTZ NOT NULL
);
GRANT SELECT ON medication_set_aside_checked TO cairn_agent;
CREATE INDEX IF NOT EXISTS medication_set_aside_checked_patient_idx ON medication_set_aside_checked (patient_id);
CREATE INDEX IF NOT EXISTS medication_set_aside_checked_version_idx ON medication_set_aside_checked (version_id);

-- The applier reads the CLEAR payload (ADR-0052): NULL = no custody here, nothing to project. A
-- shape it cannot represent is SKIPPED, never raised (a sealed peer body admitted without custody
-- never met the structural floor; a raise would fail the late-custody heal). Insert-or-nothing.
CREATE OR REPLACE FUNCTION medication_set_aside_checked_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := cairn_clear_payload(e);
BEGIN
    IF p IS NULL
       OR jsonb_typeof(p -> 'version_id') IS DISTINCT FROM 'string'
       OR NOT pg_input_is_valid(p ->> 'version_id', 'uuid')
       OR jsonb_typeof(p -> 'info_source') IS DISTINCT FROM 'string' THEN
        RETURN;
    END IF;
    INSERT INTO medication_set_aside_checked
        (content_address, event_id, patient_id, version_id, info_source, source_detail, why, t_recorded)
    VALUES (e.content_address, e.event_id, e.patient_id, (p ->> 'version_id')::uuid,
            p ->> 'info_source', p ->> 'source_detail', p ->> 'why',
            to_timestamp(e.hlc_wall / 1000.0))
    ON CONFLICT (content_address) DO NOTHING;
END;
$$;
REVOKE EXECUTE ON FUNCTION medication_set_aside_checked_apply(event_log) FROM PUBLIC;

INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('clinical.medication-set-aside-checked.asserted', 'medication_set_aside_checked_apply',
     ARRAY['medication_set_aside_checked'], 20, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
