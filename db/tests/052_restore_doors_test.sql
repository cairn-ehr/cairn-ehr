\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- Issue #554 slice 2d — the in-DB mirror for the two restore-path doors in db/052.
--
-- WHY THIS MIRROR EXISTS. `restore_actor_registry` writes the trust anchor that EVERY
-- clinical apply door gates on (`actor_current`, db/004), and it deliberately bypasses
-- `enroll_actor`'s collision guards. That makes it the highest-privilege new door in the
-- slice, and its two fences plus its ordering property are properties of the SQL itself —
-- they must be provable without a Rust caller in the picture, because a Rust caller is
-- exactly what a later slice might replace.
--
-- `cairn_quarantine_event` is mirrored here for the opposite reason: it was LIFTED out of
-- `cairn-sync`'s Rust (a binary-only crate `cairn-node` cannot call) so both crates share
-- one pen. A behaviour difference between the two callers would now be a difference in this
-- function, so the quota's two arms are pinned at the SQL layer where both can see them.

BEGIN;

-- ---------------------------------------------------------------------------
-- restore_actor_registry
-- ---------------------------------------------------------------------------

-- A registry as it would arrive from a dead node's export: an enroll and its later revoke,
-- deliberately pinned to the SAME recorded_at. That is not incidental — `actor_current`
-- orders on (recorded_at, seq) with recorded_at PRIMARY, so with distinct timestamps the
-- timestamp alone would decide and the seq-ordering half of this door would be untested.
-- The hex actor_id is a 32-byte content address's shape; its value carries no meaning.
CREATE TEMP TABLE registry_fixture (rows JSONB);
INSERT INTO registry_fixture VALUES ($json$[
  {"actor_event_id": "aaaaaaaa-0000-7000-8000-000000000001",
   "actor_id": "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
   "op": "enroll", "kind": "human", "signing_key_id": "beef01", "seq": 1,
   "recorded_at": "2026-01-01 00:00:00.000001+00"},
  {"actor_event_id": "aaaaaaaa-0000-7000-8000-000000000002",
   "actor_id": "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
   "op": "revoke", "seq": 2,
   "recorded_at": "2026-01-01 00:00:00.000001+00"}
]$json$::jsonb);

-- 1. A clean install on a fresh, un-enrolled database inserts everything and says so.
DO $$
DECLARE n INT;
BEGIN
    SELECT restore_actor_registry(rows) INTO n FROM registry_fixture;
    IF n <> 2 THEN
        RAISE EXCEPTION 'a clean install must insert and report both rows, got %', n;
    END IF;
    IF (SELECT count(*) FROM actor_event) <> 2 THEN
        RAISE EXCEPTION 'both rows must be in actor_event';
    END IF;
END $$;

-- 2. THE ORDERING PROPERTY. enroll -> revoke restores to a registry where the actor is
--    ABSENT from actor_current. This is what a defaulted or re-stamped recorded_at, or an
--    insertion in the wrong order, would silently break: a restored `revoke` that landed
--    before its `enroll` would leave a RECALLED actor authorised to author clinical events,
--    through the very door built to restore the registry.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM actor_current
                WHERE actor_id = decode('0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20', 'hex')) THEN
        RAISE EXCEPTION 'a revoked actor must not be current after a restore';
    END IF;
END $$;

-- 3. THE COUNTER IS NOT LEFT BEHIND. `seq` is GENERATED ALWAYS AS IDENTITY and this door
--    lets Postgres assign it rather than replaying the dead node's values, so the next
--    ordinary enroll still gets a seq above every restored one. An explicit-seq restore
--    would leave the identity counter at 1 and plant a duplicate for the NEXT enroll.
DO $$
DECLARE v_next BIGINT; v_max BIGINT;
BEGIN
    INSERT INTO actor_event (actor_id, op, kind, signing_key_id)
    VALUES (decode('99', 'hex'), 'enroll', 'human', 'cafe01');
    SELECT max(seq) INTO v_next FROM actor_event WHERE signing_key_id = 'cafe01';
    SELECT max(seq) INTO v_max  FROM actor_event WHERE signing_key_id IS DISTINCT FROM 'cafe01';
    IF v_next <= v_max THEN
        RAISE EXCEPTION 'the identity counter was left behind: next seq % is not above restored max %', v_next, v_max;
    END IF;
    DELETE FROM actor_event WHERE signing_key_id = 'cafe01';
EXCEPTION WHEN others THEN
    -- actor_event is append-only, so the DELETE above raises. Swallow only that, after the
    -- assertion has already run: this test's subject is the counter, not the trigger.
    IF SQLERRM NOT LIKE '%append-only%' THEN RAISE; END IF;
END $$;

ROLLBACK;

-- ---------------------------------------------------------------------------
-- Fence 2 needs its own transaction: the half-provisioned node's row must exist
-- BEFORE the door is called, and the refusal aborts the transaction it runs in.
-- ---------------------------------------------------------------------------

BEGIN;
CREATE TEMP TABLE registry_fixture2 (rows JSONB);
INSERT INTO registry_fixture2 VALUES ($json$[
  {"actor_event_id": "aaaaaaaa-0000-7000-8000-000000000001",
   "actor_id": "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
   "op": "enroll", "kind": "human", "signing_key_id": "beef01", "seq": 1,
   "recorded_at": "2026-01-01 00:00:00.000001+00"},
  {"actor_event_id": "aaaaaaaa-0000-7000-8000-000000000002",
   "actor_id": "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
   "op": "revoke", "seq": 2,
   "recorded_at": "2026-01-01 00:00:00.000001+00"}
]$json$::jsonb);

-- 4. THE RESUME PATH. Install a PARTIAL registry (the residue of a restore that failed
--    part-way), then run the full set: only the remainder is inserted, the count says so,
--    and the resulting actor_current is identical to a clean install. This is what makes
--    design §3's "finalize_identity moves LAST" argument true — a door that refused because
--    `actor_event` held ANY row would turn a resumable restore into a fresh-database one.
DO $$
DECLARE n INT;
BEGIN
    INSERT INTO actor_event (actor_event_id, actor_id, op, kind, signing_key_id, recorded_at)
    VALUES ('aaaaaaaa-0000-7000-8000-000000000001',
            decode('0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20', 'hex'),
            'enroll', 'human', 'beef01', '2026-01-01 00:00:00.000001+00');

    SELECT restore_actor_registry(rows) INTO n FROM registry_fixture2;
    IF n <> 1 THEN
        RAISE EXCEPTION 'a resume must insert ONLY the remainder and report it, got %', n;
    END IF;
    IF (SELECT count(*) FROM actor_event) <> 2 THEN
        RAISE EXCEPTION 'a resume must leave exactly the restored set';
    END IF;
    IF EXISTS (SELECT 1 FROM actor_current
                WHERE actor_id = decode('0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20', 'hex')) THEN
        RAISE EXCEPTION 'a resumed restore must reach the same actor_current as a clean one';
    END IF;
END $$;
ROLLBACK;

BEGIN;
-- 5. FENCE 2. A registry holding a row that is NOT in p_rows is a half-provisioned node
--    that enrolled its own actor, never the residue of this restore — and injecting into it
--    would silently merge two nodes' trust anchors. `enroll_actor` has zero `local_node`
--    references, so a populated actor_event on an un-enrolled database is NOT provably a
--    failed restore: this is why the fence is set-shaped rather than a blanket TRUNCATE.
DO $$
DECLARE ok BOOLEAN := FALSE;
BEGIN
    INSERT INTO actor_event (actor_id, op, kind, signing_key_id)
    VALUES (decode('77', 'hex'), 'enroll', 'human', 'f00d01');
    BEGIN
        PERFORM restore_actor_registry($json$[
          {"actor_event_id": "aaaaaaaa-0000-7000-8000-000000000001",
           "actor_id": "01", "op": "enroll", "seq": 1,
           "recorded_at": "2026-01-01 00:00:00+00"}
        ]$json$::jsonb);
    EXCEPTION WHEN others THEN
        ok := TRUE;
        IF SQLERRM NOT LIKE '%restore_actor_registry%' THEN
            RAISE EXCEPTION 'the refusal must name its door, got: %', SQLERRM;
        END IF;
    END;
    IF NOT ok THEN
        RAISE EXCEPTION 'a foreign registry row must refuse the whole call';
    END IF;
END $$;
ROLLBACK;

BEGIN;
-- 6. FENCE 1. A LIVE node is never a restore target. Checked separately from fence 2
--    because they fail for different reasons and an operator must be told which.
DO $$
DECLARE ok BOOLEAN := FALSE;
BEGIN
    INSERT INTO local_node (node_id, signer_key_id) VALUES (decode('aa', 'hex'), 'live01');
    BEGIN
        PERFORM restore_actor_registry($json$[]$json$::jsonb);
    EXCEPTION WHEN others THEN
        ok := TRUE;
    END;
    IF NOT ok THEN
        RAISE EXCEPTION 'restore_actor_registry must refuse an enrolled node';
    END IF;
END $$;
ROLLBACK;

BEGIN;
-- 7. `recorded_at` REFUSES rather than defaulting. It is `actor_current`'s PRIMARY ordering
--    key, so a row whose timestamp defaulted to now() would outrank a genuine older revoke
--    and silently re-authorise a recalled actor. Refusing the row is the only safe verdict.
DO $$
DECLARE ok BOOLEAN := FALSE;
BEGIN
    BEGIN
        PERFORM restore_actor_registry($json$[
          {"actor_event_id": "aaaaaaaa-0000-7000-8000-000000000009",
           "actor_id": "01", "op": "enroll", "seq": 1}
        ]$json$::jsonb);
    EXCEPTION WHEN others THEN
        ok := TRUE;
        IF SQLERRM NOT LIKE '%recorded_at%' THEN
            RAISE EXCEPTION 'the refusal must name the missing field, got: %', SQLERRM;
        END IF;
    END;
    IF NOT ok THEN
        RAISE EXCEPTION 'a row with no recorded_at must be refused, never defaulted';
    END IF;
END $$;
ROLLBACK;

-- ---------------------------------------------------------------------------
-- cairn_quarantine_event
-- ---------------------------------------------------------------------------

BEGIN;

-- 8. A new row is penned WITH its custody, and the row count is reported.
DO $$
DECLARE v_acked BOOLEAN;
BEGIN
    SELECT cairn_quarantine_event('\x01020304'::bytea, '\xaa'::bytea, NULL, NULL,
                                  '(restore)', 0, 'a fixture refusal',
                                  '\xfeedface'::bytea, NULL, NULL)
      INTO v_acked;
    IF v_acked THEN RAISE EXCEPTION 'a freshly penned row is not acked'; END IF;
    IF (SELECT dek_wrapped FROM sync_quarantine WHERE content_digest = '\x01020304'::bytea)
       IS DISTINCT FROM '\xfeedface'::bytea THEN
        RAISE EXCEPTION 'the pen must preserve the record custody — without it a requeued \
sealed event is permanently unopenable ciphertext';
    END IF;
END $$;

-- 9. A re-offer of known bytes BUMPS rather than duplicating, and enriches an absent
--    attestation. This is the sync path's behaviour, unchanged by the lift.
DO $$
DECLARE v_seen INT;
BEGIN
    PERFORM cairn_quarantine_event('\x01020304'::bytea, '\xaa'::bytea, '\xbb'::bytea, NULL,
                                   'node-a', 5, 'a second look', NULL, NULL, NULL);
    SELECT seen_count INTO v_seen FROM sync_quarantine WHERE content_digest = '\x01020304'::bytea;
    IF v_seen <> 2 THEN RAISE EXCEPTION 'a re-offer must bump seen_count, got %', v_seen; END IF;
    IF (SELECT attestation FROM sync_quarantine WHERE content_digest = '\x01020304'::bytea)
       IS DISTINCT FROM '\xbb'::bytea THEN
        RAISE EXCEPTION 'a token once seen is never dropped';
    END IF;
    IF (SELECT reason FROM sync_quarantine WHERE content_digest = '\x01020304'::bytea)
       <> 'a fixture refusal' THEN
        RAISE EXCEPTION 'the ORIGINAL forensic reason must survive a re-offer';
    END IF;
END $$;

-- 10. THE QUOTA IS THE CALLER'S POLICY, and NULL means unbounded. A restore passes NULL
--     because the quota bounds a hostile PEER and a restore has none — and because the bytes
--     it would refuse are bytes the node is about to lose permanently. A zero row cap proves
--     the enforced arm still enforces; NULL proves the carve-out is real.
DO $$
DECLARE ok BOOLEAN := FALSE;
BEGIN
    BEGIN
        PERFORM cairn_quarantine_event('\x05060708'::bytea, '\xcc'::bytea, NULL, NULL,
                                       'node-a', 1, 'over quota', NULL, 0, NULL);
    EXCEPTION WHEN others THEN
        ok := TRUE;
        IF SQLERRM NOT LIKE '%quota%' THEN
            RAISE EXCEPTION 'a full pen must say so, got: %', SQLERRM;
        END IF;
    END;
    IF NOT ok THEN RAISE EXCEPTION 'a zero row cap must refuse a NEW row'; END IF;

    -- The same call with an unbounded quota succeeds. Without this the test above would
    -- pass against a door that refused everything.
    PERFORM cairn_quarantine_event('\x05060708'::bytea, '\xcc'::bytea, NULL, NULL,
                                   '(restore)', 1, 'unbounded', NULL, NULL, NULL);
    IF NOT EXISTS (SELECT 1 FROM sync_quarantine WHERE content_digest = '\x05060708'::bytea) THEN
        RAISE EXCEPTION 'an unbounded quota must admit the row';
    END IF;
END $$;

ROLLBACK;
