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

-- Monotonic insertion tiebreak (issue #99). recorded_at is clock_timestamp() with
-- microsecond resolution, so two registry rows written by one ceremony (or one
-- transaction) can carry the SAME timestamp; ordering actor_current on recorded_at
-- alone then makes "which row is current?" nondeterministic. seq is assigned in
-- insertion order and never reused, so latest-row-wins is exact. Additive column
-- (ADR-0012 discipline); existing rows are backfilled in table order, which for an
-- append-only table is insertion order.
--
-- GENERATED ALWAYS, deliberately (PR #106 review finding 2): the tiebreak decides
-- which registration/revocation is CURRENT on a trust anchor, so a quiet explicit
-- seq (e.g. a revoke back-dated below a registration's seq) must not be one stray
-- INSERT away. ALWAYS refuses an explicit value unless the writer says
-- OVERRIDING SYSTEM VALUE — still possible for the owner, but loud in review.
ALTER TABLE actor_event ADD COLUMN IF NOT EXISTS seq BIGINT GENERATED ALWAYS AS IDENTITY;
-- Self-heal a database that ran the earlier BY DEFAULT revision of this file
-- (ADD COLUMN IF NOT EXISTS skips existing columns, so the line above alone
-- would leave it soft). SET GENERATED ALWAYS is idempotent.
ALTER TABLE actor_event ALTER COLUMN seq SET GENERATED ALWAYS;

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
-- no later revoke. "Later" is (recorded_at, seq) — the seq tiebreak makes both the
-- winner and the revoke comparison deterministic when timestamps collide (a revoke
-- inserted after a same-microsecond registration still kills it; one inserted
-- before does not resurrect against a re-enrollment).
CREATE OR REPLACE VIEW actor_current AS
SELECT DISTINCT ON (ae.actor_id)
       ae.actor_id, ae.kind, ae.pinned, ae.signing_key_id, ae.recorded_at
FROM actor_event ae
WHERE ae.op IN ('enroll','supersede')
  AND NOT EXISTS (
      SELECT 1 FROM actor_event r
      WHERE r.actor_id = ae.actor_id AND r.op = 'revoke'
        AND (r.recorded_at, r.seq) >= (ae.recorded_at, ae.seq))
ORDER BY ae.actor_id, ae.recorded_at DESC, ae.seq DESC;

-- issue #152: an actor_id is the content-address of the PINNED set only, never the
-- signing key (the key must stay mutable across a future rotate-key, ADR-0011 §5). So
-- two DIFFERENT signing keys enrolled with an IDENTICAL pinned set compute the SAME
-- actor_id, and actor_current's `DISTINCT ON (actor_id)` silently keeps only the
-- latest key — a silent identity merge on the trust anchor (principle 2). This pure
-- predicate is TRUE iff some existing actor_event row for this actor_id does NOT carry
-- exactly p_key.
--
-- `IS DISTINCT FROM` intentionally matches TWO kinds of row: (a) a row bound to a
-- DIFFERENT key (the #152 silent-merge), and (b) a row with a NULL key — revoke and
-- supersede rows carry no signing_key_id. Case (b) is deliberate, not incidental: it
-- means a fresh enroll onto an actor_id that was ever revoked/superseded is refused even
-- with the ORIGINAL key. That prevents RESURRECTION — a post-revoke enroll would outrank
-- the revoke in actor_current's (recorded_at, seq) order and silently re-authorise a
-- recalled actor (the exact hazard matcher_actor.rs guards in Rust; here it is enforced
-- at the DB floor itself, principle 2). Do NOT add `signing_key_id IS NOT NULL` to
-- "tidy up" case (b) without adding a separate explicit revoke-history guard, or you
-- reopen resurrection.
--
-- Whole history (incl. revoked rows): an actor_id is immortal and is never reusable by a
-- different key, even after revoke. STABLE + a small pure function so it is independently
-- testable and reusable at the future actor-sync apply door.
CREATE OR REPLACE FUNCTION cairn_actor_id_key_conflict(p_actor_id BYTEA, p_key TEXT)
RETURNS BOOLEAN LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM actor_event
        WHERE actor_id = p_actor_id
          AND signing_key_id IS DISTINCT FROM p_key
    );
$$;

-- issue #166: the B-direction mirror of cairn_actor_id_key_conflict. submit_event (db/005)
-- resolves a signer to an actor purely by signing_key_id; if one key maps to MORE than one
-- actor_id it stamps actor_id = NULL for EVERY event that key authors node-wide (a silent,
-- irreversible attribution loss). So enroll must fail closed when p_key already binds a
-- DIFFERENT actor_id. This pure predicate is TRUE iff some existing enroll/supersede row
-- carries p_key under an actor_id other than p_actor_id.
--
-- WHOLE-HISTORY, deliberately (the guard-scope decision): a key that ever bound a different
-- actor can never enroll a new one, even after that actor is revoked/superseded — the same
-- anti-reuse posture cairn_actor_id_key_conflict takes in the A-direction (principle 2, a
-- key is one lifelong actor identity). `op IN ('enroll','supersede')` restricts to the
-- key-bearing ops; revoke rows carry a NULL signing_key_id and are excluded by the equality
-- anyway. STABLE + pure so it is independently testable and reusable at the future
-- actor-sync apply door (ADR-0044 §3) and a future rotate-key/supersede door.
--
-- Note the argument order is (key, actor_id) — the MIRROR of
-- cairn_actor_id_key_conflict(actor_id, key) above; each predicate's arguments follow its
-- own name. A future door that calls both must pass them in each function's own order (the
-- disjoint bytea/text types make a swapped call fail loudly at plan time, never silently).
CREATE OR REPLACE FUNCTION cairn_key_actor_id_conflict(p_key TEXT, p_actor_id BYTEA)
RETURNS BOOLEAN LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM actor_event
        WHERE signing_key_id = p_key
          AND actor_id IS DISTINCT FROM p_actor_id
          AND op IN ('enroll','supersede')
    );
$$;

-- Enroll an actor; its identity is derived in-DB from the pinned set (cairn_pgx),
-- so "identity = hash of what is pinned" is enforced, not asserted. Because the
-- signing key is deliberately NOT part of that hash (rotate-key preserves actor_id,
-- ADR-0011 §5), enroll must fail CLOSED if the computed actor_id already binds a
-- DIFFERENT key — otherwise two distinct actors silently merge (issue #152). A HUMAN
-- actor therefore needs a person-distinguishing determinant in its pinned set (a handle
-- / registration id); the minimal `{"role":...}` collides across people. That field is
-- left to the future enrollment surface (ADR-0011 keeps pinned-set CONTENTS as policy);
-- this floor only makes a forgotten determinant LOUD on the second enroll.
-- SINGLE DOOR: there is no remote-apply door for actor enrollment yet (INSERT INTO
-- actor_event lives only here). When actor-event sync lands (ADR-0011 §4), mirror this
-- check at that apply door — same shape as the #154 apply-door caveat.
CREATE OR REPLACE FUNCTION enroll_actor(p_kind TEXT, p_pinned JSONB, p_key TEXT)
RETURNS BYTEA LANGUAGE plpgsql AS $$
DECLARE aid BYTEA;
BEGIN
    aid := cairn_actor_id(p_pinned);
    -- Close the check-then-insert race (TOCTOU): the conflict check below reads
    -- COMMITTED rows, so two concurrent transactions enrolling the SAME actor_id with
    -- DIFFERENT keys could each see no conflict and both insert — the very silent merge
    -- this guard exists to prevent. A txn-scoped advisory lock keyed on the actor_id
    -- serializes only same-actor_id enrolls (never distinct actors) and is released at
    -- COMMIT/ROLLBACK; under the default READ COMMITTED isolation the loser then re-reads
    -- the winner's committed row and is refused. (hashtextextended → the bigint lock key.)
    -- issue #166: serialize concurrent enrolls of the SAME KEY (this guard) as well as of
    -- the same actor_id (the #152 guard). Key lock FIRST, then actor_id lock — one global
    -- acquire order across the single enroll door, so no deadlock is possible. Both locks
    -- live in Postgres's single advisory-lock keyspace; the distinct seed ((…, 1) vs (…, 0))
    -- keeps the two hash VALUES from colliding, so a key string that happens to equal an
    -- actor_id hex string cannot map onto the same lock. Under READ COMMITTED the loser
    -- blocks on the key lock, then re-reads the winner's committed row and is refused by the
    -- B-check below.
    PERFORM pg_advisory_xact_lock(hashtextextended(p_key, 1));
    PERFORM pg_advisory_xact_lock(hashtextextended(encode(aid, 'hex'), 0));
    IF cairn_actor_id_key_conflict(aid, p_key) THEN
        RAISE EXCEPTION
            'enroll_actor: actor_id % already has prior registration history under this identity (a different signing key, and/or a revoke/supersede) — a fresh enroll is refused: it would silently merge two actors or resurrect a retired one (issue #152)',
            encode(aid, 'hex')
        USING HINT =
            'Give this actor a distinguishing pinned determinant (e.g. a person handle / registration id), or use rotate-key to add a key to the SAME actor.';
    END IF;
    -- issue #166 (B-direction): refuse if this signing key already binds a DIFFERENT
    -- actor_id anywhere in history — otherwise db/005 would NULL this key's authorship
    -- node-wide (whole-history / anti-key-reuse; see cairn_key_actor_id_conflict).
    IF cairn_key_actor_id_conflict(p_key, aid) THEN
        RAISE EXCEPTION
            'enroll_actor: signing key % already binds a different actor_id — a fresh enroll is refused: one key mapping to two actors silently NULLs that key''s authorship node-wide (db/005; issue #166)',
            p_key
        USING HINT =
            'A genuinely different entity needs its own key. To add a key to the SAME actor, use rotate-key/supersede (no door yet). A retired key never becomes a new actor (whole-history, anti-key-reuse).';
    END IF;
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
-- Defense in depth: the collision predicate reads the trust-anchor table; keep it off
-- PUBLIC too (STABLE + read-only makes it low-risk, but the floor stays explicit).
REVOKE EXECUTE ON FUNCTION cairn_actor_id_key_conflict(bytea, text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_key_actor_id_conflict(text, bytea) FROM PUBLIC;

COMMIT;
