\set ON_ERROR_STOP on
-- ADR-0026 slice C — schema tests for the node-level supersede op + lineage view.
-- PURE SQL (no pgrx / no cairn_verify): inserts as the table OWNER straight into
-- node_event (the door REVOKEs bind cairn_node/PUBLIC, not the owner), so this
-- exercises the op CHECK constraint and the node_lineage view in isolation.
-- Run with: psql -v ON_ERROR_STOP=1 -f db/001..009 then this file.

-- Helper: a content-address that satisfies the 001/007 CHECK for given bytes.
-- node_event.content_address must equal '\x1220' || sha256(signed_bytes).

DO $$
DECLARE
    v_sb  bytea := convert_to('supersede-fixture', 'UTF8');
    v_ca  bytea := '\x1220'::bytea || digest(convert_to('supersede-fixture','UTF8'), 'sha256');
    v_old bytea := '\x1220'::bytea || digest(convert_to('old-node','UTF8'), 'sha256');
    v_new bytea := '\x1220'::bytea || digest(convert_to('new-node','UTF8'), 'sha256');
BEGIN
    -- The widened CHECK must ACCEPT op='supersede'.
    INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
        signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
    VALUES (gen_random_uuid(), 'supersede', v_new, v_old,
        'deadbeef', 1, 0, 'test', v_sb, v_ca);

    -- node_lineage must resolve the edge new <- old.
    PERFORM 1 FROM node_lineage
        WHERE superseded_node_id = v_old AND new_node_id = v_new;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'node_lineage did not resolve the supersede edge';
    END IF;
END $$;

-- The CHECK must still REJECT an unknown op (fail-closed).
DO $$
BEGIN
    BEGIN
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (gen_random_uuid(), 'bogus', '\x00', '\x00', 'k', 0, 0, 't',
            convert_to('x','UTF8'),
            '\x1220'::bytea || digest(convert_to('x','UTF8'),'sha256'));
        RAISE EXCEPTION 'op CHECK accepted an unknown op (should fail closed)';
    EXCEPTION WHEN check_violation THEN
        NULL; -- expected
    END;
END $$;

\echo '009_node_supersede_test: PASS'
