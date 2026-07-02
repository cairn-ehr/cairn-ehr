\set ON_ERROR_STOP on
-- db/020 apply_remote_event — grant-floor and legible-rejection tests (issue #91).
-- Mirrors 005_submit_test.sql: self-checking DO blocks; positive-path coverage with
-- real signed events lives in crates/cairn-node/tests/apply_remote_event.rs.

-- The apply door exists and is executable by the sync runtime role (cairn_node):
-- a malformed event must fail on SIGNATURE, not on privilege.
DO $$ BEGIN
    SET LOCAL ROLE cairn_node;
    BEGIN
        PERFORM apply_remote_event('\xdeadbeef'::bytea);
        RESET ROLE;
        RAISE EXCEPTION 'FAILED: malformed remote event accepted';
    EXCEPTION
        WHEN insufficient_privilege THEN
            RESET ROLE;
            RAISE EXCEPTION 'FAILED: cairn_node cannot execute apply_remote_event (grant missing)';
        WHEN others THEN
            RESET ROLE;
            IF SQLERRM LIKE '%signature%' OR SQLERRM LIKE '%verif%' THEN
                RAISE NOTICE 'OK: cairn_node may knock; malformed refused legibly: %', SQLERRM;
            ELSE
                RAISE;
            END IF;
    END;
END $$;

-- The authoring agent role may NOT drive the replication door: an agent authors via
-- submit_event only; apply_remote_event is the sync plane's door (privilege gradient,
-- ADR-0021 §9.5).
DO $$ BEGIN
    SET LOCAL ROLE cairn_agent;
    BEGIN
        PERFORM apply_remote_event('\xdeadbeef'::bytea);
        RESET ROLE;
        RAISE EXCEPTION 'FAILED: cairn_agent may execute apply_remote_event';
    EXCEPTION WHEN insufficient_privilege THEN
        RESET ROLE;
        RAISE NOTICE 'OK: apply_remote_event denied to cairn_agent';
    END;
END $$;

-- The raw-INSERT floor still holds for the sync role: routing sync through the door
-- is meaningful only if the role cannot write event_log directly.
DO $$ BEGIN
    SET LOCAL ROLE cairn_node;
    BEGIN
        INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
            hlc_wall, hlc_counter, node_origin, signed_bytes, content_address, body,
            contributors, signer_key_id, plaintext_twin)
        VALUES (gen_random_uuid(), gen_random_uuid(), 'x','x',0,0,'n','\x00',
            '\x1220'||digest('\x00','sha256'), '{}','[]','k','t');
        RESET ROLE;
        RAISE EXCEPTION 'FAILED: cairn_node raw INSERT into event_log succeeded';
    EXCEPTION WHEN insufficient_privilege THEN
        RESET ROLE;
        RAISE NOTICE 'OK: raw INSERT into event_log denied to cairn_node';
    END;
END $$;
