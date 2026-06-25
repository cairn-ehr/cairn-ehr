-- Cairn — node-level supersede + self-trusting restore (ADR-0026 slice C).
--
-- WHY: slice B exports a node's signed node_event set to a cold-peer medium; this is
-- the APPLY half. Restoring a node's OWN history into a fresh DB cannot use the live
-- apply_remote_node_event gate (that is the PEER-admission path — it rejects events
-- whose author is not an already-trusted peer, which a fresh node has none of). So we
-- add a SELF-TRUSTING restore door, fenced so it is a permanent no-op on a live node,
-- plus the node-level `supersede` op (a restored node mints a NEW key — the signing key
-- is never backed up — and records supersede(dead -> new), already the actor-algebra
-- shape for agents, now applied to nodes). See ADR-0026 §7.10 points 1/2/4.

BEGIN;

-- (1) Widen the op CHECK additively (ADR-0012): a superset rejects nothing previously
-- accepted. The constraint is the auto-named column CHECK from db/007's CREATE TABLE.
ALTER TABLE node_event DROP CONSTRAINT IF EXISTS node_event_op_check;
ALTER TABLE node_event ADD CONSTRAINT node_event_op_check
    CHECK (op IN ('enroll','peer','revoke','supersede'));

-- (2) The supersede lineage view: who superseded whom. Read by `status`/audit. A
-- supersede event's author is the NEW (live) node; its subject is the dead node-id.
CREATE OR REPLACE VIEW node_lineage AS
SELECT ne.subject_node_id AS superseded_node_id,
       ne.author_node_id  AS new_node_id,
       ne.hlc_wall, ne.hlc_counter, ne.recorded_at
FROM node_event ne
WHERE ne.op = 'supersede';

GRANT SELECT ON node_lineage TO cairn_node;

-- (3) The self-trusting restore door. Unlike apply_remote_node_event (the PEER-admission
-- gate), this applies a node's OWN history into a fresh DB WITHOUT a peer-trust check —
-- a fresh node has no trust set yet. The danger (a federation-admission bypass) is closed
-- structurally: the door fails closed unless local_node is empty, so on any LIVE node it
-- is a permanent no-op. Signature + content-address ARE enforced, so a tampered/bit-rotted
-- medium event is rejected exactly as a hostile peer would be (ADR-0026 point 2). The door
-- NEVER writes local_node — only a real new genesis (submit_node_event) does, and that is
-- what permanently fences this door closed at the end of a restore.
CREATE OR REPLACE FUNCTION restore_node_event(p_signed BYTEA)
RETURNS UUID
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public
AS $$
DECLARE
    b JSONB; v_type TEXT; v_op TEXT; v_ca BYTEA; v_eid UUID; v_signer TEXT;
    v_payload JSONB; v_author_node BYTEA; v_subject BYTEA;
BEGIN
    -- FENCE: restore is only into a fresh, un-enrolled node.
    IF EXISTS (SELECT 1 FROM local_node WHERE id) THEN
        RAISE EXCEPTION 'restore_node_event: node already enrolled; restore applies only into a fresh node (live admission is apply_remote_node_event)';
    END IF;
    IF NOT cairn_verify(p_signed) THEN
        RAISE EXCEPTION 'restore_node_event: signature verification failed';
    END IF;
    b := cairn_body(p_signed);
    v_type := b ->> 'event_type'; v_eid := (b ->> 'event_id')::uuid;
    v_signer := b ->> 'signer_key_id'; v_payload := b -> 'payload';
    v_ca := '\x1220'::bytea || digest(p_signed, 'sha256');
    v_op := CASE v_type WHEN 'node.enrolled' THEN 'enroll' WHEN 'peer.added' THEN 'peer'
                        WHEN 'peer.revoked' THEN 'revoke' WHEN 'node.superseded' THEN 'supersede'
                        ELSE NULL END;
    IF v_op IS NULL THEN
        RAISE EXCEPTION 'restore_node_event: unknown node event_type % (fail closed)', v_type;
    END IF;

    IF v_op = 'enroll' THEN
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (v_eid, 'enroll', v_ca, v_ca, v_signer,
            (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
            b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
        ON CONFLICT (node_event_id) DO NOTHING;
    ELSE
        -- The node's own enroll is restored first (medium seq order), so its key resolves.
        SELECT node_id INTO v_author_node FROM node_current WHERE signer_key_id = v_signer;
        IF v_author_node IS NULL THEN
            RAISE EXCEPTION 'restore_node_event: author key % maps to no restored enroll (apply genesis first)', v_signer;
        END IF;
        v_subject := CASE v_op
            WHEN 'supersede' THEN decode(v_payload ->> 'superseded_node_id_hex','hex')
            ELSE decode(v_payload ->> 'peer_node_id_hex','hex') END;
        IF v_subject IS NULL THEN
            RAISE EXCEPTION 'restore_node_event: % missing subject node id in payload', v_type;
        END IF;
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, peer_pubkey, fingerprint, role, scope_hint, target_event_id,
            hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (v_eid, v_op, v_author_node, v_subject,
            v_signer, v_payload ->> 'peer_pubkey', v_payload ->> 'fingerprint',
            v_payload ->> 'role', v_payload ->> 'scope_hint',
            NULLIF(v_payload ->> 'target_event_id','')::uuid,
            (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
            b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
        ON CONFLICT (node_event_id) DO NOTHING;
    END IF;
    -- Clock never falls behind a restored event (HLC invariant A3, mirrors the apply path).
    UPDATE hlc_state SET
        hlc_wall    = GREATEST(hlc_wall, (b -> 'hlc' ->> 'wall')::bigint),
        hlc_counter = CASE
            WHEN (b -> 'hlc' ->> 'wall')::bigint > hlc_wall THEN (b -> 'hlc' ->> 'counter')::int
            WHEN (b -> 'hlc' ->> 'wall')::bigint = hlc_wall THEN GREATEST(hlc_counter, (b -> 'hlc' ->> 'counter')::int)
            ELSE hlc_counter END
        WHERE id;
    RETURN v_eid;
END;
$$;

REVOKE EXECUTE ON FUNCTION restore_node_event(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION restore_node_event(bytea) TO cairn_node;

COMMIT;
