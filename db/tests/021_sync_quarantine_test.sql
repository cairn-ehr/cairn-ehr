\set ON_ERROR_STOP on
-- db/021 sync_quarantine — grant-floor tests (issue #108).
-- Mirrors 020_apply_remote_event_test.sql: self-checking DO blocks. Behavioral
-- coverage (quarantine on pull, dedupe, loud all-unverifiable failure, requeue
-- through the apply door) lives in crates/cairn-sync/src/main.rs::quarantine_tests.

-- The sync runtime role holds the full quarantine lifecycle: INSERT (quarantine),
-- SELECT (inspect/requeue), UPDATE (re-offer bump / reason refresh), DELETE
-- (release after a successful requeue — legitimate here because admission itself
-- is still gated by apply_remote_event; a quarantine row is a *candidate*, not
-- clinical content).
DO $$ BEGIN
    SET LOCAL ROLE cairn_node;
    INSERT INTO sync_quarantine (content_digest, signed_bytes, peer, reason)
    VALUES ('\x0021', '\xdeadbeef', 'test-peer', 'grant-floor probe');
    UPDATE sync_quarantine SET seen_count = seen_count + 1
    WHERE content_digest = '\x0021';
    PERFORM 1 FROM sync_quarantine WHERE content_digest = '\x0021';
    DELETE FROM sync_quarantine WHERE content_digest = '\x0021';
    RESET ROLE;
    RAISE NOTICE 'OK: cairn_node holds the full quarantine lifecycle';
END $$;

-- The authoring agent role has NO business in the quarantine (it is sync-plane
-- operational state): the explicit REVOKE from PUBLIC must leave cairn_agent
-- with nothing — not even SELECT (quarantined bytes are unverified foreign
-- input; nothing should read them but the sync runtime and the owner).
DO $$ BEGIN
    SET LOCAL ROLE cairn_agent;
    BEGIN
        PERFORM 1 FROM sync_quarantine;
        RESET ROLE;
        RAISE EXCEPTION 'FAILED: cairn_agent can read sync_quarantine';
    EXCEPTION WHEN insufficient_privilege THEN
        RESET ROLE;
        RAISE NOTICE 'OK: sync_quarantine SELECT denied to cairn_agent';
    END;
END $$;

DO $$ BEGIN
    SET LOCAL ROLE cairn_agent;
    BEGIN
        INSERT INTO sync_quarantine (content_digest, signed_bytes, peer, reason)
        VALUES ('\x0022', '\xdeadbeef', 'test-peer', 'should be denied');
        RESET ROLE;
        RAISE EXCEPTION 'FAILED: cairn_agent can write sync_quarantine';
    EXCEPTION WHEN insufficient_privilege THEN
        RESET ROLE;
        RAISE NOTICE 'OK: sync_quarantine INSERT denied to cairn_agent';
    END;
END $$;
