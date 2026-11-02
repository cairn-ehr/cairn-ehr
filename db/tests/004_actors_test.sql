-- Run with:  scripts/run-db-sql-tests.sh — since #169 this file refuses any database that is not
-- marked disposable, so an ad-hoc `psql -f db/004_actors.sql -f db/tests/004_actors_test.sql`
-- against a dev database now stops at the guard. See db/tests/_scratch_database_guard.sql.
\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
BEGIN;

-- Enroll an agent; its actor_id is the hash of its pinned set (C4).
SELECT enroll_actor('agent',
    '{"model":"triage-stub","version":"1","skill_epoch":"epoch-a"}'::jsonb,
    'deadbeef') AS aid \gset
SELECT count(*) = 1 AS enrolled_one FROM actor_current WHERE actor_id = :'aid'::bytea;

-- Bumping skill_epoch mints a DIFFERENT actor_id (the supersede trigger for C4). A fresh
-- key per epoch matches the real matcher (matcher_actor.rs) and the issue #166 floor guard
-- (one key binds at most one actor_id); the actor_id derives from the pinned set alone, so
-- the distinct key does not affect what this asserts.
SELECT enroll_actor('agent',
    '{"model":"triage-stub","version":"1","skill_epoch":"epoch-b"}'::jsonb,
    'deadbee2') AS aid2 \gset
SELECT (:'aid'::bytea <> :'aid2'::bytea) AS epoch_bump_is_new_actor;

-- Monotonic tiebreak (issue #99): registry rows landing in the SAME microsecond
-- must resolve deterministically by insertion order (seq), and a same-timestamp
-- revoke inserted after a registration must still end its currency.
DO $$
DECLARE t timestamptz := clock_timestamp(); winner text;
BEGIN
    INSERT INTO actor_event (actor_id, op, kind, pinned, signing_key_id, recorded_at)
    VALUES ('\xaa'::bytea, 'enroll', 'agent', '{"v":"first"}',  'tiekey', t),
           ('\xaa'::bytea, 'enroll', 'agent', '{"v":"second"}', 'tiekey', t);
    SELECT pinned ->> 'v' INTO winner FROM actor_current WHERE actor_id = '\xaa'::bytea;
    IF winner IS DISTINCT FROM 'second' THEN
        RAISE EXCEPTION 'tiebreak FAILED: same-timestamp winner is %, want the later insert', winner;
    END IF;
    INSERT INTO actor_event (actor_id, op, recorded_at) VALUES ('\xaa'::bytea, 'revoke', t);
    IF EXISTS (SELECT 1 FROM actor_current WHERE actor_id = '\xaa'::bytea) THEN
        RAISE EXCEPTION 'tiebreak FAILED: same-timestamp revoke did not end currency';
    END IF;
    RAISE NOTICE 'monotonic tiebreak OK';
END $$;

-- The registry is append-only: UPDATE/DELETE must raise.
DO $$ BEGIN
    BEGIN
        UPDATE actor_event SET op = 'revoke';
        RAISE EXCEPTION 'append-only check FAILED: update succeeded';
    EXCEPTION WHEN others THEN
        IF SQLERRM LIKE '%append-only%' THEN RAISE NOTICE 'append-only OK'; ELSE RAISE; END IF;
    END;
END $$;

-- issue #152: enroll fails CLOSED when the same pinned set (→ same actor_id) is
-- enrolled with a DIFFERENT signing key. The minimal human pinned set is the classic
-- collision; a distinguishing determinant is what keeps two clinicians distinct.
DO $$
DECLARE aid1 bytea; aid2 bytea;
BEGIN
    aid1 := enroll_actor('human', '{"role":"clinician"}', 'keyAAAA');
    -- Idempotent same-key re-enroll is allowed (re-runnable provisioning).
    aid2 := enroll_actor('human', '{"role":"clinician"}', 'keyAAAA');
    IF aid1 IS DISTINCT FROM aid2 THEN
        RAISE EXCEPTION 'collision test FAILED: same (pinned,key) should map to one actor_id';
    END IF;
    -- Different key, identical pinned set → must raise.
    BEGIN
        PERFORM enroll_actor('human', '{"role":"clinician"}', 'keyBBBB');
        RAISE EXCEPTION 'collision test FAILED: distinct-key collision was NOT refused';
    EXCEPTION WHEN others THEN
        IF SQLERRM LIKE '%different signing key%' THEN
            RAISE NOTICE 'actor_id collision refusal OK';
        ELSE RAISE; END IF;
    END;
    -- The pure predicate agrees.
    IF NOT cairn_actor_id_key_conflict(aid1, 'keyBBBB') THEN
        RAISE EXCEPTION 'predicate FAILED: keyBBBB should conflict with aid1';
    END IF;
    IF cairn_actor_id_key_conflict(aid1, 'keyAAAA') THEN
        RAISE EXCEPTION 'predicate FAILED: keyAAAA is the SAME key, no conflict';
    END IF;
END $$;

-- issue #152, immortality edge: after a REVOKE, re-enrolling even the ORIGINAL key is
-- refused — a fresh enroll would outrank the revoke in actor_current and RESURRECT a
-- retired actor. The keyless revoke row makes the predicate fire for the same key too;
-- pin it so the predicate can't be "tidied" back to reopening resurrection.
DO $$
DECLARE aid bytea;
BEGIN
    aid := enroll_actor('human', '{"role":"clinician","actor":"revoke-edge"}', 'keyCCCC');
    INSERT INTO actor_event (actor_id, op) VALUES (aid, 'revoke');
    -- The predicate now conflicts even with the SAME key (a keyless revoke row exists).
    IF NOT cairn_actor_id_key_conflict(aid, 'keyCCCC') THEN
        RAISE EXCEPTION 'predicate FAILED: same key after revoke must conflict (no resurrection)';
    END IF;
    BEGIN
        PERFORM enroll_actor('human', '{"role":"clinician","actor":"revoke-edge"}', 'keyCCCC');
        RAISE EXCEPTION 'resurrection test FAILED: same-key re-enroll after revoke was NOT refused';
    EXCEPTION WHEN others THEN
        IF SQLERRM LIKE '%issue #152%' THEN
            RAISE NOTICE 'post-revoke resurrection refusal OK';
        ELSE RAISE; END IF;
    END;
END $$;

-- issue #166 (B-direction): one signing key must not bind two actor_ids. A fresh enroll
-- under a distinct pinned set with an already-bound key is refused (else db/005 NULLs that
-- key's authorship node-wide). The mirror of the #152 A-direction guard above.
DO $$
DECLARE ok boolean := false;
BEGIN
    PERFORM enroll_actor('agent', '{"model":"m","skill_epoch":"dm-a"}'::jsonb, 'dualkey');
    BEGIN
        PERFORM enroll_actor('agent', '{"model":"m","skill_epoch":"dm-b"}'::jsonb, 'dualkey');
    EXCEPTION WHEN others THEN
        ok := (SQLERRM LIKE '%different actor_id%');
        IF NOT ok THEN
            RAISE EXCEPTION 'wrong error for #166 dual-mapping: %', SQLERRM;
        END IF;
    END;
    IF NOT ok THEN
        RAISE EXCEPTION 'FAILED: #166 dual-mapping enroll (one key, two actor_ids) was allowed';
    END IF;
    RAISE NOTICE 'issue #166 dual-mapping refusal OK';
END $$;

-- Sign-in slice 2 (#664/#666): the widened op CHECK, the per-op shape CHECK
-- (actor_event_op_shape) and actor_current's op list. Mirrors actor_rotate_key.rs.
DO $$
DECLARE a bytea; b bytea; k text;
BEGIN
    a := enroll_actor('human', '{"role":"clinician","handle":"rot-a"}', 'rotK1');
    b := enroll_actor('human', '{"role":"clinician","handle":"rot-b"}', 'rotK9');
    -- rotate-key on the SAME actor: latest-row-wins makes the new key current.
    PERFORM cairn_actor_append(a, 'rotate-key', 'rotK2', NULL, NULL);
    SELECT signing_key_id INTO k FROM actor_current WHERE actor_id = a;
    IF k IS DISTINCT FROM 'rotK2' THEN
        RAISE EXCEPTION 'rotate-key FAILED: current key is %, want rotK2', k;
    END IF;
    IF (SELECT count(*) FROM actor_key_vouches WHERE actor_id = a) <> 2 THEN
        RAISE EXCEPTION 'actor_key_vouches FAILED: both keys must vouch after a rotation';
    END IF;
    -- A repudiated latest key leaves no current key (no fall-back to rotK1).
    PERFORM cairn_actor_append(a, 'repudiate', 'rotK2', 'su', NULL);
    IF EXISTS (SELECT 1 FROM actor_current WHERE actor_id = a) THEN
        RAISE EXCEPTION 'repudiate FAILED: a repudiated latest key must leave no current key';
    END IF;
    IF cairn_key_confirmation('rotK2') <> 'repudiated' THEN
        RAISE EXCEPTION 'cairn_key_confirmation FAILED for a repudiated key';
    END IF;
    -- Shape: each op's key/superseded_by/approved_by are fixed (#666).
    BEGIN
        INSERT INTO actor_event (actor_id, op, signing_key_id, superseded_by)
        VALUES (b, 'supersede', 'rotK9', a);
        RAISE EXCEPTION 'shape FAILED: a supersede row carrying a key was accepted';
    EXCEPTION WHEN check_violation THEN NULL;
    END;
    BEGIN
        INSERT INTO actor_event (actor_id, op, signing_key_id) VALUES (b, 'revoke', 'rotK9');
        RAISE EXCEPTION 'shape FAILED: a revoke row carrying a key was accepted';
    EXCEPTION WHEN check_violation THEN NULL;
    END;
    BEGIN
        INSERT INTO actor_event (actor_id, op, signing_key_id) VALUES (b, 'confirm', 'rotK9');
        RAISE EXCEPTION 'shape FAILED: a confirm row with no approver was accepted';
    EXCEPTION WHEN check_violation THEN NULL;
    END;
    -- A keyless supersede on the OLD actor_id ends its currency (#664).
    INSERT INTO actor_event (actor_id, op, superseded_by) VALUES (b, 'supersede', a);
    IF EXISTS (SELECT 1 FROM actor_current WHERE actor_id = b) THEN
        RAISE EXCEPTION 'supersede FAILED: a superseded actor is still current';
    END IF;
    RAISE NOTICE 'rotate-key / shape / supersede OK';
END $$;

ROLLBACK;
