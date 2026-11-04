-- db/065 — the progress-note DRAFT store (sign-in slice 3a, Task 3).
--
-- WHY: a note being typed is not yet a clinical fact — it may be abandoned, resumed after a
-- crash, or picked up on another terminal. So it lives NEXT TO the append-only record, in the
-- one deliberately MUTABLE table here, and becomes an event only when the clinician signs
-- (clinical.note.asserted, db/064). Nothing in note_draft is ever synced or signed.
--
-- HOW IT IS KEPT HONEST:
--   * The tables are reachable ONLY through SECURITY DEFINER doors (here and db/066); cairn_window
--     (db/062) holds EXECUTE on the doors and no table privilege at all.
--   * One draft belongs to one chart on one terminal ('elsewhere'/'moved' otherwise), so two screens
--     cannot silently overwrite each other's text.
--   * A spent draft can never come back: spending moves the row into note_draft_spent, and a
--     later save of that id answers 'spent'. A deferred constraint trigger refuses, at COMMIT,
--     a spend that names no clinical.note.asserted event of that chart INSERTED IN THE SAME
--     TRANSACTION (ruling R9) — so a draft cannot be "spent" without the note it became.
--   * Drafts hold the RAW text as typed (the window escapes markup only when it signs, ADR-0041);
--     the doors store and return it byte-for-byte.
--
-- SINCE SLICE 3b-1, db/066 holds the draft's TEXT and every draft door: a draft became a sequence
-- of writers' parts (note_draft_part), and 3a's three doors (cairn_note_draft_save,
-- cairn_note_drafts, cairn_note_draft_spend) were removed from this file in place — a door's
-- return shape is final, so the part-aware doors are new names. db/066 also migrates a 3a draft's
-- text into part 1 and drops the two columns marked below. What stays here: the header table,
-- the spend record and its commit-time trigger, and the two byline doors.
--
-- Replay-safe: this file re-runs on every schema replay.

CREATE TABLE IF NOT EXISTS note_draft (
    draft_id          uuid PRIMARY KEY,
    patient_id        uuid NOT NULL,
    -- Which terminal holds the draft; text because a terminal id is a site-local label.
    terminal_id       text NOT NULL CHECK (btrim(terminal_id) <> '' AND terminal_id !~ '[[:cntrl:]]'),
    -- narrative and stated_time: 3a's text, moved into note_draft_part and DROPPED by db/066.
    -- Still created here so a fresh database takes the same path an upgraded one did.
    narrative         text NOT NULL,
    stated_time       timestamptz,
    recording_started timestamptz NOT NULL,
    -- The latest REAL change over the draft's parts (see db/066's cairn_note_draft_part_save).
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

REVOKE EXECUTE ON FUNCTION cairn_note_signer_name(text), cairn_note_vouched_attester(uuid),
    note_draft_spent_names_its_note() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_note_signer_name(text), cairn_note_vouched_attester(uuid) TO cairn_window;
