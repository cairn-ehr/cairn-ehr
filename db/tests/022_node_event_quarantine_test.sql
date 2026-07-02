\set ON_ERROR_STOP on
-- db/022 node_event_quarantine — grant-floor tests (issue #111). Mirrors
-- 021_sync_quarantine_test.sql: self-checking DO blocks. Behavioral coverage
-- (quarantine on pull, dedupe, derived floor re-offer, auto-release on success,
-- ack) lives in crates/cairn-node/tests/node_quarantine.rs.

-- The node runtime role holds the full quarantine lifecycle: INSERT (pen),
-- SELECT (inspect / floor query), UPDATE (re-offer seen_count bump / ack),
-- DELETE (auto-release once the bytes apply). DELETE is legitimate here because
-- admission itself is still gated by apply_remote_node_event; a row is a
-- *candidate*, never clinical/federation content.
DO $$ BEGIN
    SET LOCAL ROLE cairn_node;
    INSERT INTO node_event_quarantine (content_digest, signed_bytes, peer, refused_seq, reason)
    VALUES ('\x0022', '\xdeadbeef', '127.0.0.1:7843', 5, 'grant-floor probe');
    UPDATE node_event_quarantine
       SET seen_count = seen_count + 1,
           last_seen  = clock_timestamp(),
           acked      = TRUE
     WHERE content_digest = '\x0022';
    PERFORM 1 FROM node_event_quarantine WHERE content_digest = '\x0022';
    DELETE FROM node_event_quarantine WHERE content_digest = '\x0022';
    RESET ROLE;
    RAISE NOTICE 'OK: cairn_node holds the full node_event_quarantine lifecycle';
END $$;

-- The authoring agent role has NO business in the node-plane quarantine (it is
-- sync-plane operational state over unverified foreign bytes): the explicit
-- REVOKE from PUBLIC must leave cairn_agent with nothing — not even SELECT.
DO $$ BEGIN
    SET LOCAL ROLE cairn_agent;
    BEGIN
        PERFORM 1 FROM node_event_quarantine;
        RESET ROLE;
        RAISE EXCEPTION 'FAILED: cairn_agent can read node_event_quarantine';
    EXCEPTION WHEN insufficient_privilege THEN
        RESET ROLE;
        RAISE NOTICE 'OK: node_event_quarantine denied to cairn_agent';
    END;
END $$;
