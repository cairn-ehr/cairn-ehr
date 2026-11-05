-- db/066 — a note draft is a sequence of writers' PARTS (sign-in slice 3b-1, Task 1).
--
-- WHY: on a ward several people write into one note before one of them signs it — the nurse's
-- observations, then the doctor's plan below them. On paper each hand is plainly its own, and
-- nobody can rub out what a colleague wrote above. So a draft (db/065's `note_draft`, now only
-- the HEADER: chart, terminal, times) holds an ordered list of parts, one writer per part:
--   * a save rewrites the TAIL part only when the tail is the saver's own;
--   * any other save APPENDS a new part (nurse A, Dr B, nurse A again = three parts);
--   * no door takes a part number, so no call can address, change or remove another's part.
--     (Shortcut, stated in the plan's global constraints: a writer's own EARLIER part is
--     read-only too, once a colleague has written below it. Upgrade if clinicians ask.)
-- Every real change also leaves a revision row (who, where, what, when) for the draft's life.
--
-- The writer is the staff id of a vault member whose role may act clinically (db/061; never a
-- superuser or an admin, §5.1), normalised with cairn_vault_staff_id, or NULL for an unknown
-- writer ("Don't know" is an allowed answer to who's here, design §3.1). Anything else answers
-- 'no-such-writer'. "Empty" text is the floor's own blank test, cairn_twin_is_present (db/005).
-- "Who's here" is a claim, never authentication: nothing here signs.
--
-- MOVING A DRAFT. "Continue" at another terminal moves the draft there (cairn_note_draft_take,
-- one statement). The old terminal's next save then answers 'moved' — the draft is still on this
-- chart, another terminal holds it — distinct from 'elsewhere', the draft is another CHART's (#958).
--
-- 3a's three draft doors are REMOVED (here and from db/065 in place), never reshaped: a door's
-- return shape is final, and a `CREATE OR REPLACE` of a same-named function with a new shape would
-- make db/065's replay fail on every existing database. The part-aware doors take new names.
--
-- KEPT FROM 3a, exactly: one advisory lock key 'note_draft:' || draft_id on every door touching a
-- draft (and in this order: advisory lock, then the header row, then its parts); the spent-check
-- before any write; last_keystroke moves only on a REAL change (ruling R1) and is never stamped
-- before its recording_started (final review I1); a NULL draft/patient/terminal raises; the
-- tables are reachable only through these definer doors; db/065's deferred trigger is unchanged.
--
-- Replay-safe: this file re-runs on every schema replay.

CREATE TABLE IF NOT EXISTS note_draft_part (
    draft_id          uuid NOT NULL REFERENCES note_draft ON DELETE CASCADE,
    -- 1, 2, 3 … in writing order. The first non-blank part becomes the note; each later non-blank
    -- part an addendum to the non-blank part before it (a blank part is never signed).
    seq               int  NOT NULL CHECK (seq >= 1),
    -- The vault staff id (normalised), or NULL = the writer is not known.
    writer            text,
    -- The RAW text as typed (the window escapes markup only when it signs, ADR-0041).
    narrative         text NOT NULL,
    -- The writer's asserted time for this part; NULL = not stated (principle 4).
    stated_time       timestamptz,
    recording_started timestamptz NOT NULL,
    -- This part's last REAL change; becomes its note's recording_ended.
    last_keystroke    timestamptz NOT NULL,
    PRIMARY KEY (draft_id, seq)
);

-- One row per real change: what was saved, by whom, at which terminal. Cascades with its draft
-- (and the spend deletes them explicitly), so no revision outlives the draft it records.
CREATE TABLE IF NOT EXISTS note_draft_revision (
    draft_id    uuid NOT NULL REFERENCES note_draft ON DELETE CASCADE,
    seq         int  NOT NULL,
    rev         bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    writer      text,
    terminal_id text NOT NULL,
    narrative   text NOT NULL,
    stated_time timestamptz,
    saved_at    timestamptz NOT NULL
);
CREATE INDEX IF NOT EXISTS note_draft_revision_draft_idx ON note_draft_revision (draft_id);

REVOKE ALL ON note_draft_part, note_draft_revision FROM PUBLIC;

-- MIGRATION from 3a's single-text draft. Runs only while note_draft still has its 3a `narrative`
-- column: every header without parts becomes part 1 by an unknown (NULL) writer — 3a never knew
-- who typed — with its text, stated time and recording times as they were. Then the two columns
-- go. Dynamic SQL, because once they are dropped a static reference would not even plan.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
                WHERE table_schema = 'public' AND table_name = 'note_draft' AND column_name = 'narrative') THEN
        EXECUTE 'INSERT INTO note_draft_part (draft_id, seq, writer, narrative, stated_time,
                                              recording_started, last_keystroke)
                 SELECT d.draft_id, 1, NULL, d.narrative, d.stated_time, d.recording_started, d.last_keystroke
                   FROM note_draft d
                  WHERE NOT EXISTS (SELECT 1 FROM note_draft_part p WHERE p.draft_id = d.draft_id)';
        ALTER TABLE note_draft DROP COLUMN IF EXISTS narrative, DROP COLUMN IF EXISTS stated_time;
    END IF;
END $$;

DROP FUNCTION IF EXISTS cairn_note_draft_save(uuid, uuid, text, text, timestamptz);
DROP FUNCTION IF EXISTS cairn_note_drafts(uuid);
DROP FUNCTION IF EXISTS cairn_note_draft_spend(uuid, uuid);

-- Save writer p_writer's text into draft p_draft of p_patient, held by p_terminal. Answers
-- 'saved' | 'spent' (signed) | 'committed' (the quarantine timer put it in unsigned, db/067) |
-- 'discarded' (thrown away with an audit record, db/069) |
-- 'moved' (this chart, another terminal holds it) | 'elsewhere' (another chart's) | 'no-chart'
-- (no event names the patient) | 'no-such-writer'. Every answer but 'saved' wrote nothing.
--
-- HOW, in order: refuse NULLs; normalise the writer and check they are an enrolled clinical
-- member (a plain read of vault_member, taking no lock, so it cannot deadlock with the lock order
-- below); take the
-- draft's advisory lock; a spent draft answers 'spent', 'committed' or 'discarded'; an existing draft must be
-- this chart's and this terminal's. Then the TAIL part (highest seq) decides: the saver's own tail is rewritten
-- if the text or stated time changed; otherwise a non-empty text appends part tail+1 (an empty one
-- makes no part — a colleague clicking in and out writes nothing). A new draft always gets part 1.
-- Every write stamps the part's and the header's last_keystroke and leaves a revision row.
CREATE OR REPLACE FUNCTION cairn_note_draft_part_save(
    p_draft uuid, p_patient uuid, p_terminal text, p_writer text, p_narrative text,
    p_stated_time timestamptz)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_writer text := cairn_vault_staff_id(p_writer);
    v_head   note_draft%ROWTYPE;
    v_tail   note_draft_part%ROWTYPE;
    v_seq    int;
    v_now    timestamptz;
    v_spent  text;
BEGIN
    -- A NULL is a caller bug, not an outcome: `<> NULL` is NULL, so it would slip past the
    -- 'moved'/'elsewhere' guards below and write into another chart's or terminal's draft.
    IF p_draft IS NULL OR p_patient IS NULL OR p_terminal IS NULL THEN
        RAISE EXCEPTION 'cairn_note_draft_part_save needs a draft, a patient and a terminal';
    END IF;
    -- A writer must be a member whose role may act clinically (db/061): a superuser or an admin
    -- confers no clinical authorship (§5.1), so is never a part's writer — the same rule the
    -- directory below lists by, held here too so a raw-SQL client cannot record one.
    IF v_writer IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM vault_member m
         WHERE m.staff_id = v_writer AND cairn_vault_purpose_allowed(m.role, false, 'clinical')) THEN
        RETURN 'no-such-writer';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('note_draft:' || p_draft::text, 0));
    -- A timer's commit is never this window's Sign, so it gets its own word (db/067).
    v_spent := (SELECT CASE WHEN s.discarded_by IS NOT NULL THEN 'discarded'  -- db/069
                            WHEN s.by_timer THEN 'committed' ELSE 'spent' END
                  FROM note_draft_spent s WHERE s.draft_id = p_draft);
    IF v_spent IS NOT NULL THEN
        RETURN v_spent;
    END IF;
    -- ONE clock reading per call, so a new draft's header and part 1 start at the same instant.
    -- (An addendum's header is made by db/070's door with its own reading, just before this
    -- call, so there part 1 starts a moment after the header.)
    v_now := clock_timestamp();
    SELECT * INTO v_head FROM note_draft WHERE draft_id = p_draft FOR UPDATE;
    IF NOT FOUND THEN
        IF NOT EXISTS (SELECT 1 FROM event_log WHERE patient_id = p_patient) THEN
            RETURN 'no-chart';
        END IF;
        INSERT INTO note_draft (draft_id, patient_id, terminal_id, recording_started, last_keystroke)
        VALUES (p_draft, p_patient, p_terminal, v_now, v_now);
        v_seq := 1;
        INSERT INTO note_draft_part (draft_id, seq, writer, narrative, stated_time, recording_started, last_keystroke)
        VALUES (p_draft, v_seq, v_writer, p_narrative, p_stated_time, v_now, v_now);
    ELSE
        IF v_head.patient_id IS DISTINCT FROM p_patient THEN
            RETURN 'elsewhere';
        END IF;
        IF v_head.terminal_id IS DISTINCT FROM p_terminal THEN
            RETURN 'moved';
        END IF;
        SELECT * INTO v_tail FROM note_draft_part WHERE draft_id = p_draft ORDER BY seq DESC LIMIT 1 FOR UPDATE;
        IF FOUND AND v_tail.writer IS NOT DISTINCT FROM v_writer THEN
            IF v_tail.narrative IS NOT DISTINCT FROM p_narrative
               AND v_tail.stated_time IS NOT DISTINCT FROM p_stated_time THEN
                RETURN 'saved';  -- R1: an idle re-save is not a keystroke
            END IF;
            v_seq := v_tail.seq;
            UPDATE note_draft_part
               SET narrative = p_narrative, stated_time = p_stated_time,
                   last_keystroke = GREATEST(v_now, v_tail.recording_started)
             WHERE draft_id = p_draft AND seq = v_seq;
        ELSE
            -- "Empty" is the floor's ONE blank test (db/005; rule 2 of db/064 refuses a blank
            -- note with it): locale-free, so every node and Rust's trim() agree. A regex class
            -- would read a lone NBSP as text under COLLATE "C" and make the draft unsignable.
            IF NOT cairn_twin_is_present(p_narrative) THEN
                RETURN 'saved';  -- nothing typed by a new writer: no part
            END IF;
            v_seq := COALESCE(v_tail.seq, 0) + 1;
            INSERT INTO note_draft_part (draft_id, seq, writer, narrative, stated_time, recording_started, last_keystroke)
            VALUES (p_draft, v_seq, v_writer, p_narrative, p_stated_time, v_now, v_now);
        END IF;
        UPDATE note_draft SET last_keystroke = GREATEST(v_now, v_head.recording_started)
         WHERE draft_id = p_draft;
    END IF;
    INSERT INTO note_draft_revision (draft_id, seq, writer, terminal_id, narrative, stated_time, saved_at)
    VALUES (p_draft, v_seq, v_writer, p_terminal, p_narrative, p_stated_time, v_now);
    RETURN 'saved';
END;
$$;

-- A chart's open drafts, one row per PART: drafts oldest first, parts in writing order.
-- writer_name is the member's display name, else their staff id; NULL for an unknown writer.
CREATE OR REPLACE FUNCTION cairn_note_draft_parts(p_patient uuid)
RETURNS TABLE(draft_id uuid, terminal_id text, seq int, writer text, writer_name text,
              narrative text, stated_time timestamptz, recording_started timestamptz,
              last_keystroke timestamptz)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT d.draft_id, d.terminal_id, p.seq, p.writer,
           CASE WHEN p.writer IS NOT NULL THEN COALESCE(m.display_name, p.writer) END,
           p.narrative, p.stated_time, p.recording_started, p.last_keystroke
      FROM note_draft d
      JOIN note_draft_part p ON p.draft_id = d.draft_id
      LEFT JOIN vault_member m ON m.staff_id = p.writer
     WHERE d.patient_id = p_patient
     ORDER BY d.recording_started, d.draft_id, p.seq;
$$;

-- Move draft p_draft of p_patient to terminal p_terminal ("Continue" at another terminal).
-- Answers 'taken' | 'spent' (signed) | 'committed' (the quarantine timer's, db/067) |
-- 'discarded' (db/069) | 'elsewhere'
-- (another chart's) | 'gone' (no such draft). Only note_draft.terminal_id changes: no text, no
-- time, no part. Taking a draft already held here answers 'taken'.
CREATE OR REPLACE FUNCTION cairn_note_draft_take(p_draft uuid, p_patient uuid, p_terminal text)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_patient uuid;
    v_spent   text;
BEGIN
    IF p_draft IS NULL OR p_patient IS NULL OR p_terminal IS NULL THEN
        RAISE EXCEPTION 'cairn_note_draft_take needs a draft, a patient and a terminal';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('note_draft:' || p_draft::text, 0));
    -- A timer's commit is never this window's Sign, so it gets its own word (db/067).
    v_spent := (SELECT CASE WHEN s.discarded_by IS NOT NULL THEN 'discarded'  -- db/069
                            WHEN s.by_timer THEN 'committed' ELSE 'spent' END
                  FROM note_draft_spent s WHERE s.draft_id = p_draft);
    IF v_spent IS NOT NULL THEN
        RETURN v_spent;
    END IF;
    SELECT d.patient_id INTO v_patient FROM note_draft d WHERE d.draft_id = p_draft FOR UPDATE;
    IF NOT FOUND THEN
        RETURN 'gone';
    END IF;
    IF v_patient IS DISTINCT FROM p_patient THEN
        RETURN 'elsewhere';
    END IF;
    UPDATE note_draft SET terminal_id = p_terminal WHERE draft_id = p_draft;
    RETURN 'taken';
END;
$$;

-- Spend draft p_draft into note event p_event (the first non-blank part's event, when the note
-- has several): return its NON-empty parts in writing order, then delete the header (its parts
-- cascade) and its revisions, and record the spend. Zero rows means NOTHING was spent: the draft
-- is absent, already spent, or every part is empty (there is nothing to sign). db/065's commit-time trigger then
-- insists p_event is a note of this chart inserted in this same transaction.
CREATE OR REPLACE FUNCTION cairn_note_draft_spend_parts(p_draft uuid, p_event uuid)
RETURNS TABLE(patient_id uuid, seq int, writer text, narrative text, stated_time timestamptz,
              recording_started timestamptz, last_keystroke timestamptz)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_patient uuid;
BEGIN
    PERFORM pg_advisory_xact_lock(hashtextextended('note_draft:' || p_draft::text, 0));
    SELECT d.patient_id INTO v_patient FROM note_draft d WHERE d.draft_id = p_draft FOR UPDATE;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    -- RETURN QUERY buffers these rows now, so the deletes below cannot take them away.
    RETURN QUERY
        SELECT v_patient, p.seq, p.writer, p.narrative, p.stated_time, p.recording_started, p.last_keystroke
          FROM note_draft_part p
         WHERE p.draft_id = p_draft AND cairn_twin_is_present(p.narrative)  -- the floor's blank test
         ORDER BY p.seq;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM note_draft_revision r WHERE r.draft_id = p_draft;
    DELETE FROM note_draft d WHERE d.draft_id = p_draft;
    INSERT INTO note_draft_spent (draft_id, patient_id, event_id) VALUES (p_draft, v_patient, p_event);
END;
$$;

-- Who may be named as here: every member whose role may act clinically (cairn_vault_purpose_allowed,
-- db/061) — so never a superuser or an admin, who confer no clinical authorship (§5.1). A definer
-- door because vault_member has no grant at all; it returns no key and no sealed blob.
CREATE OR REPLACE FUNCTION cairn_whos_here_directory()
RETURNS TABLE(staff_id text, display_name text, role text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT m.staff_id, m.display_name, m.role FROM vault_member m
     WHERE cairn_vault_purpose_allowed(m.role, false, 'clinical')
     ORDER BY m.display_name, m.staff_id;
$$;

REVOKE EXECUTE ON FUNCTION cairn_note_draft_part_save(uuid, uuid, text, text, text, timestamptz),
    cairn_note_draft_parts(uuid), cairn_note_draft_take(uuid, uuid, text),
    cairn_note_draft_spend_parts(uuid, uuid), cairn_whos_here_directory() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_note_draft_part_save(uuid, uuid, text, text, text, timestamptz),
    cairn_note_draft_parts(uuid), cairn_note_draft_take(uuid, uuid, text),
    cairn_note_draft_spend_parts(uuid, uuid), cairn_whos_here_directory() TO cairn_window;
