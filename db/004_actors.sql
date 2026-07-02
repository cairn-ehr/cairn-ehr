-- Cairn walking skeleton — the append-only actor registry (Spike 0002 §4.1).
--
-- ADR-0011: actor identity is version-pinned and immutable. An actor_id IS the
-- content-address of its pinned-determinant set (computed by cairn_pgx), so
-- bumping any determinant (incl. skill_epoch) mints a new actor via a fresh
-- enroll/supersede row — never an edit (principle 2). The closed actor-event
-- algebra is enroll | supersede | revoke.

BEGIN;

CREATE TABLE IF NOT EXISTS actor_event (
    actor_event_id  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_id        BYTEA   NOT NULL,           -- content-address of the pinned set
    op              TEXT    NOT NULL CHECK (op IN ('enroll','supersede','revoke')),
    kind            TEXT    CHECK (kind IN ('human','agent','device')),
    pinned          JSONB,                       -- the version-pinned determinant set
    signing_key_id  TEXT,                        -- hex Ed25519 public key
    superseded_by   BYTEA,                       -- for supersede: the new actor_id
    recorded_at     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE INDEX IF NOT EXISTS actor_event_actor_idx ON actor_event (actor_id);
CREATE INDEX IF NOT EXISTS actor_event_key_idx ON actor_event (signing_key_id);

-- Append-only: refuse UPDATE and DELETE (principle 1), same pattern as event_log.
CREATE OR REPLACE FUNCTION actor_event_is_append_only()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'actor_event is append-only: % is not permitted (Cairn principle #1)', TG_OP;
END;
$$;
DROP TRIGGER IF EXISTS actor_event_no_update ON actor_event;
CREATE TRIGGER actor_event_no_update BEFORE UPDATE OR DELETE ON actor_event
    FOR EACH ROW EXECUTE FUNCTION actor_event_is_append_only();

-- Current, non-revoked identities: the latest enroll/supersede per actor_id with
-- no later revoke.
CREATE OR REPLACE VIEW actor_current AS
SELECT DISTINCT ON (ae.actor_id)
       ae.actor_id, ae.kind, ae.pinned, ae.signing_key_id, ae.recorded_at
FROM actor_event ae
WHERE ae.op IN ('enroll','supersede')
  AND NOT EXISTS (
      SELECT 1 FROM actor_event r
      WHERE r.actor_id = ae.actor_id AND r.op = 'revoke' AND r.recorded_at >= ae.recorded_at)
ORDER BY ae.actor_id, ae.recorded_at DESC;

-- Enroll an actor; its identity is derived in-DB from the pinned set (cairn_pgx),
-- so "identity = hash of what is pinned" is enforced, not asserted.
CREATE OR REPLACE FUNCTION enroll_actor(p_kind TEXT, p_pinned JSONB, p_key TEXT)
RETURNS BYTEA LANGUAGE plpgsql AS $$
DECLARE aid BYTEA;
BEGIN
    aid := cairn_actor_id(p_pinned);
    INSERT INTO actor_event (actor_id, op, kind, pinned, signing_key_id)
    VALUES (aid, 'enroll', p_kind, p_pinned, p_key);
    RETURN aid;
END;
$$;

-- The agent's DB role: it may EXECUTE the submit door and READ projections, but
-- has NO write privilege on the event log (the C5.4 grant floor; granted in 005).
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_agent') THEN
        CREATE ROLE cairn_agent NOLOGIN;
    END IF;
END $$;

-- Trust-anchor floor (make it EXPLICIT, do not rest on implicit defaults). The actor
-- registry decides WHO may author: submit_event (005) trusts actor_current, so anyone who
-- can enroll a pubkey can author "legitimately signed" events. Enrollment must stay an
-- owner-privileged ceremony — never reachable by the runtime agent role or PUBLIC.
-- enroll_actor is invoker-rights (deliberately NOT SECURITY DEFINER), so today the gate
-- holds only because cairn_agent has no INSERT on actor_event by default. That is too
-- fragile for a trust anchor: one stray `GRANT INSERT ON actor_event TO cairn_agent`, or
-- copy-pasting the SECURITY DEFINER pattern the other doors use, would silently collapse
-- it. State the floor so such a change stands out in review. (A negative test asserts
-- cairn_agent cannot enroll — mirrors the C5.4 raw-INSERT floor tests.)
REVOKE INSERT, UPDATE, DELETE ON actor_event FROM PUBLIC, cairn_agent;
REVOKE EXECUTE ON FUNCTION enroll_actor(text, jsonb, text) FROM PUBLIC;

COMMIT;
