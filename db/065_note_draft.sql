-- db/065 — the progress-note DRAFT store (sign-in slice 3a, Task 3).
--
-- WHY: a note being typed is not yet a clinical fact — it may be abandoned, resumed after a
-- crash, or picked up on another terminal. So it lives NEXT TO the append-only record, in the
-- one deliberately MUTABLE table here, and becomes an event only when the clinician signs
-- (clinical.note.asserted, db/064). Nothing in note_draft is ever synced or signed.
--
-- HOW IT IS KEPT HONEST:
--   * The tables are reachable ONLY through the SECURITY DEFINER doors below; cairn_window
--     (db/062) holds EXECUTE on the doors and no table privilege at all.
--   * One draft belongs to one chart on one terminal ('elsewhere' otherwise), so two screens
--     cannot silently overwrite each other's text.
--   * A spent draft can never come back: spending moves the row into note_draft_spent, and a
--     later save of that id answers 'spent'. A deferred constraint trigger refuses, at COMMIT,
--     a spend that names no real clinical.note.asserted event for that chart — so a draft
--     cannot be "spent" without a note actually existing.
--   * Drafts hold the RAW text as typed (the window escapes markup only when it signs, ADR-0041);
--     the doors store and return it byte-for-byte.
--
-- Replay-safe: this file re-runs on every schema replay.

CREATE TABLE IF NOT EXISTS note_draft (
    draft_id          uuid PRIMARY KEY,
    patient_id        uuid NOT NULL,
    -- Which terminal holds the draft; text because a terminal id is a site-local label.
    terminal_id       text NOT NULL CHECK (btrim(terminal_id) <> '' AND terminal_id !~ '[[:cntrl:]]'),
    narrative         text NOT NULL,
    -- The clinician's asserted time for the note, if any; NULL = not stated (principle 4).
    stated_time       timestamptz,
    recording_started timestamptz NOT NULL,
    -- The last REAL change (see cairn_note_draft_save); becomes the note's recording_ended.
    last_keystroke    timestamptz NOT NULL
);
CREATE INDEX IF NOT EXISTS note_draft_patient_idx ON note_draft (patient_id, recording_started);

CREATE TABLE IF NOT EXISTS note_draft_spent (
    draft_id   uuid PRIMARY KEY,
    -- Copied from the draft so the commit-time check never has to join to the deleted row.
    patient_id uuid NOT NULL,
    -- UNIQUE: one note event backs exactly one spend, so an existing note can never be reused
    -- to wipe other drafts on the chart.
    event_id   uuid NOT NULL UNIQUE,
    spent_at   timestamptz NOT NULL DEFAULT clock_timestamp()
);

-- Paired ALTER for a database created before UNIQUE existed (#207's rule). Constraint name is
-- the one Postgres gives an inline UNIQUE.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conrelid = 'note_draft_spent'::regclass
                    AND contype = 'u' AND conname = 'note_draft_spent_event_id_key') THEN
        ALTER TABLE note_draft_spent ADD CONSTRAINT note_draft_spent_event_id_key UNIQUE (event_id);
    END IF;
END $$;

REVOKE ALL ON note_draft, note_draft_spent FROM PUBLIC;

-- At COMMIT, a spent draft must be backed by its note event: right type, this patient's chart,
-- and INSERTED IN THIS SAME TRANSACTION (xmin = our xid) -- a spend is only ever the sign path's
-- own submit, so an older note (already committed) must not be able to justify wiping a draft.
-- Caveat: a row inserted inside a subtransaction carries the subxid, so the sign path must submit
-- the note without a savepoint around it.
-- SECURITY DEFINER with a pinned search_path and a qualified table: a caller must not be able to
-- shadow event_log with a TEMP table and fake the evidence. Deferred so the spend and the note's
-- submit can happen in either order inside one transaction.
CREATE OR REPLACE FUNCTION note_draft_spent_names_its_note() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM public.event_log e
                    WHERE e.event_id = NEW.event_id
                      AND e.event_type = 'clinical.note.asserted'
                      AND e.patient_id = NEW.patient_id
                      AND e.xmin = pg_current_xact_id()::xid) THEN
        RAISE EXCEPTION 'a note draft can only be spent by a clinical.note.asserted event of the same chart, submitted in the same transaction';
    END IF;
    RETURN NULL;
END;
$$;

DROP TRIGGER IF EXISTS note_draft_spent_names_its_note ON note_draft_spent;
CREATE CONSTRAINT TRIGGER note_draft_spent_names_its_note
    AFTER INSERT ON note_draft_spent
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION note_draft_spent_names_its_note();

-- Save (insert or update) a draft. Answers 'saved' | 'spent' | 'elsewhere' | 'no-chart'.
-- "The patient has a chart" means event_log holds at least one event with that patient_id.
-- last_keystroke moves only when the text or stated time really changed (ruling R1), so an
-- idle autosave tick does not drag recording_ended forward. A per-draft advisory lock
-- serialises save against spend (the spent-check and the insert must not interleave with one).
-- last_keystroke is never stamped BEFORE recording_started (final review I1): if the wall clock
-- steps back while a draft is open (an NTP step after a long offline spell, a manual fix), a
-- plain clock_timestamp() would give recording_ended < recording_started, which floor rule 8
-- (db/064) refuses -- and under R1 re-pressing Sign with unchanged text never restamps, so the
-- draft would stay unsignable. GREATEST keeps it a near-truth ("no earlier than writing began").
CREATE OR REPLACE FUNCTION cairn_note_draft_save(
    p_draft uuid, p_patient uuid, p_terminal text, p_narrative text, p_stated_time timestamptz)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v     note_draft%ROWTYPE;
    v_now timestamptz;
BEGIN
    -- A NULL is a caller bug, not an outcome: `<> NULL` is NULL, so it would slip past the
    -- 'elsewhere' guard below and overwrite another chart's or terminal's draft.
    IF p_draft IS NULL OR p_patient IS NULL OR p_terminal IS NULL THEN
        RAISE EXCEPTION 'cairn_note_draft_save needs a draft, a patient and a terminal';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('note_draft:' || p_draft::text, 0));
    IF EXISTS (SELECT 1 FROM note_draft_spent WHERE draft_id = p_draft) THEN
        RETURN 'spent';
    END IF;
    SELECT * INTO v FROM note_draft WHERE draft_id = p_draft FOR UPDATE;
    IF FOUND THEN
        IF v.patient_id IS DISTINCT FROM p_patient OR v.terminal_id IS DISTINCT FROM p_terminal THEN
            RETURN 'elsewhere';
        END IF;
        IF v.narrative IS DISTINCT FROM p_narrative OR v.stated_time IS DISTINCT FROM p_stated_time THEN
            UPDATE note_draft
               SET narrative = p_narrative, stated_time = p_stated_time,
                   last_keystroke = GREATEST(clock_timestamp(), v.recording_started)
             WHERE draft_id = p_draft;
        END IF;
        RETURN 'saved';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM event_log WHERE patient_id = p_patient) THEN
        RETURN 'no-chart';
    END IF;
    -- ONE clock reading for both, so a fresh draft starts with last_keystroke = recording_started.
    v_now := clock_timestamp();
    INSERT INTO note_draft (draft_id, patient_id, terminal_id, narrative, stated_time,
                            recording_started, last_keystroke)
    VALUES (p_draft, p_patient, p_terminal, p_narrative, p_stated_time, v_now, v_now);
    RETURN 'saved';
END;
$$;

-- A chart's open drafts, oldest first (for the "resume a draft" list).
CREATE OR REPLACE FUNCTION cairn_note_drafts(p_patient uuid)
RETURNS TABLE(draft_id uuid, terminal_id text, narrative text, stated_time timestamptz,
              recording_started timestamptz, last_keystroke timestamptz)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT d.draft_id, d.terminal_id, d.narrative, d.stated_time, d.recording_started, d.last_keystroke
      FROM note_draft d WHERE d.patient_id = p_patient ORDER BY d.recording_started, d.draft_id;
$$;

-- Spend a draft into note event p_event: lock and move the row, return it. Zero rows when the
-- draft is absent or already spent. The commit-time trigger then insists p_event is real.
CREATE OR REPLACE FUNCTION cairn_note_draft_spend(p_draft uuid, p_event uuid)
RETURNS TABLE(patient_id uuid, narrative text, stated_time timestamptz,
              recording_started timestamptz, last_keystroke timestamptz)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v note_draft%ROWTYPE;
BEGIN
    PERFORM pg_advisory_xact_lock(hashtextextended('note_draft:' || p_draft::text, 0));
    DELETE FROM note_draft d WHERE d.draft_id = p_draft RETURNING * INTO v;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    INSERT INTO note_draft_spent (draft_id, patient_id, event_id) VALUES (p_draft, v.patient_id, p_event);
    RETURN QUERY SELECT v.patient_id, v.narrative, v.stated_time, v.recording_started, v.last_keystroke;
END;
$$;

-- Who a signing key stands for, as a display string: the member's display name, else their
-- staff id; NULL for an unknown key. A definer wrapper because cairn_vault_key_holder (db/061)
-- is owner-only and the window must show "signed by ..." without reading vault_member.
CREATE OR REPLACE FUNCTION cairn_note_signer_name(p_key text) RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT COALESCE(h.display_name, h.staff_id) FROM cairn_vault_key_holder(p_key) h LIMIT 1;
$$;

-- Who VOUCHED for an event, as the hex key the contributor set names: event_log.attester_key,
-- but ONLY when one is stored AND cairn_attestation_vouched clears it (db/001: db/020's deferred
-- arm stores a peer's token UNVERIFIED, "carried, not vouched"); NULL otherwise. The note read
-- (cairn_node::note::read) grades authorship from this, and the window role may not call the
-- predicate itself (db/001 grants it to no runtime role), hence a definer door (ruling R11).
-- THE `attester_key IS NOT NULL` GUARD IS LOAD-BEARING: cairn_attestation_vouched is TRUE for an
-- event carrying no attestation at all ("vouched" = no unvouched marker row), exactly the trap
-- db/005's cairn_claim_authority header spells out for its R1 arm.
CREATE OR REPLACE FUNCTION cairn_note_vouched_attester(p_event uuid) RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT encode(e.attester_key, 'hex') FROM public.event_log e
     WHERE e.event_id = p_event
       AND e.attester_key IS NOT NULL
       AND cairn_attestation_vouched(e.event_id);
$$;

REVOKE EXECUTE ON FUNCTION cairn_note_draft_save(uuid, uuid, text, text, timestamptz),
    cairn_note_drafts(uuid), cairn_note_draft_spend(uuid, uuid), cairn_note_signer_name(text),
    cairn_note_vouched_attester(uuid), note_draft_spent_names_its_note() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_note_draft_save(uuid, uuid, text, text, timestamptz),
    cairn_note_drafts(uuid), cairn_note_draft_spend(uuid, uuid), cairn_note_signer_name(text),
    cairn_note_vouched_attester(uuid) TO cairn_window;
