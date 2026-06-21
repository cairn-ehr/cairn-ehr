-- Cairn walking skeleton — recall + contamination overlay (Spike 0002 §4.6 / C4).
-- An actor recall marks affected events via an append-only overlay; it NEVER edits
-- or deletes event_log (principle 2: never erase, always overlay).

BEGIN;

CREATE TABLE IF NOT EXISTS recall_overlay (
    recall_id       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    target_event_id UUID NOT NULL,
    reason          TEXT NOT NULL,
    recorded_at     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE OR REPLACE FUNCTION recall_overlay_is_append_only()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'recall_overlay is append-only: % not permitted (principle #2)', TG_OP;
END;
$$;
DROP TRIGGER IF EXISTS recall_overlay_no_update ON recall_overlay;
CREATE TRIGGER recall_overlay_no_update BEFORE UPDATE OR DELETE ON recall_overlay
    FOR EACH ROW EXECUTE FUNCTION recall_overlay_is_append_only();

-- Events authored by the actor(s) whose pinned skill_epoch matches (C4 recall query).
-- NOTE: joins actor_current (current state). Valid for recall at the actor's current epoch. After a supersede bumps skill_epoch, a query for the OLD epoch will miss its events — production recall must resolve against historical actor_event rows, not actor_current.
CREATE OR REPLACE FUNCTION events_by_actor_epoch(p_key TEXT, p_epoch TEXT)
RETURNS TABLE(event_id UUID, event_type TEXT) LANGUAGE sql STABLE AS $$
    SELECT el.event_id, el.event_type
    FROM event_log el
    JOIN actor_current ac ON ac.signing_key_id = el.signer_key_id
    WHERE el.signer_key_id = p_key
      AND ac.pinned ->> 'skill_epoch' = p_epoch;
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

COMMIT;
