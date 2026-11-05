-- db/070 — "Add to this note": a draft that names the signed note it adds to (sign-in slice 3c-1).
--
-- WHY: on paper you add to a note by writing under it, dated and signed, in the same folder; the
-- original is never touched. Here an addendum is an ordinary clinical.note.asserted whose refs
-- carry {rel: "addendum-to", event: <the note>} (no wire change). While it is being typed it is an
-- ordinary draft (db/065 + db/066) whose HEADER remembers the note: note_draft.addendum_to, created
-- in db/065 with its table. The window names the ROOT of the compound the clinician pressed on;
-- at Sign or at the timer's commit, the draft's first non-blank part carries that target.
--
-- THE TWO DOORS:
--   * cairn_note_draft_addendum_part_save: the save for an addendum draft. It owns only the HEADER
--     (is the target a note of this chart? is this draft already a different draft?) and then
--     hands the text to db/066's cairn_note_draft_part_save, so the writer check, the tail rule
--     and the revision row stay ONE rule in ONE place. A later plain save of the same draft (db/066's
--     door) keeps the target: no door ever updates addendum_to.
--   * cairn_note_draft_addendum_to: what a draft's header says it adds to, for Sign and the timer,
--     which read the target from the header, never from the window.
--
-- LOCK ORDER is every draft door's (db/066): the advisory lock 'note_draft:' || draft_id first,
-- then the header row FOR UPDATE. db/066's door takes the same advisory lock again inside this
-- one; a transaction-level advisory lock is re-entrant, so that second take returns at once.
--
-- THE FLOOR backs the door: db/064 rule 6a refuses, at the authoring door, an addendum-to target
-- that is not a clinical.note.asserted of the event's own chart, so a raw-SQL client that skips
-- this door still cannot sign an addendum to another patient's note.
--
-- Replay-safe: this file re-runs on every schema replay. Its return shapes are final.

-- Save writer p_writer's text into ADDENDUM draft p_draft of p_patient, held by p_terminal, adding
-- to note p_addendum_to. Answers db/066's words ('saved' | 'spent' | 'committed' | 'discarded' |
-- 'moved' | 'elsewhere' | 'no-such-writer') plus 'no-such-note': no header yet, and p_addendum_to
-- is not a note this node shows on this chart. Every answer but 'saved' wrote nothing.
--
-- HOW, in order: refuse NULLs (a caller bug); take the draft's lock; a spent draft answers its
-- word. With NO header yet: the target must be a projected note of this chart ('no-such-note');
-- a blank text makes no header at all (a click in and out writes nothing, as db/066's empty
-- new-writer rule); otherwise make the header WITH its target. With a header: it must name the
-- same target ('elsewhere' otherwise: a new note's draft never becomes an addendum, nor one
-- addendum another's). Then db/066's door does the rest. If it refuses a header made by THIS call
-- (only 'no-such-writer' can), the header goes too, so the refusal wrote nothing.
CREATE OR REPLACE FUNCTION cairn_note_draft_addendum_part_save(
    p_draft uuid, p_patient uuid, p_terminal text, p_writer text, p_narrative text,
    p_stated_time timestamptz, p_addendum_to uuid)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_head   note_draft%ROWTYPE;
    v_spent  text;
    v_now    timestamptz;
    v_made   boolean := false;
    v_answer text;
BEGIN
    -- A NULL is a caller bug, not an outcome (db/066's reason: `<> NULL` slips past every guard).
    -- p_writer NULL = "Don't know"; p_stated_time NULL = not stated.
    IF p_draft IS NULL OR p_patient IS NULL OR p_terminal IS NULL OR p_narrative IS NULL
       OR p_addendum_to IS NULL THEN
        RAISE EXCEPTION 'cairn_note_draft_addendum_part_save needs a draft, a patient, a terminal, a narrative and the note it adds to';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('note_draft:' || p_draft::text, 0));
    -- The same words as db/066's doors, so the window handles a spent addendum draft as any other.
    v_spent := (SELECT CASE WHEN s.discarded_by IS NOT NULL THEN 'discarded'
                            WHEN s.by_timer THEN 'committed' ELSE 'spent' END
                  FROM note_draft_spent s WHERE s.draft_id = p_draft);
    IF v_spent IS NOT NULL THEN
        RETURN v_spent;
    END IF;
    SELECT * INTO v_head FROM note_draft WHERE draft_id = p_draft FOR UPDATE;
    IF NOT FOUND THEN
        -- The projection, not event_log: what this node SHOWS as a note of this chart (a peer's
        -- newer-profile note is carried but not projected, ruling R8, so it is never a target).
        IF NOT EXISTS (SELECT 1 FROM clinical_note n
                        WHERE n.event_id = p_addendum_to AND n.patient_id = p_patient) THEN
            RETURN 'no-such-note';
        END IF;
        -- The floor's one blank test (db/005), as db/066's door uses.
        IF NOT cairn_twin_is_present(p_narrative) THEN
            RETURN 'saved';
        END IF;
        v_now := clock_timestamp();
        INSERT INTO note_draft (draft_id, patient_id, terminal_id, recording_started, last_keystroke, addendum_to)
        VALUES (p_draft, p_patient, p_terminal, v_now, v_now, p_addendum_to);
        v_made := true;
    ELSIF v_head.addendum_to IS DISTINCT FROM p_addendum_to THEN
        RETURN 'elsewhere';
    END IF;
    v_answer := cairn_note_draft_part_save(p_draft, p_patient, p_terminal, p_writer, p_narrative, p_stated_time);
    IF v_made AND v_answer <> 'saved' THEN
        DELETE FROM note_draft WHERE draft_id = p_draft;
    END IF;
    RETURN v_answer;
END;
$$;

-- The note draft p_draft adds to: its header's addendum_to. NULL for a new note's draft, and for
-- no such draft (never made, or spent: a spend deletes the header). The column is never updated,
-- so no lock is needed to read it.
CREATE OR REPLACE FUNCTION cairn_note_draft_addendum_to(p_draft uuid)
RETURNS uuid LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT d.addendum_to FROM note_draft d WHERE d.draft_id = p_draft;
$$;

REVOKE EXECUTE ON FUNCTION cairn_note_draft_addendum_part_save(uuid, uuid, text, text, text, timestamptz, uuid),
    cairn_note_draft_addendum_to(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_note_draft_addendum_part_save(uuid, uuid, text, text, text, timestamptz, uuid),
    cairn_note_draft_addendum_to(uuid) TO cairn_window;
