-- Cairn — the person menu's two read doors (sign-in slice 3c-3c; design:
-- docs/superpowers/specs/2026-11-06-sign-in-slice-3c-3-supervision-and-pending-signatures-design.md
-- §3.4). The menu is a cross-chart "Waiting for you" list in the clinician window: drafts and
-- notes that wait for ONE person's signature, co-signature or review, over every chart.
--
-- WHY DEFINER DOORS. The window's login role (cairn_window, db/062) has no grant on vault_member
-- and none on actor_event's actor columns, yet the menu must (a) turn a key id into the PERSON who
-- held it, and name that person, and (b) find the drafts each staff member has written into. Both
-- facts live in tables the window cannot read, so each gets one read-only SECURITY DEFINER door,
-- as db/071 §7 and db/072 did for their readers. Neither writes; neither returns text of a note.
--
-- Replay-safe: this file re-runs on every schema replay (CREATE OR REPLACE, fixed return shapes).

BEGIN;

-- Everyone the menu may have to NAME, one row per actor: every vault member, plus every actor the
-- supervision record mentions as subject or supervisor (such an actor may have lost its member
-- row, as after a restore — its row then answers staff_id from the handle pinned on its enroll
-- row, and display_name NULL, which the window words honestly as "a writer this node cannot name").
-- keys: EVERY signing key the actor ever enrolled or rotated to, repudiated ones included — a part
-- signed with a since-repudiated key still belongs to the real person (ADR-0085), so the window
-- must map that old key back to this actor. actor_id is lower hex, as db/071's doors answer.
CREATE OR REPLACE FUNCTION cairn_note_people()
RETURNS TABLE(actor_id text, staff_id text, display_name text, keys text[])
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT encode(i.actor_id, 'hex'),
           cairn_supervision_staff_of(i.actor_id),
           (SELECT m.display_name FROM vault_member m
             WHERE m.actor_id = i.actor_id ORDER BY m.staff_id LIMIT 1),
           ARRAY(SELECT DISTINCT e.signing_key_id FROM actor_event e
                  WHERE e.actor_id = i.actor_id AND e.op IN ('enroll', 'rotate-key')
                    AND e.signing_key_id IS NOT NULL
                  ORDER BY 1)
      FROM (SELECT m.actor_id FROM vault_member m
            UNION SELECT decode(r.actor_id, 'hex') FROM cairn_supervision_record() r
            UNION SELECT decode(r.related_actor, 'hex') FROM cairn_supervision_record() r
                   WHERE r.related_actor IS NOT NULL) i
     ORDER BY 1;
$$;

-- Every open draft with its writers: one row per (draft, distinct named writer of a NON-BLANK
-- part — the commit door's own blank test, cairn_twin_is_present). A part by an unknown (NULL)
-- writer names nobody, so it belongs on nobody's menu. due and stuck are cairn_note_drafts_due's
-- (db/067) expressions copied verbatim, so the menu and the chart's own draft row can never
-- disagree; they are per DRAFT, hence repeated on each of its writers' rows. No clock read, so
-- the window decides "overdue" itself. No text and no terminal leave this door.
CREATE OR REPLACE FUNCTION cairn_note_drafts_by_writer()
RETURNS TABLE(writer text, draft_id uuid, patient_id uuid, due timestamptz, stuck boolean)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT w.writer, d.draft_id, d.patient_id,
           d.last_keystroke + make_interval(mins => cairn_note_quarantine_minutes()),
           EXISTS (SELECT 1 FROM note_draft_part p
                    WHERE p.draft_id = d.draft_id AND p.writer IS NOT NULL
                      AND cairn_twin_is_present(p.narrative)  -- the commit door's blank test
                      AND cairn_vault_member_key(p.writer) IS NULL)
      FROM (SELECT DISTINCT p.draft_id, p.writer FROM note_draft_part p
             WHERE p.writer IS NOT NULL AND cairn_twin_is_present(p.narrative)) w
      JOIN note_draft d ON d.draft_id = w.draft_id
     ORDER BY 4, d.draft_id, w.writer;
$$;

-- Only the window reads them; PUBLIC (and so any other role) does not.
REVOKE EXECUTE ON FUNCTION cairn_note_people() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_note_drafts_by_writer() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_note_people() TO cairn_window;
GRANT EXECUTE ON FUNCTION cairn_note_drafts_by_writer() TO cairn_window;

COMMIT;
