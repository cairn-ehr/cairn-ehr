-- Cairn walking skeleton — the append-only actor registry (Spike 0002 §4.1).
--
-- ADR-0011: actor identity is version-pinned and immutable. An actor_id IS the
-- content-address of its pinned-determinant set (computed by cairn_pgx), so
-- bumping any determinant (incl. skill_epoch) mints a new actor via a fresh
-- enroll/supersede row — never an edit (principle 2). The closed actor-event
-- algebra is enroll | supersede | revoke | rotate-key | confirm | repudiate |
-- grant-enroller | withdraw-enroller (the last five: GUI sign-in slice 2).
--
-- WHICH ROW CARRIES WHAT (#666, settled by sign-in slice 2; enforced by the
-- actor_event_op_shape CHECK below, so the schema now says what the comments say):
--   enroll, rotate-key      a signing key, never a superseded_by — the key-bearing ops.
--                           rotate-key sits on the SAME actor_id (ADR-0011 §5: the key is
--                           not a determinant), so latest-row-wins retires the old key.
--   supersede               NO key; superseded_by names the NEW actor. The row sits on the
--                           OLD actor_id, which is what lets actor_current drop it (#664).
--   revoke                  NO key; it retires the whole actor_id.
--   confirm, repudiate      the key being judged + approved_by (who judged it).
--   grant-enroller,         NO key + approved_by: a capability on the actor, not a key.
--   withdraw-enroller
-- approved_by is the approver's hex key, or 'console' for an owner-console act; approval is
-- the approver's attestation token (NULL for 'console'). Rows written before slice 2 carry
-- NULL in both: they are outside the confirmation mechanism (cairn_key_confirmation).

BEGIN;

CREATE TABLE IF NOT EXISTS actor_event (
    actor_event_id  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_id        BYTEA   NOT NULL,           -- content-address of the pinned set
    -- Widened below (actor_event_op_check is swapped, not edited here: an existing table
    -- never re-reads this line). Kept as first written so a fresh and an old database meet
    -- the same ALTERs.
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

-- Sign-in slice 2: who approved a key-bearing row (or judged/granted), kept APPEND-ONLY on
-- the registry row itself — the vault's member row is custody state a re-key overwrites, and
-- the cascade (a repudiated enroller makes its unconfirmed keys doubtful) must know who
-- approved every key there ever was. Additive columns (ADR-0012).
ALTER TABLE actor_event ADD COLUMN IF NOT EXISTS approved_by TEXT;
ALTER TABLE actor_event ADD COLUMN IF NOT EXISTS approval    BYTEA;

-- Widen the op CHECK and add the per-op shape CHECK (#666), once. The op CHECK was declared
-- inline above, so Postgres named it actor_event_op_check; it is swapped, not edited. Guarded on
-- the shape constraint's existence so a second replay is a true no-op (no re-validation, no
-- ACCESS EXCLUSIVE lock on every connect). A LATER change to either definition therefore needs
-- a new guard (e.g. on the new definition's text), not an edit here.
--
-- ADD CONSTRAINT validates every EXISTING row, so a pre-#666 row of the wrong shape (say a
-- supersede that carried a key) would fail this load on every connect with Postgres's bare
-- "is violated by some row". So the rows that break it are found first, with the SAME expression
-- (held once, in v_shape), and named in a refusal that says what to do.
DO $$
DECLARE
    v_shape TEXT := $shape$CASE
            WHEN op IN ('enroll','rotate-key')
                THEN signing_key_id IS NOT NULL AND superseded_by IS NULL
            WHEN op = 'supersede'
                THEN signing_key_id IS NULL AND superseded_by IS NOT NULL
            WHEN op = 'revoke'
                THEN signing_key_id IS NULL
            WHEN op IN ('confirm','repudiate')
                THEN signing_key_id IS NOT NULL AND approved_by IS NOT NULL
            WHEN op IN ('grant-enroller','withdraw-enroller')
                THEN signing_key_id IS NULL AND approved_by IS NOT NULL
            ELSE false
        END$shape$;
    v_bad TEXT;
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint
                   WHERE conrelid = 'actor_event'::regclass
                     AND conname = 'actor_event_op_shape') THEN
        EXECUTE format('SELECT string_agg(actor_event_id::text || '' ('' || op || '')'', '', '' ORDER BY seq)
                          FROM actor_event WHERE NOT (%s)', v_shape) INTO v_bad;
        IF v_bad IS NOT NULL THEN
            RAISE EXCEPTION 'actor_event: these registry rows break the per-op shape rule (#666) and must be corrected by the node''s owner before the schema can load: %. A supersede row carries no key (its key belongs on the new actor''s enroll row) and a revoke row carries none; actor_event is append-only, so the owner disables actor_event_no_update, clears the stray column on exactly these rows, and re-enables it', v_bad;
        END IF;
        ALTER TABLE actor_event DROP CONSTRAINT IF EXISTS actor_event_op_check;
        ALTER TABLE actor_event ADD CONSTRAINT actor_event_op_check CHECK (op IN (
            'enroll','supersede','revoke','rotate-key','confirm','repudiate',
            'grant-enroller','withdraw-enroller'));
        EXECUTE format('ALTER TABLE actor_event ADD CONSTRAINT actor_event_op_shape CHECK (%s)', v_shape);
    END IF;
END $$;

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

-- Current identities: per actor_id, the LATEST key-bearing or superseding row
-- (enroll / rotate-key / supersede), kept only if
--   * it is not a supersede (a superseded actor is not current — #664),
--   * no revoke is at or after it, and
--   * no repudiate names its key (a repudiated latest key leaves the actor with NO current
--     key — deliberately no fall-back to the previous key, which was retired by the rotation).
-- "Later" is (recorded_at, seq) — the seq tiebreak makes both the winner and the revoke
-- comparison deterministic when timestamps collide (a revoke inserted after a same-microsecond
-- registration still kills it; one inserted before does not resurrect against a re-enrollment).
-- The latest row is picked in a subquery and filtered OUTSIDE it: filtering inside would let a
-- repudiated or superseded latest row fall back to an older one. The column list is unchanged
-- (CREATE OR REPLACE VIEW cannot change it on an existing database).
--
-- This answers "may this key sign something NEW?". "Did this key legitimately belong to the
-- actor when it signed?" is actor_key_vouches, below.
CREATE OR REPLACE VIEW actor_current AS
SELECT l.actor_id, l.kind, l.pinned, l.signing_key_id, l.recorded_at
FROM (
    SELECT DISTINCT ON (ae.actor_id)
           ae.actor_id, ae.op, ae.kind, ae.pinned, ae.signing_key_id, ae.recorded_at, ae.seq
    FROM actor_event ae
    WHERE ae.op IN ('enroll','rotate-key','supersede')
    ORDER BY ae.actor_id, ae.recorded_at DESC, ae.seq DESC
) l
WHERE l.op <> 'supersede'
  AND NOT EXISTS (
      SELECT 1 FROM actor_event r
      WHERE r.actor_id = l.actor_id AND r.op = 'revoke'
        AND (r.recorded_at, r.seq) >= (l.recorded_at, l.seq))
  AND NOT EXISTS (
      SELECT 1 FROM actor_event p
      WHERE p.op = 'repudiate' AND p.signing_key_id = l.signing_key_id);

-- Every key that ever legitimately belonged to an actor and still vouches for what it signed:
-- each enroll / rotate-key row whose actor was never revoked and whose key was never
-- repudiated. A rotated-away key stays here (its past work is still the actor's), which is the
-- difference from actor_current. For readers that RE-JUDGE an already-admitted event; a write
-- door asks actor_current instead. DISTINCT: an idempotent re-enroll writes a second identical
-- row, and a reader's `count(*) = 1` guard must not see two.
-- A SUPERSEDED actor's keys still vouch here (only revoke and repudiate remove a key). No
-- supersede writer exists yet; the first supersede door owes the decision whether a superseded
-- actor's keys should keep vouching for what they signed, and must change this view if not.
CREATE OR REPLACE VIEW actor_key_vouches AS
SELECT DISTINCT ae.actor_id, ae.kind, ae.signing_key_id
FROM actor_event ae
WHERE ae.op IN ('enroll','rotate-key')
  AND NOT EXISTS (SELECT 1 FROM actor_event r
                  WHERE r.actor_id = ae.actor_id AND r.op = 'revoke')
  AND NOT EXISTS (SELECT 1 FROM actor_event p
                  WHERE p.op = 'repudiate' AND p.signing_key_id = ae.signing_key_id);

-- issue #152: an actor_id is the content-address of the PINNED set only, never the
-- signing key (the key must stay mutable across a future rotate-key, ADR-0011 §5). So
-- two DIFFERENT signing keys enrolled with an IDENTICAL pinned set compute the SAME
-- actor_id, and actor_current's `DISTINCT ON (actor_id)` silently keeps only the
-- latest key — a silent identity merge on the trust anchor (principle 2). This pure
-- predicate is TRUE iff some existing actor_event row for this actor_id does NOT carry
-- exactly p_key.
--
-- `IS DISTINCT FROM` intentionally matches TWO kinds of row: (a) a row bound to a
-- DIFFERENT key (the #152 silent-merge — and, since slice 2, a rotate-key row: re-enrolling
-- the rotated-AWAY key would outrank the rotation and make the retired key current again),
-- and (b) a row with a NULL key — revoke, supersede, grant-enroller and withdraw-enroller rows
-- carry no signing_key_id (the per-op shape CHECK, #666). Case (b) is deliberate, not
-- incidental: it means a fresh enroll onto an actor_id that was ever revoked/superseded is
-- refused even with the ORIGINAL key. That prevents RESURRECTION — a post-revoke enroll would outrank
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
-- DIFFERENT actor_id. This pure predicate is TRUE iff some existing enroll/rotate-key row
-- carries p_key under an actor_id other than p_actor_id.
--
-- WHOLE-HISTORY, deliberately (the guard-scope decision): a key that ever bound a different
-- actor can never enroll a new one, even after that actor is revoked/superseded — the same
-- anti-reuse posture cairn_actor_id_key_conflict takes in the A-direction (principle 2, a
-- key is one lifelong actor identity). `op IN ('enroll','rotate-key')` restricts to the
-- key-bearing ops (#666: supersede carries NO key — an earlier revision listed it here, which
-- contradicted the header; the shape CHECK now settles it). confirm/repudiate rows carry a key
-- too, but only one the same actor already holds (cairn_actor_append checks), so they add
-- nothing. STABLE + pure so it is independently testable and reusable at the future
-- actor-sync apply door (ADR-0044 §3) and at cairn_actor_append's rotate-key arm.
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
          AND op IN ('enroll','rotate-key')
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
-- check at that apply door — same shape as the #154 apply-door caveat. (Since slice 2 there
-- is one more INSERT site, cairn_actor_append below; it refuses op 'enroll'.)
--
-- Sign-in slice 2: two trailing arguments carry who approved this key (approved_by: the
-- approver's hex key, or 'console') and their attestation token (approval). They DEFAULT NULL,
-- so every existing 3-argument caller resolves to this one function unchanged and writes a row
-- outside the confirmation mechanism, exactly as before. The 3-arg signature is DROPPED first,
-- on every replay: CREATE OR REPLACE with a different argument list makes a SECOND overload,
-- and a 3-arg call would then be ambiguous between the two. (The one deliberate exception to
-- the fixed-arity rule; the vault doors that call this are plpgsql, resolved at run time, so
-- the drop never cascades into them.)
DROP FUNCTION IF EXISTS enroll_actor(text, jsonb, text);
CREATE OR REPLACE FUNCTION enroll_actor(p_kind TEXT, p_pinned JSONB, p_key TEXT,
                                        p_approved_by TEXT DEFAULT NULL,
                                        p_approval BYTEA DEFAULT NULL)
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
            'A genuinely different entity needs its own key. To add a key to the SAME actor, use rotate-key (cairn_actor_append). A retired key never becomes a new actor (whole-history, anti-key-reuse).';
    END IF;
    INSERT INTO actor_event (actor_id, op, kind, pinned, signing_key_id, approved_by, approval)
    VALUES (aid, 'enroll', p_kind, p_pinned, p_key, p_approved_by, p_approval);
    RETURN aid;
END;
$$;

-- The ONE other writer of actor_event (sign-in slice 2): appends a rotate-key, confirm,
-- repudiate, grant-enroller or withdraw-enroller row for an already-enrolled actor. enroll
-- keeps enroll_actor (it derives the actor_id); supersede and revoke keep their own paths
-- (db/006's recall, the restore door) — so those three are refused here, and so is anything
-- outside the algebra.
--
-- Invoker rights, deliberately (the enroll_actor posture): this is an owner-only primitive,
-- revoked from PUBLIC. The vault's SECURITY DEFINER doors (db/063) call it AFTER verifying the
-- approver's signed act; nothing here checks approval — the floor here is the registry's own
-- consistency:
--   * rotate-key: the new key must not bind another actor (#166, under the same two advisory
--     locks as enroll_actor, in the same order, so the two doors cannot race or deadlock), must
--     not be a key this actor already held (a rotated-away key never comes back), and the actor
--     must not be revoked or superseded (a rotate-key row would outrank the revoke/supersede in
--     actor_current and silently RESURRECT a retired actor — the #152 hazard by another door).
--     kind and pinned are copied from the actor's enroll row, so actor_current reads the same
--     columns whichever key-bearing row is latest.
--   * confirm / repudiate: the key must be one this actor holds or held.
--   * the per-op shape CHECK (actor_event_op_shape) refuses a missing key or approver.
--   * every op: this machine's clock must not read EARLIER than the actor's latest row. Readers
--     (actor_current, cairn_vault_latest_grant) order by recorded_at first, and a restore keeps
--     each row's original recorded_at — so on a replacement machine whose clock is behind the
--     backup, a new row would sort BEFORE the restored ones: a rotate-key would leave the old key
--     current, a withdrawal would change nothing, each while "succeeding". Refused, naming the
--     clock. An EQUAL timestamp is fine: the seq tiebreak puts the new row last (issue #99).
CREATE OR REPLACE FUNCTION cairn_actor_append(p_actor_id BYTEA, p_op TEXT, p_key TEXT,
                                              p_approved_by TEXT, p_approval BYTEA)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_kind TEXT; v_pinned JSONB; v_latest TIMESTAMPTZ; v_now TIMESTAMPTZ;
BEGIN
    IF p_op IS NULL OR p_op NOT IN
       ('rotate-key','confirm','repudiate','grant-enroller','withdraw-enroller') THEN
        RAISE EXCEPTION 'cairn_actor_append: op % is not appended here — enroll goes through enroll_actor, supersede and revoke through their own paths, and nothing else is in the actor-event algebra',
            COALESCE(p_op, 'NULL');
    END IF;
    IF p_op = 'rotate-key' THEN
        -- Same locks, same order as enroll_actor (key first, then actor_id).
        PERFORM pg_advisory_xact_lock(hashtextextended(p_key, 1));
        PERFORM pg_advisory_xact_lock(hashtextextended(encode(p_actor_id, 'hex'), 0));
    END IF;
    SELECT kind, pinned INTO v_kind, v_pinned
    FROM actor_event WHERE actor_id = p_actor_id AND op = 'enroll'
    ORDER BY seq LIMIT 1;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'cairn_actor_append: actor_id % was never enrolled', encode(p_actor_id, 'hex');
    END IF;
    SELECT max(recorded_at) INTO v_latest FROM actor_event WHERE actor_id = p_actor_id;
    v_now := clock_timestamp();
    IF v_now < v_latest THEN
        RAISE EXCEPTION 'cairn_actor_append: this machine''s clock reads %, earlier than the actor''s latest registry row (%) — a row written now would sort before it and be overruled; set the clock, then retry',
            v_now, v_latest;
    END IF;
    IF p_op = 'rotate-key' THEN
        IF EXISTS (SELECT 1 FROM actor_event WHERE actor_id = p_actor_id
                   AND op IN ('revoke','supersede')) THEN
            RAISE EXCEPTION 'cairn_actor_append: actor_id % is revoked or superseded — a rotate-key would outrank that and resurrect a retired actor (issue #152)',
                encode(p_actor_id, 'hex');
        END IF;
        IF cairn_key_actor_id_conflict(p_key, p_actor_id) THEN
            RAISE EXCEPTION 'cairn_actor_append: signing key % already binds a different actor_id — one key mapping to two actors silently NULLs that key''s authorship node-wide (db/005; issue #166)',
                p_key;
        END IF;
        IF EXISTS (SELECT 1 FROM actor_event WHERE actor_id = p_actor_id
                   AND op IN ('enroll','rotate-key') AND signing_key_id = p_key) THEN
            RAISE EXCEPTION 'cairn_actor_append: signing key % already belongs to this actor — a rotation needs a NEW key; a rotated-away key never becomes current again',
                p_key;
        END IF;
    ELSIF p_op IN ('confirm','repudiate') THEN
        IF NOT EXISTS (SELECT 1 FROM actor_event WHERE actor_id = p_actor_id
                       AND op IN ('enroll','rotate-key') AND signing_key_id = p_key) THEN
            RAISE EXCEPTION 'cairn_actor_append: % names signing key %, which actor_id % never held',
                p_op, COALESCE(p_key, 'NULL'), encode(p_actor_id, 'hex');
        END IF;
    END IF;
    INSERT INTO actor_event (actor_id, op, kind, pinned, signing_key_id, approved_by, approval)
    VALUES (p_actor_id, p_op,
            CASE WHEN p_op = 'rotate-key' THEN v_kind END,
            CASE WHEN p_op = 'rotate-key' THEN v_pinned END,
            p_key, p_approved_by, p_approval);
END;
$$;

-- A key's confirmation status, defined in ONE place (sign-in slice 2; every door and reader
-- asks this rather than re-deriving it). Status belongs to a KEY, never to the actor:
--   'unknown'      no enroll/rotate-key row carries this key;
--   'repudiated'   a superuser repudiated it (final: a later confirm does not undo it);
--   'outside'      no row introducing it carries an approver — rows written before slice 2,
--                  devices, agents. Not unconfirmed: the mechanism never applied to them;
--   'confirmed'    a confirm row names it;
--   'doubtful'     unconfirmed, AND a key that approved it has since been repudiated (the
--                  cascade, derived on read — no extra acts);
--   'unconfirmed'  approved (by an enroller), not yet confirmed.
-- "Introduced with an approver" reads ANY introducing row, so a later approval-less re-enroll
-- of the same key cannot quietly lift it out of the mechanism (the safe direction).
-- SECURITY DEFINER because cairn_agent (the window) has no SELECT on actor_event; it returns a
-- status word for a public key and nothing else.
CREATE OR REPLACE FUNCTION cairn_key_confirmation(p_key TEXT)
RETURNS TEXT LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT CASE
        WHEN NOT EXISTS (SELECT 1 FROM actor_event
                         WHERE signing_key_id = p_key AND op IN ('enroll','rotate-key'))
            THEN 'unknown'
        WHEN EXISTS (SELECT 1 FROM actor_event
                     WHERE signing_key_id = p_key AND op = 'repudiate')
            THEN 'repudiated'
        WHEN NOT EXISTS (SELECT 1 FROM actor_event
                         WHERE signing_key_id = p_key AND op IN ('enroll','rotate-key')
                           AND approved_by IS NOT NULL)
            THEN 'outside'
        WHEN EXISTS (SELECT 1 FROM actor_event
                     WHERE signing_key_id = p_key AND op = 'confirm')
            THEN 'confirmed'
        WHEN EXISTS (SELECT 1 FROM actor_event v
                     JOIN actor_event r ON r.op = 'repudiate' AND r.signing_key_id = v.approved_by
                     WHERE v.signing_key_id = p_key AND v.op IN ('enroll','rotate-key'))
            THEN 'doubtful'
        ELSE 'unconfirmed'
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
-- owner-privileged ceremony — never reachable by PUBLIC, and by the runtime agent role only
-- through a GATED definer door: db/061's cairn_vault_enrol lets cairn_agent enrol a member
-- (of any role, another superuser included), but only with a current superuser's attestation over
-- that exact request. Any other path for
-- the runtime role is a hole.
-- enroll_actor is invoker-rights (deliberately NOT SECURITY DEFINER), so today the gate
-- holds only because cairn_agent has no INSERT on actor_event by default. That is too
-- fragile for a trust anchor: one stray `GRANT INSERT ON actor_event TO cairn_agent`, or
-- copy-pasting the SECURITY DEFINER pattern the other doors use, would silently collapse
-- it. State the floor so such a change stands out in review. (A negative test asserts
-- cairn_agent cannot enroll — mirrors the C5.4 raw-INSERT floor tests.)
REVOKE INSERT, UPDATE, DELETE ON actor_event FROM PUBLIC, cairn_agent;
REVOKE EXECUTE ON FUNCTION enroll_actor(text, jsonb, text, text, bytea) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_actor_append(bytea, text, text, text, bytea) FROM PUBLIC;
-- The two read-side answers the window needs (slice 2): which keys vouch for an actor, and a
-- key's confirmation status. Reads only; neither reveals more than the status of a public key.
REVOKE EXECUTE ON FUNCTION cairn_key_confirmation(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_key_confirmation(text) TO cairn_agent;
GRANT SELECT ON actor_key_vouches TO cairn_agent;
-- Defense in depth: the collision predicate reads the trust-anchor table; keep it off
-- PUBLIC too (STABLE + read-only makes it low-risk, but the floor stays explicit).
REVOKE EXECUTE ON FUNCTION cairn_actor_id_key_conflict(bytea, text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_key_actor_id_conflict(text, bytea) FROM PUBLIC;

COMMIT;
