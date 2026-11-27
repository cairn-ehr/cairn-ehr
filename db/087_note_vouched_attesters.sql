-- db/087 — the verified attesters of many parts, in one call (#1021).
--
-- WHY: the note list and the person menu (`note::read`'s `notes_sql!`) grade each part by its
-- verified attester. They asked db/065's `cairn_note_vouched_attester(uuid)` about every part,
-- one SECURITY DEFINER call each, and the planner cannot inline such a call. At a ward-year
-- (33 000 parts) that was about a fifth of the menu's counts read, although almost no part has
-- an attester at all. This door answers for a whole array of events in one call, so a read
-- pays one invocation instead of one per part.
--
-- It is db/065's door widened, not a new rule: the SAME predicate (an attester key is stored
-- AND `cairn_attestation_vouched` clears it, ruling R11 / #412), and an event with no verified
-- attester simply has no row (the one-event door's NULL). db/tests/087 pins that the two
-- answer alike event by event. db/065's door stays: the vouch column and the medication
-- version reads still ask it about one event at a time, and older windows still call it.
--
-- Why a new FILE and not an in-place edit of db/065 (ADR-0080 would allow one): the window
-- needs this door, and a window meets a database of its OWN generation as "current" whatever
-- that database's content. Bumping the generation makes a window built with this door refuse a
-- database without it, instead of failing every note list with "function does not exist".
--
-- Replay-safe: CREATE OR REPLACE FUNCTION, idempotent grants.

BEGIN;

CREATE OR REPLACE FUNCTION cairn_note_vouched_attesters(p_events uuid[])
RETURNS TABLE (event_id uuid, attester text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT e.event_id, encode(e.attester_key, 'hex') FROM public.event_log e
     WHERE e.event_id = ANY(p_events)
       AND e.attester_key IS NOT NULL
       AND cairn_attestation_vouched(e.event_id);
$$;

REVOKE EXECUTE ON FUNCTION cairn_note_vouched_attesters(uuid[]) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_note_vouched_attesters(uuid[]) TO cairn_window;

COMMIT;
