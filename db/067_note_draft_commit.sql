-- db/067 — the quarantine policy and the overdue-commit door (sign-in slice 3b-2a, Task 1).
--
-- WHY: on paper an entry left unsigned on the progress sheet is simply in the record: it stays in
-- the folder, readable by anyone, plainly not signed. A Cairn draft (db/065 + db/066) is not in
-- the record until someone signs it. So a draft nobody has typed into for the site's QUARANTINE
-- time (default 3 hours) goes in by itself, unsigned: the window's runner asks this file's doors
-- which drafts are overdue, builds one node-signed `clinical.note.asserted` per non-blank part,
-- and spends the draft through cairn_note_draft_commit_overdue in the same transaction.
--
-- WHY THE CLOCK IS THE DATABASE'S (design §3.3). A draft has one home, the database it lives in,
-- and that database already stamps every keystroke save (db/066's last_keystroke). Comparing
-- against the SAME clock means no terminal clock ever enters the decision: the window never sends
-- a time. A step in that clock loses nothing: forward only commits early (still honestly
-- unsigned), backward only delays. The quarantine is operational only; it never touches the
-- stated time, t_effective or t_recorded.
--
-- WHY THE DOOR, NOT THE CALLER, DECIDES "OVERDUE". cairn_note_drafts_overdue is only a hint (it
-- takes no lock). The commit door re-decides under the draft's advisory lock and consumes the
-- draft in the same call, so two windows ticking at once, or a Sign racing the timer, can never
-- put two sets of notes in the log, and a raw-SQL client cannot commit someone's draft early.
-- Lock order is every draft door's (db/066): advisory lock 'note_draft:' || draft_id, then the
-- header row, then its parts.
--
-- WHAT A TIMER'S SPEND LEAVES. The same note_draft_spent row a Sign leaves, with by_timer = true
-- (the column lives in db/065, because db/066's save and take doors read it to answer 'committed'
-- rather than 'spent'). db/065's commit-time trigger is unchanged: the spend must name a
-- clinical.note.asserted of this chart inserted in this same transaction. An overdue draft whose
-- every part is blank is not a note at all (design §3.5): it is deleted, and no spend is recorded.
--
-- THE POLICY is one row, like db/061's vault_policy: custody state, not the record, so in no
-- backup (a restore comes back with the default), and set only at the console (no runtime role
-- holds the setter).
--
-- Replay-safe: this file re-runs on every schema replay.

CREATE TABLE IF NOT EXISTS note_policy (
    singleton          boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    -- Minutes of no typing before a draft goes in unsigned; NULL (or no row) = the default, 180.
    -- At most 30 days: long enough for any site, short enough that a draft is never forgotten.
    quarantine_minutes int CONSTRAINT note_policy_quarantine_in_range
                           CHECK (quarantine_minutes BETWEEN 1 AND 43200)
);

-- Nobody but the owner reads or writes it; the doors below are the only way in (db/061's posture).
REVOKE ALL ON note_policy FROM PUBLIC, cairn_agent, cairn_node;

-- Set the site's quarantine (the console verb's door). NULL = back to the default. Out of range
-- raises, so a typo at the console is told, not stored. No grant to any runtime role.
CREATE OR REPLACE FUNCTION cairn_note_set_quarantine(p_minutes int)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
BEGIN
    IF p_minutes NOT BETWEEN 1 AND 43200 THEN
        RAISE EXCEPTION 'cairn_note_set_quarantine: the quarantine must be at least 1 minute and at most 43200 minutes (30 days); NULL means the default, 180';
    END IF;
    INSERT INTO note_policy (singleton, quarantine_minutes) VALUES (true, p_minutes)
    ON CONFLICT (singleton) DO UPDATE SET quarantine_minutes = EXCLUDED.quarantine_minutes;
END;
$$;

-- The site's quarantine in minutes: the stored value, else 180. A definer, because the window
-- shows when a draft falls due but may not read note_policy.
CREATE OR REPLACE FUNCTION cairn_note_quarantine_minutes()
RETURNS int LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT COALESCE((SELECT p.quarantine_minutes FROM note_policy p), 180);
$$;

-- The drafts now overdue (no real change for the quarantine time, on the database's clock),
-- oldest last keystroke first; p_patient = one chart, NULL = every chart. A HINT only: it takes no
-- lock, so a draft listed here may be typed into, signed or committed before the caller gets to
-- it. cairn_note_draft_commit_overdue re-decides. Volatile, as clock_timestamp() is.
CREATE OR REPLACE FUNCTION cairn_note_drafts_overdue(p_patient uuid)
RETURNS TABLE(draft_id uuid)
LANGUAGE sql SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT d.draft_id FROM note_draft d
     WHERE (p_patient IS NULL OR d.patient_id = p_patient)
       AND d.last_keystroke + make_interval(mins => cairn_note_quarantine_minutes()) <= clock_timestamp()
     ORDER BY d.last_keystroke, d.draft_id;
$$;

-- Each open draft on p_patient's chart: when it falls due and whether it is overdue now. Both from
-- the HEADER's last keystroke, the input cairn_note_drafts_overdue and the commit door read, so the
-- window's due words can never disagree with the timer (a backward clock step lets db/066 clamp a
-- part's last keystroke apart from its header's). For the window's words only: no lock, no write.
CREATE OR REPLACE FUNCTION cairn_note_drafts_due(p_patient uuid)
RETURNS TABLE(draft_id uuid, due timestamptz, overdue boolean)
LANGUAGE sql SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT q.draft_id, q.due, q.due <= clock_timestamp()
      FROM (SELECT d.draft_id,
                   d.last_keystroke + make_interval(mins => cairn_note_quarantine_minutes()) AS due
              FROM note_draft d WHERE d.patient_id = p_patient) q;
$$;

-- Commit overdue draft p_draft into note event p_event (the first non-blank part's event).
-- Returns the same columns as cairn_note_draft_spend_parts (db/066), so the caller builds the
-- parts exactly as a Sign does. Zero rows = nothing to build, for one of three reasons:
--   * the draft is absent or already spent (by a Sign or another tick), or not yet overdue —
--     nothing was written;
--   * it is overdue but every part is blank — its header (parts cascade) and revisions were
--     deleted, and no spend recorded (an empty draft never becomes a note, design §3.5).
-- Otherwise: the non-blank parts in writing order; the header, parts and revisions deleted; and
-- the spend recorded with by_timer = true. db/065's trigger then insists, at COMMIT, that p_event
-- is a note of this chart inserted in this same transaction.
CREATE OR REPLACE FUNCTION cairn_note_draft_commit_overdue(p_draft uuid, p_event uuid)
RETURNS TABLE(patient_id uuid, seq int, writer text, narrative text, stated_time timestamptz,
              recording_started timestamptz, last_keystroke timestamptz)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_patient   uuid;
    v_keystroke timestamptz;
    v_any       boolean;
BEGIN
    IF p_draft IS NULL OR p_event IS NULL THEN
        RAISE EXCEPTION 'cairn_note_draft_commit_overdue needs a draft and an event';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('note_draft:' || p_draft::text, 0));
    -- Read AFTER the lock: a save that just finished has moved last_keystroke, and is seen here.
    SELECT d.patient_id, d.last_keystroke INTO v_patient, v_keystroke
      FROM note_draft d WHERE d.draft_id = p_draft FOR UPDATE;
    IF NOT FOUND THEN
        RETURN;  -- absent, or spent (a spend deletes the header)
    END IF;
    IF v_keystroke + make_interval(mins => cairn_note_quarantine_minutes()) > clock_timestamp() THEN
        RETURN;  -- not yet overdue
    END IF;
    -- RETURN QUERY buffers these rows now, so the deletes below cannot take them away.
    RETURN QUERY
        SELECT v_patient, p.seq, p.writer, p.narrative, p.stated_time, p.recording_started, p.last_keystroke
          FROM note_draft_part p
         WHERE p.draft_id = p_draft AND cairn_twin_is_present(p.narrative)  -- the floor's blank test
         ORDER BY p.seq;
    v_any := FOUND;
    DELETE FROM note_draft_revision r WHERE r.draft_id = p_draft;
    DELETE FROM note_draft d WHERE d.draft_id = p_draft;
    IF v_any THEN
        INSERT INTO note_draft_spent (draft_id, patient_id, event_id, by_timer)
        VALUES (p_draft, v_patient, p_event, true);
    END IF;
END;
$$;

REVOKE EXECUTE ON FUNCTION cairn_note_set_quarantine(int), cairn_note_quarantine_minutes(),
    cairn_note_drafts_overdue(uuid), cairn_note_drafts_due(uuid),
    cairn_note_draft_commit_overdue(uuid, uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_note_quarantine_minutes(), cairn_note_drafts_overdue(uuid),
    cairn_note_drafts_due(uuid), cairn_note_draft_commit_overdue(uuid, uuid) TO cairn_window;
