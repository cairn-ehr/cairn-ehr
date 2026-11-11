-- db/078 — the per-writer MEDICATION DRAFT LIST store (batch charting slice 1, #1072, Task 1).
--
-- WHAT: a clinician types several medication lines into one durable, UNSIGNED list for a chart;
-- a later single Sign charts the ticked ones as ordinary clinical.medication.asserted events.
-- This file is only the list's storage and its doors. Nothing here signs or commits anything.
--
-- WHY IT IS MUTABLE: a line being typed is not yet a clinical fact. It may be reworded, ticked,
-- abandoned, or picked up on another terminal after a crash. So, like the note draft (db/065),
-- it lives NEXT TO the append-only record in a deliberately MUTABLE table, and becomes an event
-- only when the clinician signs. Nothing in medication_draft_line is ever synced or signed, and
-- it is in no backup or export (those read named tables, and this is not one).
--
-- WHY IT NEVER COMMITS ITSELF: unlike a note draft there is no quarantine timer. A timer that
-- charted unsigned medications on someone's behalf would put words in a prescriber's mouth; an
-- unsigned list simply waits (the sign-in's responsibility baseline, ADR-0053).
--
-- WHY DELETING IS UNAUDITED: a deleted draft line was never an assertion by anyone, so the
-- delete is a plain DELETE with no record. What the clinician SIGNED is in the event log;
-- what they merely typed and removed is paper that went into the bin.
--
-- HOW IT IS KEPT HONEST:
--   * Reachable ONLY through the SECURITY DEFINER doors below; cairn_window (db/062) holds
--     EXECUTE on the doors and no table privilege at all.
--   * One line belongs to one chart on one terminal ('elsewhere' / 'moved'), and to one writer
--     ('not-yours'), so two screens cannot silently overwrite each other.
--   * A named writer must be a vault member whose role may act clinically (db/066's test).
--     writer NULL = "not known", its own bucket (principle 4: never force a name).
--   * A HAND TICK is the clinician's attestation that they saw the listed warnings and still
--     want the line. It is bound to the line's content: if the text, the coded parts or the
--     charted-as changed, the tick is cleared HERE, in the door, so a raw-SQL client cannot keep
--     a stale tick on edited text.
--   * Lines hold the RAW text as typed; the doors store and return it byte-for-byte.
--
-- Replay-safe: this file re-runs on every schema replay.

CREATE TABLE IF NOT EXISTS medication_draft_line (
    line_id        uuid PRIMARY KEY,
    patient_id     uuid NOT NULL,
    -- Normalised staff id (cairn_vault_staff_id); NULL = not known.
    writer         text,
    -- Which terminal holds the line; text because a terminal id is a site-local label.
    terminal_id    text NOT NULL CHECK (btrim(terminal_id) <> '' AND terminal_id !~ '[[:cntrl:]]'),
    -- Display order within (patient, writer); assigned on insert only.
    seq            int  NOT NULL,
    charted_as     text NOT NULL CHECK (charted_as IN ('started-here', 'pre-existing')),
    -- The whole line, raw, as typed.
    line_text      text NOT NULL,
    -- [{term, coding: {system, code, display} | null}]
    parts          jsonb NOT NULL DEFAULT '[]',
    reason         text,
    hand_ticked    boolean NOT NULL DEFAULT false,
    -- [{source, ref}] the warnings seen when hand-ticked.
    ticked_against jsonb NOT NULL DEFAULT '[]',
    finished       boolean NOT NULL DEFAULT false,
    created_at     timestamptz NOT NULL,
    saved_at       timestamptz NOT NULL
);
CREATE INDEX IF NOT EXISTS medication_draft_line_owner_idx
    ON medication_draft_line (patient_id, writer, seq);

REVOKE ALL ON medication_draft_line FROM PUBLIC;

-- Save (insert or update) one line. Answers 'saved' | 'moved' | 'elsewhere' | 'no-chart' |
-- 'no-such-writer' | 'not-yours'; every answer but 'saved' wrote nothing. NULL line, patient or
-- terminal is a caller bug and raises (`<> NULL` is NULL and would slip past the guards).
CREATE OR REPLACE FUNCTION cairn_med_draft_line_save(
    p_line uuid, p_patient uuid, p_terminal text, p_writer text, p_charted_as text,
    p_text text, p_parts jsonb, p_reason text, p_hand_ticked boolean, p_ticked_against jsonb,
    p_finished boolean)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_writer  text := cairn_vault_staff_id(p_writer);
    v_row     medication_draft_line%ROWTYPE;
    v_changed boolean;
    v_now     timestamptz;
BEGIN
    IF p_line IS NULL OR p_patient IS NULL OR p_terminal IS NULL THEN
        RAISE EXCEPTION 'cairn_med_draft_line_save needs a line, a patient and a terminal';
    END IF;
    IF v_writer IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM vault_member m
         WHERE m.staff_id = v_writer AND cairn_vault_purpose_allowed(m.role, false, 'clinical')) THEN
        RETURN 'no-such-writer';
    END IF;
    -- One lock per (chart, writer) list: serialises seq assignment and edits to the same list.
    PERFORM pg_advisory_xact_lock(hashtextextended(
        'med_draft:' || p_patient::text || ':' || coalesce(v_writer, ''), 0));
    v_now := clock_timestamp();
    SELECT * INTO v_row FROM medication_draft_line WHERE line_id = p_line FOR UPDATE;
    IF FOUND THEN
        IF v_row.patient_id IS DISTINCT FROM p_patient THEN RETURN 'elsewhere'; END IF;
        IF v_row.terminal_id IS DISTINCT FROM p_terminal THEN RETURN 'moved'; END IF;
        IF v_row.writer IS DISTINCT FROM v_writer THEN RETURN 'not-yours'; END IF;
        -- A tick vouches for THIS content; changed content voids it, whatever the caller sent.
        v_changed := v_row.line_text IS DISTINCT FROM p_text
                  OR v_row.parts IS DISTINCT FROM coalesce(p_parts, '[]'::jsonb)
                  OR v_row.charted_as IS DISTINCT FROM p_charted_as;
        UPDATE medication_draft_line
           SET charted_as = p_charted_as, line_text = p_text,
               parts = coalesce(p_parts, '[]'::jsonb), reason = p_reason,
               hand_ticked = coalesce(p_hand_ticked, false) AND NOT v_changed,
               ticked_against = CASE WHEN v_changed THEN '[]'::jsonb
                                     ELSE coalesce(p_ticked_against, '[]'::jsonb) END,
               finished = coalesce(p_finished, false), saved_at = v_now
         WHERE line_id = p_line;
        RETURN 'saved';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM event_log WHERE patient_id = p_patient) THEN
        RETURN 'no-chart';
    END IF;
    INSERT INTO medication_draft_line (line_id, patient_id, writer, terminal_id, seq, charted_as,
        line_text, parts, reason, hand_ticked, ticked_against, finished, created_at, saved_at)
    SELECT p_line, p_patient, v_writer, p_terminal,
           coalesce(max(l.seq), 0) + 1, p_charted_as, p_text, coalesce(p_parts, '[]'::jsonb),
           p_reason, coalesce(p_hand_ticked, false), coalesce(p_ticked_against, '[]'::jsonb),
           coalesce(p_finished, false), v_now, v_now
      FROM medication_draft_line l
     WHERE l.patient_id = p_patient AND l.writer IS NOT DISTINCT FROM v_writer;
    RETURN 'saved';
END;
$$;

-- The writer's lines for one chart, in display order. IS NOT DISTINCT FROM so a NULL writer
-- reads exactly the NULL bucket.
CREATE OR REPLACE FUNCTION cairn_med_draft_lines(p_patient uuid, p_writer text)
RETURNS TABLE(line_id uuid, terminal_id text, seq int, charted_as text, line_text text,
              parts jsonb, reason text, hand_ticked boolean, ticked_against jsonb,
              finished boolean, saved_at timestamptz)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT l.line_id, l.terminal_id, l.seq, l.charted_as, l.line_text, l.parts, l.reason,
           l.hand_ticked, l.ticked_against, l.finished, l.saved_at
      FROM medication_draft_line l
     WHERE l.patient_id = p_patient AND l.writer IS NOT DISTINCT FROM cairn_vault_staff_id(p_writer)
     ORDER BY l.seq;
$$;

-- Delete one line (unaudited, see the header). 'deleted' | 'gone' | 'moved' | 'elsewhere'.
CREATE OR REPLACE FUNCTION cairn_med_draft_line_delete(p_line uuid, p_patient uuid, p_terminal text)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_row medication_draft_line%ROWTYPE;
BEGIN
    IF p_line IS NULL OR p_patient IS NULL OR p_terminal IS NULL THEN
        RAISE EXCEPTION 'cairn_med_draft_line_delete needs a line, a patient and a terminal';
    END IF;
    SELECT * INTO v_row FROM medication_draft_line WHERE line_id = p_line FOR UPDATE;
    IF NOT FOUND THEN RETURN 'gone'; END IF;
    IF v_row.patient_id IS DISTINCT FROM p_patient THEN RETURN 'elsewhere'; END IF;
    IF v_row.terminal_id IS DISTINCT FROM p_terminal THEN RETURN 'moved'; END IF;
    DELETE FROM medication_draft_line WHERE line_id = p_line;
    RETURN 'deleted';
END;
$$;

-- "Continue here": move every line of this writer's list on this chart to p_terminal, in one
-- statement. Returns how many lines changed terminal (0 when they were already here).
CREATE OR REPLACE FUNCTION cairn_med_draft_take(p_patient uuid, p_writer text, p_terminal text)
RETURNS int LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_writer text := cairn_vault_staff_id(p_writer);
    v_n      int;
BEGIN
    IF p_patient IS NULL OR p_terminal IS NULL THEN
        RAISE EXCEPTION 'cairn_med_draft_take needs a patient and a terminal';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended(
        'med_draft:' || p_patient::text || ':' || coalesce(v_writer, ''), 0));
    UPDATE medication_draft_line
       SET terminal_id = p_terminal
     WHERE patient_id = p_patient AND writer IS NOT DISTINCT FROM v_writer
       AND terminal_id IS DISTINCT FROM p_terminal;
    GET DIAGNOSTICS v_n = ROW_COUNT;
    RETURN v_n;
END;
$$;

-- Who has unsigned medication lines waiting, per chart: named writers only (an unnamed list has
-- no one to show it to). Feeds the person menu.
CREATE OR REPLACE FUNCTION cairn_med_drafts_by_writer()
RETURNS TABLE(writer text, patient_id uuid, lines bigint, newest timestamptz)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT l.writer, l.patient_id, count(*)::bigint, max(l.saved_at)
      FROM medication_draft_line l
     WHERE l.writer IS NOT NULL
     GROUP BY l.writer, l.patient_id
     ORDER BY 4 DESC, 1, 2;
$$;

REVOKE EXECUTE ON FUNCTION
    cairn_med_draft_line_save(uuid, uuid, text, text, text, text, jsonb, text, boolean, jsonb, boolean),
    cairn_med_draft_lines(uuid, text), cairn_med_draft_line_delete(uuid, uuid, text),
    cairn_med_draft_take(uuid, text, text), cairn_med_drafts_by_writer() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION
    cairn_med_draft_line_save(uuid, uuid, text, text, text, text, jsonb, text, boolean, jsonb, boolean),
    cairn_med_draft_lines(uuid, text), cairn_med_draft_line_delete(uuid, uuid, text),
    cairn_med_draft_take(uuid, text, text), cairn_med_drafts_by_writer() TO cairn_window;
