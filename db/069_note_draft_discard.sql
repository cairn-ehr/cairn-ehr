-- db/069 — the audited DISCARD of a draft progress note (sign-in slice 3b-2b, ADR-0089 decision 15).
--
-- WHY: a draft (db/065 + db/066) is autosaved, can be carried to another terminal and taken over
-- by a colleague, who may have read it and acted on it. On paper you may tear up your OWN
-- unsigned scribble, never a colleague's writing. So a draft may be thrown away only when every
-- non-blank part is the discarder's own (or by a writer not known), and never silently: the
-- discarder signs a clinical.note-discard.asserted (db/068) that records what the draft said.
--
-- THE RULE, exactly: a part counts only when it is non-blank (cairn_twin_is_present, the floor's
-- one blank test). Each counting part's writer must be NULL (not known) or a member whose CURRENT
-- vault key (cairn_vault_member_key) equals the discarder's key. Compared by KEY, never by staff
-- id. A named writer with no key is 'not-yours': reading them as "not known" would quietly lose
-- who wrote it (principle 4).
--
-- THE DISCARDER is named by key (p_discarder_key) and must be a vault member whose role may act
-- clinically: never a superuser or an admin, who confer no clinical authorship (§5.1).
--
-- WHY THE DOOR'S YES IS NOT YET A DISCARD. The door trusts the key its caller names, and a key
-- id is public. What makes "only your own" true is db/065's commit-time trigger: a spend with
-- discarded_by set must name a discard event of this chart, inserted in this same transaction
-- and SIGNED BY discarded_by, whose t_effective is the instant this door minted under the lock
-- (stored as the spend's spent_at). Only the private key can make that row (the strict door
-- verifies every signature), and no event signed before this call can carry an instant minted
-- inside it, so a raw-SQL client naming a colleague's key, even holding a colleague's captured
-- discard event, gets the draft consumed and the whole transaction refused at COMMIT. Hence NO savepoint around the submit: a row inserted in a
-- subtransaction carries a subxid, and the trigger compares xmin to this transaction's id.
--
-- LOCK ORDER is every draft door's (db/066): the advisory lock 'note_draft:' || draft_id
-- (WAITED for, as save, take and spend do; only the timer's door tries it), then the header row
-- FOR UPDATE, then its parts. So a discard racing the timer or a Sign elsewhere has exactly one
-- winner, and the loser answers 'committed' or 'spent' (or, after a discard, 'discarded').
--
-- Replay-safe: this file re-runs on every schema replay. Its return shape is final.

-- Discard draft p_draft of p_patient, held by p_terminal, into discard event p_event, as the
-- member whose signing key is p_discarder_key.
--
-- A refusal is ONE row whose outcome is 'spent' | 'committed' | 'discarded' | 'moved' |
-- 'elsewhere' | 'gone' | 'not-yours' | 'no-such-signer', every other column NULL, and nothing
-- written. 'empty' (every part blank) is one such row too, but the draft and its revisions WERE
-- deleted and no spend recorded: there is nothing to audit (design §3.5), so no event is needed.
-- Success is one row per non-blank part, outcome 'ok', in writing order, by_discarder = the part
-- has a named writer (under the rule, that writer IS the discarder), all sharing one
-- discarded_at: the discard instant, which the caller puts in the event's t_effective (the window
-- never sends a time). The header, parts and revisions are then deleted and the spend recorded.
CREATE OR REPLACE FUNCTION cairn_note_draft_discard(
    p_draft uuid, p_patient uuid, p_terminal text, p_event uuid, p_discarder_key text)
RETURNS TABLE(outcome text, seq int, by_discarder boolean, narrative text, stated_time timestamptz,
              recording_started timestamptz, last_keystroke timestamptz, discarded_at timestamptz)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_head  note_draft%ROWTYPE;
    v_spent text;
    v_now   timestamptz;
BEGIN
    -- A NULL is a caller bug, not an outcome: `<> NULL` is NULL and would slip past the guards.
    IF p_draft IS NULL OR p_patient IS NULL OR p_terminal IS NULL OR p_event IS NULL
       OR p_discarder_key IS NULL THEN
        RAISE EXCEPTION 'cairn_note_draft_discard needs a draft, a patient, a terminal, an event and a discarder key';
    END IF;
    -- The discarder must be a clinical member. A plain read of vault_member taking no lock, BEFORE
    -- the draft's lock, as the save door's writer check is, so it cannot deadlock with the order.
    IF NOT EXISTS (SELECT 1 FROM vault_member m
                    WHERE m.signing_key_id = p_discarder_key
                      AND cairn_vault_purpose_allowed(m.role, false, 'clinical')) THEN
        RETURN QUERY SELECT 'no-such-signer', NULL::int, NULL::boolean, NULL::text, NULL::timestamptz,
                            NULL::timestamptz, NULL::timestamptz, NULL::timestamptz;
        RETURN;
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('note_draft:' || p_draft::text, 0));
    -- The discard instant: ONE clock reading per call, shared by every returned row and stored as
    -- the spend's spent_at. Read AFTER the lock (as db/066's save door does): a discard queued
    -- behind a save must not be stamped before that save's last keystroke, since this instant is
    -- signed into the event's t_effective.
    v_now := clock_timestamp();
    -- Already gone into the record or thrown away: each says which (db/066's doors say the same).
    v_spent := (SELECT CASE WHEN s.discarded_by IS NOT NULL THEN 'discarded'
                            WHEN s.by_timer THEN 'committed' ELSE 'spent' END
                  FROM note_draft_spent s WHERE s.draft_id = p_draft);
    IF v_spent IS NULL THEN
        SELECT * INTO v_head FROM note_draft d WHERE d.draft_id = p_draft FOR UPDATE;
        v_spent := CASE WHEN NOT FOUND THEN 'gone'
                        WHEN v_head.patient_id IS DISTINCT FROM p_patient THEN 'elsewhere'
                        WHEN v_head.terminal_id IS DISTINCT FROM p_terminal THEN 'moved' END;
    END IF;
    -- The rule (header comment): any counting part neither not-known nor under this key refuses.
    IF v_spent IS NULL AND EXISTS (
        SELECT 1 FROM note_draft_part p
         WHERE p.draft_id = p_draft AND cairn_twin_is_present(p.narrative)
           AND p.writer IS NOT NULL
           AND cairn_vault_member_key(p.writer) IS DISTINCT FROM p_discarder_key) THEN
        v_spent := 'not-yours';
    END IF;
    IF v_spent IS NOT NULL THEN
        RETURN QUERY SELECT v_spent, NULL::int, NULL::boolean, NULL::text, NULL::timestamptz,
                            NULL::timestamptz, NULL::timestamptz, NULL::timestamptz;
        RETURN;
    END IF;
    -- RETURN QUERY buffers these rows now, so the deletes below cannot take them away.
    RETURN QUERY
        SELECT 'ok'::text, p.seq, p.writer IS NOT NULL, p.narrative, p.stated_time,
               p.recording_started, p.last_keystroke, v_now
          FROM note_draft_part p
         WHERE p.draft_id = p_draft AND cairn_twin_is_present(p.narrative)  -- the floor's blank test
         ORDER BY p.seq;
    IF NOT FOUND THEN
        -- Every part blank: nothing to audit, so nothing to sign; the deletion is the whole act.
        DELETE FROM note_draft_revision r WHERE r.draft_id = p_draft;
        DELETE FROM note_draft d WHERE d.draft_id = p_draft;
        RETURN QUERY SELECT 'empty'::text, NULL::int, NULL::boolean, NULL::text, NULL::timestamptz,
                            NULL::timestamptz, NULL::timestamptz, NULL::timestamptz;
        RETURN;
    END IF;
    DELETE FROM note_draft_revision r WHERE r.draft_id = p_draft;
    DELETE FROM note_draft d WHERE d.draft_id = p_draft;  -- its parts cascade
    -- db/065's trigger holds this, at COMMIT, to an event signed by p_discarder_key whose
    -- t_effective is exactly v_now (spent_at): the event of THIS call, never an earlier one.
    INSERT INTO note_draft_spent (draft_id, patient_id, event_id, spent_at, by_timer, discarded_by)
    VALUES (p_draft, p_patient, p_event, v_now, false, p_discarder_key);
END;
$$;

REVOKE EXECUTE ON FUNCTION cairn_note_draft_discard(uuid, uuid, text, uuid, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_note_draft_discard(uuid, uuid, text, uuid, text) TO cairn_window;
