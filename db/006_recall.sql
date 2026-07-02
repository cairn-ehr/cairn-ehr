-- Cairn walking skeleton — recall + contamination overlay (Spike 0002 §4.6 / C4).
-- An actor recall marks affected events via an append-only overlay; it NEVER edits
-- or deletes event_log (principle 2: never erase, always overlay).
--
-- Issue #99 hardened this surface: events_by_actor_epoch resolves against the FULL
-- registry history (not actor_current), recall_event refuses a target that is not
-- in the log, and the write floor is stated explicitly.

BEGIN;

CREATE TABLE IF NOT EXISTS recall_overlay (
    recall_id       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    target_event_id UUID NOT NULL,
    reason          TEXT NOT NULL,
    recorded_at     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- A recall must name a real event (issue #99): without this FK a fat-fingered
-- target UUID "succeeds" while recalling nothing, and the operator walks away
-- believing the contamination is handled — a silent failure on a safety surface.
-- event_log rows are never deleted (append-only trigger), so the reference can
-- never dangle. NOTE: recall_overlay is node-LOCAL today (it does not replicate);
-- if a future replicated recall stream must tolerate a recall arriving before its
-- target, that relaxation belongs at the future apply door, not here.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'recall_overlay_target_fk') THEN
        ALTER TABLE recall_overlay
            ADD CONSTRAINT recall_overlay_target_fk
            FOREIGN KEY (target_event_id) REFERENCES event_log(event_id);
    END IF;
END $$;

CREATE OR REPLACE FUNCTION recall_overlay_is_append_only()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'recall_overlay is append-only: % not permitted (principle #2)', TG_OP;
END;
$$;
DROP TRIGGER IF EXISTS recall_overlay_no_update ON recall_overlay;
CREATE TRIGGER recall_overlay_no_update BEFORE UPDATE OR DELETE ON recall_overlay
    FOR EACH ROW EXECUTE FUNCTION recall_overlay_is_append_only();

-- Events authored by the actor (p_key, p_epoch) — the contamination-cascade recall
-- key (ADR-0011/0029/0030). Resolution is against the FULL registry history
-- (actor_event), never actor_current: after a supersede/re-enroll bumps a key's
-- skill_epoch, the OLD epoch's events must remain selectable forever. The previous
-- actor_current join returned NOTHING for a superseded epoch — a production recall
-- would silently under-select (issue #99, the dangerous direction).
--
-- Selection is exact where attribution is exact, conservative where it is not:
--   * 'pinned'           — the event's admission-time attribution stamp
--                          (event_log.actor_id, written by both doors) matches a
--                          historical registration of (key, epoch);
--   * 'unattributed'     — the event is signed by the key but carries no stamp
--                          (admitted before the stamp existed, or the key mapped to
--                          several actors at admission — principle 4's honest
--                          unknown). Included for EVERY epoch the key ever
--                          registered: a recall must over-select, never silently
--                          miss.
--   * 'pre-registration' — the event carries a stamp, but was admitted BEFORE this
--                          node first registered (key, p_epoch). A stamp written
--                          before the node knew this epoch existed cannot be
--                          trusted to EXCLUDE the event from this epoch's recall:
--                          the local registry lags origin-side epoch bumps by
--                          design (db/020 KNOWN LIMITATION — enrollment is a local
--                          ceremony), so an event authored under a newer epoch at
--                          origin gets confidently-but-wrongly stamped with the
--                          only actor its key had ever meant here. "Locally
--                          unambiguous" is not "actually unambiguous"; treating it
--                          so is the precise untruth principle 4 forbids. So the
--                          stamp excludes only events admitted AFTER the epoch's
--                          first local registration; older ones over-select here.
--                          (The ADR-0029 refinement — actor identity inside the
--                          signed bytes — retires this rung; until then the noise
--                          is bounded to events predating the epoch's local
--                          registration, exactly the set this node cannot
--                          attribute.)
-- A (key, epoch) pair that was never registered selects nothing.
-- (DROP first: the return shape gained the attribution column, which
--  CREATE OR REPLACE cannot change.)
DROP FUNCTION IF EXISTS events_by_actor_epoch(text, text);
CREATE FUNCTION events_by_actor_epoch(p_key TEXT, p_epoch TEXT)
RETURNS TABLE(event_id UUID, event_type TEXT, attribution TEXT) LANGUAGE sql STABLE AS $$
    WITH epoch_regs AS (
        SELECT ae.actor_id, ae.recorded_at
        FROM actor_event ae
        WHERE ae.op IN ('enroll','supersede')
          AND ae.signing_key_id = p_key
          AND ae.pinned ->> 'skill_epoch' = p_epoch
    )
    SELECT el.event_id, el.event_type,
           CASE WHEN el.actor_id IS NULL THEN 'unattributed'
                WHEN el.actor_id IN (SELECT r.actor_id FROM epoch_regs r) THEN 'pinned'
                ELSE 'pre-registration'
           END
    FROM event_log el
    WHERE el.signer_key_id = p_key
      AND EXISTS (SELECT 1 FROM epoch_regs)
      AND (el.actor_id IN (SELECT r.actor_id FROM epoch_regs r)
           OR el.actor_id IS NULL
           OR el.recorded_at < (SELECT min(r.recorded_at) FROM epoch_regs r));
$$;

-- Mark one event recalled (append-only overlay, never erase).
CREATE OR REPLACE FUNCTION recall_event(p_target UUID, p_reason TEXT)
RETURNS UUID LANGUAGE plpgsql AS $$
DECLARE rid UUID;
BEGIN
    INSERT INTO recall_overlay (target_event_id, reason)
    VALUES (p_target, p_reason) RETURNING recall_id INTO rid;
    RETURN rid;
END;
$$;

-- Write floor, made explicit (mirrors db/004's enroll_actor hardening): recalling is
-- an operator/steward ceremony, not a runtime-agent capability. recall_event is
-- invoker-rights (deliberately NOT SECURITY DEFINER), so the gate holds only because
-- nothing grants INSERT on recall_overlay — state it, so a stray GRANT (or a
-- copy-pasted SECURITY DEFINER) stands out in review.
REVOKE INSERT, UPDATE, DELETE ON recall_overlay FROM PUBLIC, cairn_agent;
REVOKE EXECUTE ON FUNCTION recall_event(uuid, text) FROM PUBLIC;

COMMIT;
