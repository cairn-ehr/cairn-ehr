\set ON_ERROR_STOP on
-- Helper: assert that a statement raises with a message matching a pattern.
-- Usage relies on DO blocks; each negative case below is self-checking.

-- C5.4: the agent role cannot raw-INSERT into event_log.
DO $$ BEGIN
    SET LOCAL ROLE cairn_agent;
    BEGIN
        INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
            hlc_wall, hlc_counter, node_origin, signed_bytes, content_address, body,
            contributors, signer_key_id, plaintext_twin)
        VALUES (gen_random_uuid(), gen_random_uuid(), 'x','x',0,0,'n','\x00','\x1220'||digest('\x00','sha256'),
            '{}','[]','k','t');
        RESET ROLE;
        RAISE EXCEPTION 'C5.4 FAILED: agent raw INSERT succeeded';
    EXCEPTION WHEN insufficient_privilege THEN
        RESET ROLE;
        RAISE NOTICE 'C5.4 OK: raw INSERT denied to cairn_agent';
    END;
END $$;

-- C5.1: submit_event rejects unsigned/malformed bytes with a legible reason.
DO $$ BEGIN
    BEGIN
        PERFORM submit_event('\xdeadbeef'::bytea);
        RAISE EXCEPTION 'C5.1 FAILED: malformed event accepted';
    EXCEPTION WHEN others THEN
        IF SQLERRM LIKE '%signature%' OR SQLERRM LIKE '%verify%'
            THEN RAISE NOTICE 'C5.1 OK: % ', SQLERRM; ELSE RAISE; END IF;
    END;
END $$;
