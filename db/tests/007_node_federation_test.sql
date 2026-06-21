\set ON_ERROR_STOP on
BEGIN;

-- A genesis enroll row maps its signer key to its node_id (= content_address).
INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
    signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
VALUES (gen_random_uuid(), 'enroll',
    '\x1220'||digest('A','sha256'), '\x1220'||digest('A','sha256'),
    'aakey', 0, 0, 'A', 'A', '\x1220'||digest('A','sha256'));

SELECT (node_id = '\x1220'||digest('A','sha256')) AS node_current_maps_key
FROM node_current WHERE signer_key_id = 'aakey';

-- The content-address invariant rejects a row whose advertised address lies.
DO $$ BEGIN
    BEGIN
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (gen_random_uuid(), 'enroll', '\x00','\x00','k',0,0,'X','realbytes','\x1220'||digest('LIE','sha256'));
        RAISE EXCEPTION 'content-address CHECK FAILED: mismatched row accepted';
    EXCEPTION WHEN check_violation THEN RAISE NOTICE 'content-address CHECK OK'; END;
END $$;

-- Append-only: UPDATE/DELETE must raise.
DO $$ BEGIN
    BEGIN
        UPDATE node_event SET role = 'x';
        RAISE EXCEPTION 'append-only FAILED';
    EXCEPTION WHEN others THEN
        IF SQLERRM LIKE '%append-only%' THEN RAISE NOTICE 'append-only OK'; ELSE RAISE; END IF;
    END;
END $$;

-- C7.1: cairn_node may not raw-INSERT into node_event (grant floor).
DO $$ BEGIN
    SET LOCAL ROLE cairn_node;
    BEGIN
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (gen_random_uuid(),'enroll','\x00','\x00','k',0,0,'X','b','\x1220'||digest('b','sha256'));
        RESET ROLE; RAISE EXCEPTION 'grant-floor FAILED: raw INSERT succeeded';
    EXCEPTION WHEN insufficient_privilege THEN RESET ROLE; RAISE NOTICE 'grant-floor OK'; END;
END $$;

-- C7.1b: the floor-detection predicate status() reports on is honest — the
-- unprivileged runtime role cannot raw-INSERT (floor binds), so a node connected as
-- cairn_node would show db_floor=ENFORCED. (PR #28 review, finding 2.)
DO $$ BEGIN
    IF has_table_privilege('cairn_node','node_event','INSERT') THEN
        RAISE EXCEPTION 'floor-detect FAILED: cairn_node can raw-INSERT node_event';
    END IF;
    RAISE NOTICE 'floor-detect OK: cairn_node cannot raw-INSERT (db_floor would be ENFORCED)';
END $$;

-- C7.2: submit_node_event rejects unsigned/malformed bytes with a legible reason (fail closed).
DO $$ BEGIN
    BEGIN
        PERFORM submit_node_event('\xdeadbeef'::bytea);
        RAISE EXCEPTION 'fail-closed FAILED: malformed node event accepted';
    EXCEPTION WHEN others THEN
        IF SQLERRM LIKE '%signature%' OR SQLERRM LIKE '%verify%'
            THEN RAISE NOTICE 'fail-closed OK: %', SQLERRM; ELSE RAISE; END IF;
    END;
END $$;

-- Seed a local node + a peer + then revoke it; trust_peer reflects active->revoked.
INSERT INTO local_node (id, node_id, signer_key_id) VALUES (TRUE, '\x1220'||digest('SELF','sha256'), 'selfkey')
    ON CONFLICT (id) DO NOTHING;
INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id, signer_key_id,
    peer_pubkey, fingerprint, role, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
VALUES (gen_random_uuid(),'peer', '\x1220'||digest('SELF','sha256'), '\x1220'||digest('P','sha256'),
    'selfkey','pkey','AAAA-BBBB-CCCC-DDDD-EEEE','peer',1,0,'SELF','p1','\x1220'||digest('p1','sha256'));
SELECT (status = 'active') AS peer_is_active FROM trust_peer WHERE peer_node_id = '\x1220'||digest('P','sha256');

INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id, signer_key_id,
    hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
VALUES (gen_random_uuid(),'revoke', '\x1220'||digest('SELF','sha256'), '\x1220'||digest('P','sha256'),
    'selfkey',2,0,'SELF','p2','\x1220'||digest('p2','sha256'));
SELECT (status = 'revoked') AS peer_is_revoked FROM trust_peer WHERE peer_node_id = '\x1220'||digest('P','sha256');

-- A well-formed but UNSIGNED blob is rejected by the admission gate (fail closed).
DO $$ BEGIN
    BEGIN
        PERFORM apply_remote_node_event('\xdeadbeef'::bytea);
        RAISE EXCEPTION 'admission FAILED: malformed remote event accepted';
    EXCEPTION WHEN others THEN
        IF SQLERRM LIKE '%verify%' OR SQLERRM LIKE '%signature%'
            THEN RAISE NOTICE 'admission fail-closed OK'; ELSE RAISE; END IF;
    END;
END $$;

ROLLBACK;
