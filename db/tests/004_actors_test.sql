-- Run with:  psql "$CONN" -v ON_ERROR_STOP=1 -f db/004_actors.sql -f db/tests/004_actors_test.sql
\set ON_ERROR_STOP on
BEGIN;

-- Enroll an agent; its actor_id is the hash of its pinned set (C4).
SELECT enroll_actor('agent',
    '{"model":"triage-stub","version":"1","skill_epoch":"epoch-a"}'::jsonb,
    'deadbeef') AS aid \gset
SELECT count(*) = 1 AS enrolled_one FROM actor_current WHERE actor_id = :'aid'::bytea;

-- Bumping skill_epoch mints a DIFFERENT actor_id (the supersede trigger for C4).
SELECT enroll_actor('agent',
    '{"model":"triage-stub","version":"1","skill_epoch":"epoch-b"}'::jsonb,
    'deadbeef') AS aid2 \gset
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

ROLLBACK;
