-- db/tests/001_hex_decode_helper_test.sql — SQL mirror of the issue #228 guards.
--
-- The three node-plane doors that read a hex node-id out of an event payload
-- (submit_node_event and apply_remote_node_event in db/007, restore_node_event in
-- db/009) used to guard the NULL case legibly and then hand the malformed case
-- straight to decode(), which raises PostgreSQL's own "invalid hexadecimal digit"
-- with no door, no field and no author. Every rejection is legible (db/007 header)
-- — so the decode now routes through cairn_decode_hex_or_raise (db/001).
--
-- It was also a SYNC STALL, which the issue did not know: sync.rs classifies a door's
-- refusal by SQLSTATE, treating P0001 as a deliberate deny-all to skip past and any
-- other code as a possible transient fault to FREEZE on. decode()'s 22-class error
-- took the freeze arm, so one malformed field from a trusted peer stalled node-plane
-- pull from that peer permanently. Hence the sqlstate guard below.
--
-- The Rust suite crates/cairn-node/tests/hex_decode_helper.rs carries the same
-- assertions and adds two SOURCE-level guards this file cannot express, because they
-- read the migration text rather than the loaded schema: that the helper is declared
-- only in db/001, and that all six call sites still call it. It also drives the three
-- real doors end-to-end, which needs signed events this file cannot mint.
--
-- Mirrored here so the properties are checked by scripts/run-db-sql-tests.sh too —
-- the lesson of PR #182, where a guard living in only one of the two places drifted.
--
-- HOUSE PATTERN for the blocks below: each case sets a flag inside the inner
-- BEGIN … EXCEPTION block and asserts on it AFTER the block closes. The tempting
-- shorter form — a sentinel `RAISE EXCEPTION 'was accepted'` as the last statement
-- inside the block — is caught by that same block's own handler, so the sentinel has
-- to be distinguished from the real refusal by its message. That is fragile and reads
-- as a false pass. Assert outside the handler's reach instead.
\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
BEGIN;

-- A malformed value names the DOOR, the FIELD and the reason — the three things
-- PostgreSQL's own hex error omits. The three shapes are the three ways real payloads
-- go wrong: a language that writes hex with a 0x prefix, a value truncated to an odd
-- number of nibbles, and a value that is not hex at all.
DO $$
DECLARE
    c RECORD;
    v_msg TEXT;
BEGIN
    FOR c IN SELECT * FROM (VALUES
        ('0xABC', 'a 0x-prefixed value'),
        ('abc',   'an odd number of nibbles'),
        ('zzzz',  'a non-hex digit')
    ) AS t(val, why)
    LOOP
        v_msg := NULL;
        BEGIN
            PERFORM cairn_decode_hex_or_raise('peer_node_id_hex', c.val, 'submit_node_event');
        EXCEPTION WHEN others THEN
            v_msg := SQLERRM;
        END;
        IF v_msg IS NULL THEN
            RAISE EXCEPTION 'legibility FAILED: % was accepted as hex', c.why;
        END IF;
        IF v_msg NOT LIKE '%submit_node_event%'
           OR v_msg NOT LIKE '%peer_node_id_hex%'
           OR v_msg NOT LIKE '%not valid hex%' THEN
            RAISE EXCEPTION 'legibility FAILED for % — refusal must name door, field and reason; got: %',
                c.why, v_msg;
        END IF;
        RAISE NOTICE 'legibility OK (%): %', c.why, v_msg;
    END LOOP;
END $$;

-- The refusal must carry P0001, the code sync.rs reads as "deliberate refusal, skip past
-- it". Anything else — notably decode()'s own 22-class hex error — puts the pull loop on
-- its FREEZE arm, where the cursor stops below that seq and the same event is re-fetched
-- and re-frozen every cycle: node-plane sync from that peer stalls permanently, logged as
-- "transient?". That was the pre-#228 behaviour. The regression this guards is a
-- well-meaning `USING ERRCODE = SQLSTATE` added to the helper's raise; every message
-- assertion in this file would stay green through it.
DO $$
DECLARE
    c RECORD;
    v_state TEXT;
BEGIN
    FOR c IN SELECT * FROM (VALUES
        ('0xABC', 'a malformed value'),
        (NULL,    'a missing value')
    ) AS t(val, why)
    LOOP
        v_state := NULL;
        BEGIN
            PERFORM cairn_decode_hex_or_raise('peer_node_id_hex', c.val, 'apply_remote_node_event');
        EXCEPTION WHEN others THEN
            v_state := SQLSTATE;
        END;
        IF v_state IS NULL THEN
            RAISE EXCEPTION 'sqlstate-guard FAILED: % was accepted', c.why;
        END IF;
        IF v_state <> 'P0001' THEN
            RAISE EXCEPTION 'sqlstate-guard FAILED for % — refusal must be P0001 (skip-and-advance), got %; that freezes the peer''s cursor forever',
                c.why, v_state;
        END IF;
        RAISE NOTICE 'sqlstate-guard OK (%): P0001', c.why;
    END LOOP;
END $$;

-- The refusal characterises the value (length + short prefix) rather than echoing it.
-- Node-ids carry nothing secret, so this is not a leak fix today — it is the habit that
-- keeps it from becoming one when a later door decodes a key or a wrapped DEK, since
-- door errors land in logs that outlive the session.
DO $$
DECLARE
    v_val TEXT := repeat('aabbccdd', 5) || 'zz';   -- 42 chars, invalid hex
    v_msg TEXT;
BEGIN
    BEGIN
        PERFORM cairn_decode_hex_or_raise('superseded_node_id_hex', v_val, 'restore_node_event');
    EXCEPTION WHEN others THEN
        v_msg := SQLERRM;
    END;
    IF v_msg IS NULL THEN
        RAISE EXCEPTION 'value-characterisation FAILED: an invalid value was accepted';
    END IF;
    IF strpos(v_msg, v_val) > 0 THEN
        RAISE EXCEPTION 'value-characterisation FAILED: the refusal echoed the whole value: %', v_msg;
    END IF;
    IF v_msg NOT LIKE '%42%' OR v_msg NOT LIKE '%aabbccdd%' THEN
        RAISE EXCEPTION 'value-characterisation FAILED: refusal must report length and a short prefix; got: %', v_msg;
    END IF;
    RAISE NOTICE 'value-characterisation OK: %', v_msg;
END $$;

-- NULL fails closed, by field name. Not reachable from db/007's doors (they keep their
-- own richer NULL guards, which can also name the authoring peer) but reachable from
-- db/009 — and it is why the helper must never be declared STRICT, which would return
-- NULL on NULL input and hand a NOT NULL column an opaque constraint error instead.
DO $$
DECLARE v_msg TEXT;
BEGIN
    BEGIN
        PERFORM cairn_decode_hex_or_raise('peer_node_id_hex', NULL, 'restore_node_event');
    EXCEPTION WHEN others THEN
        v_msg := SQLERRM;
    END;
    IF v_msg IS NULL THEN
        RAISE EXCEPTION 'null-guard FAILED: a NULL value was accepted';
    END IF;
    IF v_msg NOT LIKE '%peer_node_id_hex%' OR v_msg NOT LIKE '%missing%' THEN
        RAISE EXCEPTION 'null-guard FAILED: refusal must name the field; got: %', v_msg;
    END IF;
    RAISE NOTICE 'null-guard OK: %', v_msg;
END $$;

-- The happy path is byte-for-byte decode(v,'hex'): the whole change is a no-op for every
-- well-formed event on the wire, and a difference here would mean the subject node-id the
-- doors store had changed. Mixed case is included because peers do not agree on it.
DO $$
DECLARE v TEXT;
BEGIN
    FOREACH v IN ARRAY ARRAY['', 'deadbeef', '1220AABB', repeat('ab', 33)]
    LOOP
        IF cairn_decode_hex_or_raise('f', v, 'd') IS DISTINCT FROM decode(v, 'hex') THEN
            RAISE EXCEPTION 'passthrough FAILED: % did not decode as decode() does', v;
        END IF;
    END LOOP;
    RAISE NOTICE 'passthrough OK: valid hex decodes exactly as before';
END $$;

ROLLBACK;
