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


-- ---------------------------------------------------------------------------
-- cairn_custody_state / cairn_custody_landed / cairn_release_pen_row (#578)
-- ---------------------------------------------------------------------------
--
-- WHY THESE ARE MIRRORED HERE. Two crates must ask "did the custody a caller presented
-- actually land?": `cairn-node`'s restore, before it counts a record as applied, and
-- `cairn-sync`, before it deletes a pen row that may hold the last copy of a record's key
-- (`requeue`'s release AND `pull`'s auto-release — #578, and that review's finding that the
-- pull path had never asked). Neither crate can call the other's Rust: `cairn-node` is the
-- higher layer and the two use different Postgres clients. So the predicate lives in the
-- database, and its behaviour is pinned HERE rather than in any caller's suite — a
-- difference between callers would now be a difference in one of these functions.
--
-- Two answers must never be dropped by a reader tidying this up:
--
--   * A LOGGED SHRED IS SETTLED. db/020 step 9 refuses custody outright for an
--     already-shredded target (ADR-0005's anti-resurrection rule), so a caller that treated
--     that as "custody did not land" would hold the record forever waiting for a key that was
--     destroyed ON PURPOSE — and would keep a copy of that key in the pen while it waited.
--   * A PLAINTEXT EVENT IS SETTLED. It has no body to open, so a DEK riding beside it is
--     meaningless; holding its pen row would strand a record that is fully readable.
--
-- The fixture's UUIDs are fixed, not random, so the block that re-runs the questions AS THE
-- NODE ROLE (arm 9) can re-derive the same addresses without a shared temp table. They are
-- fixture identifiers, not key material.

BEGIN;

DO $$
DECLARE
    v_held      UUID := 'c0570d1a-0000-7000-8000-000000000001';
    v_shredded  UUID := 'c0570d1a-0000-7000-8000-000000000002';
    v_withheld  UUID := 'c0570d1a-0000-7000-8000-000000000003';
    v_plaintext UUID := 'c0570d1a-0000-7000-8000-000000000004';
    a_held      BYTEA := '\x1220'::bytea || sha256('c0570d1a-0000-7000-8000-000000000001'::bytea);
    a_shredded  BYTEA := '\x1220'::bytea || sha256('c0570d1a-0000-7000-8000-000000000002'::bytea);
    a_withheld  BYTEA := '\x1220'::bytea || sha256('c0570d1a-0000-7000-8000-000000000003'::bytea);
    a_plaintext BYTEA := '\x1220'::bytea || sha256('c0570d1a-0000-7000-8000-000000000004'::bytea);
    a_absent    BYTEA := '\x1220'::bytea || sha256('absent'::bytea);
BEGIN
    -- Four events that differ ONLY in their custody state, so a wrong answer cannot be blamed
    -- on anything else about the row. Three are SEALED, as the door records a sealed arrival
    -- (db/020 writes `sealed` from the envelope); the fourth is the plaintext control. The
    -- content address is derived from the signed bytes exactly as the real one is (`\x1220` is
    -- the sha256 multihash prefix).
    INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall,
        hlc_counter, node_origin, signed_bytes, content_address, body, contributors,
        signer_key_id, plaintext_twin, sealed)
    VALUES
      (v_held,      gen_random_uuid(), 'custody.probe', 'test-1', 1, 0, 'n',
       v_held::text::bytea,      a_held,      '{}', '[]', 'k', 't', TRUE),
      (v_shredded,  gen_random_uuid(), 'custody.probe', 'test-1', 2, 0, 'n',
       v_shredded::text::bytea,  a_shredded,  '{}', '[]', 'k', 't', TRUE),
      (v_withheld,  gen_random_uuid(), 'custody.probe', 'test-1', 3, 0, 'n',
       v_withheld::text::bytea,  a_withheld,  '{}', '[]', 'k', 't', TRUE),
      (v_plaintext, gen_random_uuid(), 'custody.probe', 'test-1', 4, 0, 'n',
       v_plaintext::text::bytea, a_plaintext, '{}', '[]', 'k', 't', FALSE);
    INSERT INTO event_dek (event_id, dek_wrapped) VALUES (v_held, '\xdeadbeef'::bytea);
    INSERT INTO erasure_shred_log (target_event_id, shred_event_id, basis)
    VALUES (v_shredded, gen_random_uuid(), 'test');

    -- 1. Every state is named. A caller counts, words and exits on these, so each must be a
    --    distinct answer rather than a boolean a caller has to reverse-engineer.
    IF cairn_custody_state(a_held) <> 'held' THEN
        RAISE EXCEPTION 'an event WITH an event_dek row is held, got %', cairn_custody_state(a_held);
    END IF;
    IF cairn_custody_state(a_shredded) <> 'shredded' THEN
        RAISE EXCEPTION 'a logged shred is shredded, got %', cairn_custody_state(a_shredded);
    END IF;
    IF cairn_custody_state(a_withheld) <> 'withheld' THEN
        RAISE EXCEPTION 'a sealed event with no DEK and no shred is withheld, got %',
                        cairn_custody_state(a_withheld);
    END IF;
    IF cairn_custody_state(a_plaintext) <> 'plaintext' THEN
        RAISE EXCEPTION 'an unsealed event is plaintext, got %', cairn_custody_state(a_plaintext);
    END IF;
    IF cairn_custody_state(a_absent) <> 'absent' THEN
        RAISE EXCEPTION 'an address this node never saw is absent, got %', cairn_custody_state(a_absent);
    END IF;

    -- 2. Held, shredded and plaintext are SETTLED: custody "landed" in the only sense a caller
    --    deciding whether to let go of a key can use.
    IF NOT cairn_custody_landed(a_held) THEN
        RAISE EXCEPTION 'held custody must report landed';
    END IF;
    IF NOT cairn_custody_landed(a_shredded) THEN
        RAISE EXCEPTION 'a SHREDDED event must report landed (ADR-0005: the key was destroyed on '
                        'purpose; waiting for it is waiting forever)';
    END IF;
    IF NOT cairn_custody_landed(a_plaintext) THEN
        RAISE EXCEPTION 'a PLAINTEXT event must report landed: there is no body a DEK could open';
    END IF;

    -- 3. Withheld is the #578 state — the answer that must stop anyone deleting the pen row.
    IF cairn_custody_landed(a_withheld) THEN
        RAISE EXCEPTION 'a sealed event with NO event_dek and NO shred must report NOT landed';
    END IF;

    -- 4. Absent is not landed either. This is NOT the anti-vacuity arm for an always-FALSE
    --    function (arm 2 kills that, and an always-FALSE function would PASS this one). What it
    --    kills is any "everything but withheld" reading — `cairn_custody_state(p) <> 'withheld'`,
    --    or an inverted anti-join over the log — which passes arms 2 and 3 and answers TRUE for an
    --    address this node does not hold. A caller whose digest matched no event would then read
    --    "landed" and delete the key.
    IF cairn_custody_landed(a_absent) THEN
        RAISE EXCEPTION 'an address absent from event_log must report NOT landed';
    END IF;

    -- Pen one row per state, each carrying a DEK and the event's OWN signed bytes (the release
    -- guard asks about the event those bytes address, not the digest a row is filed under — arm
    -- 6b), plus a KEYLESS row for an event this node never saw. Through the real pen door.
    PERFORM cairn_quarantine_event(a_held,      v_held::text::bytea,      NULL, NULL, '(restore)', 1, 'r', '\xaa'::bytea, NULL, NULL);
    PERFORM cairn_quarantine_event(a_shredded,  v_shredded::text::bytea,  NULL, NULL, '(restore)', 2, 'r', '\xaa'::bytea, NULL, NULL);
    PERFORM cairn_quarantine_event(a_withheld,  v_withheld::text::bytea,  NULL, NULL, '(restore)', 3, 'r', '\xaa'::bytea, NULL, NULL);
    PERFORM cairn_quarantine_event(a_plaintext, v_plaintext::text::bytea, NULL, NULL, '(restore)', 4, 'r', '\xaa'::bytea, NULL, NULL);
    PERFORM cairn_quarantine_event(a_absent,    'absent'::bytea,          NULL, NULL, 'node-a',    5, 'r', NULL,          NULL, NULL);
END $$;

-- 5. THE GUARD. A keyed row whose custody did not land is NOT released, whoever asks — the
--    rule `requeue` states, and the one `pull`'s auto-release used to break.
DO $$
DECLARE
    a_withheld BYTEA := '\x1220'::bytea || sha256('c0570d1a-0000-7000-8000-000000000003'::bytea);
BEGIN
    IF cairn_release_pen_row(a_withheld) THEN
        RAISE EXCEPTION 'a pen row holding a key that did NOT land must not be released';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM sync_quarantine WHERE content_digest = a_withheld) THEN
        RAISE EXCEPTION 'and the row, with its key, must still be there';
    END IF;
END $$;

-- 6. Settled keyed rows ARE released, and the function says so. Anti-vacuity for arm 5: a
--    function that refused everything would pass it.
DO $$
DECLARE
    a_held      BYTEA := '\x1220'::bytea || sha256('c0570d1a-0000-7000-8000-000000000001'::bytea);
    a_shredded  BYTEA := '\x1220'::bytea || sha256('c0570d1a-0000-7000-8000-000000000002'::bytea);
    a_plaintext BYTEA := '\x1220'::bytea || sha256('c0570d1a-0000-7000-8000-000000000004'::bytea);
BEGIN
    IF NOT cairn_release_pen_row(a_held) THEN
        RAISE EXCEPTION 'a keyed row whose custody is HELD must be released';
    END IF;
    IF NOT cairn_release_pen_row(a_shredded) THEN
        RAISE EXCEPTION 'a keyed row over a SHREDDED event must be released — keeping it would '
                        'keep a copy of a key erasure destroyed on purpose';
    END IF;
    IF NOT cairn_release_pen_row(a_plaintext) THEN
        RAISE EXCEPTION 'a keyed row over a PLAINTEXT event must be released';
    END IF;
    IF EXISTS (SELECT 1 FROM sync_quarantine WHERE content_digest IN (a_held, a_shredded, a_plaintext)) THEN
        RAISE EXCEPTION 'a released row must actually leave the pen';
    END IF;
END $$;

-- 6b. A MIS-KEYED row is judged by its BYTES, not by the digest it is filed under. Filed under
--     the held event's address (free again after arm 6) but holding the WITHHELD event's bytes and
--     a key: the address it is filed under has settled custody, the event it carries does not. A
--     guard that trusted the caller's digest would delete the only copy of that event's key.
DO $$
DECLARE
    a_held     BYTEA := '\x1220'::bytea || sha256('c0570d1a-0000-7000-8000-000000000001'::bytea);
BEGIN
    PERFORM cairn_quarantine_event(a_held, 'c0570d1a-0000-7000-8000-000000000003'::bytea, NULL,
                                   NULL, '(restore)', 6, 'mis-keyed', '\xaa'::bytea, NULL, NULL);
    IF cairn_release_pen_row(a_held) THEN
        RAISE EXCEPTION 'a row filed under a settled address but carrying a withheld event''s bytes '
                        'and key must NOT be released';
    END IF;
    DELETE FROM sync_quarantine WHERE content_digest = a_held;  -- tidy for the arms below
END $$;

-- 7. A KEYLESS row has no custody to lose and releases whatever its event's state — the modal
--    sync-path row, which a guard that read `landed` for every row would strand.
DO $$
DECLARE
    a_absent BYTEA := '\x1220'::bytea || sha256('absent'::bytea);
BEGIN
    IF NOT cairn_release_pen_row(a_absent) THEN
        RAISE EXCEPTION 'a keyless pen row must be released';
    END IF;
END $$;

-- 8. Releasing a row that is not there reports FALSE rather than raising: a concurrent pull or
--    requeue taking the row first is ordinary during a recovery session.
DO $$
BEGIN
    IF cairn_release_pen_row('\x0badd1'::bytea) THEN
        RAISE EXCEPTION 'releasing an absent row must report FALSE';
    END IF;
END $$;

-- 9. THE NODE ROLE CAN ASK. Every other arm runs as the table owner, which holds every
--    privilege and so cannot notice a missing GRANT — the exact gap a production `requeue`
--    connecting as a `cairn_node` member would hit at its first keyed row. These functions
--    run with the CALLER's rights, so this also proves `cairn_node` can read the three tables
--    they consult and delete from the pen.
SET LOCAL ROLE cairn_node;
DO $$
DECLARE
    a_held     BYTEA := '\x1220'::bytea || sha256('c0570d1a-0000-7000-8000-000000000001'::bytea);
    a_withheld BYTEA := '\x1220'::bytea || sha256('c0570d1a-0000-7000-8000-000000000003'::bytea);
BEGIN
    IF cairn_custody_state(a_held) <> 'held' OR NOT cairn_custody_landed(a_held) THEN
        RAISE EXCEPTION 'the node role must get the same answers the owner does';
    END IF;
    IF cairn_release_pen_row(a_withheld) THEN
        RAISE EXCEPTION 'the guard must hold for the node role too';
    END IF;
END $$;
RESET ROLE;

ROLLBACK;
