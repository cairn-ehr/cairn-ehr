-- Cairn — node identity & federation (ADR-0017). The actor-event algebra applied
-- to node-to-node relationships. Parallel to db/004 (actor_event): an append-only,
-- content-addressed, signed log of node enroll / peer / revoke events. node_id is
-- GENESIS-STABLE: it is the content-address of the genesis enroll event's signed
-- bytes (NOT the pinned-key hash db/004 uses for agents), so a future key rotation
-- keeps the node_id. Federation events reuse the cairn-event signed envelope
-- (nil patient, node.* type) but never touch the clinical event_log.

BEGIN;

CREATE TABLE IF NOT EXISTS node_event (
    node_event_id   UUID    PRIMARY KEY,            -- = body.event_id (UUIDv7), inside the signed bytes
    op              TEXT    NOT NULL CHECK (op IN ('enroll','peer','revoke')),
    author_node_id  BYTEA   NOT NULL,               -- node_id of the signer (self, for enroll)
    subject_node_id BYTEA   NOT NULL,               -- enroll: = author; peer/revoke: the peer
    signer_key_id   TEXT    NOT NULL,               -- hex Ed25519 public key of the author
    peer_pubkey     TEXT,                           -- peer/revoke: hex pubkey of the subject peer
    fingerprint     TEXT,                           -- peer: the operator-confirmed short fingerprint
    role            TEXT    CHECK (role IS NULL OR role IN ('upstream','downstream','peer')),
    scope_hint      TEXT,                           -- peer: optional default sync-scope label (ADR-0004)
    target_event_id UUID,                           -- revoke: the peer event it overlays
    hlc_wall        BIGINT  NOT NULL,
    hlc_counter     INTEGER NOT NULL,
    node_origin     TEXT    NOT NULL,
    signed_bytes    BYTEA   NOT NULL,
    content_address BYTEA   NOT NULL UNIQUE,
    recorded_at     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT node_event_content_addressed
        CHECK (content_address = '\x1220'::bytea || digest(signed_bytes, 'sha256')),
    CONSTRAINT node_event_hlc_nonneg CHECK (hlc_wall >= 0 AND hlc_counter >= 0)
);

CREATE INDEX IF NOT EXISTS node_event_signer_idx  ON node_event (signer_key_id);
CREATE INDEX IF NOT EXISTS node_event_subject_idx ON node_event (subject_node_id);

CREATE OR REPLACE FUNCTION node_event_is_append_only()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'node_event is append-only: % is not permitted (Cairn principle #1/#2)', TG_OP;
END;
$$;
DROP TRIGGER IF EXISTS node_event_no_update ON node_event;
CREATE TRIGGER node_event_no_update BEFORE UPDATE OR DELETE ON node_event
    FOR EACH ROW EXECUTE FUNCTION node_event_is_append_only();

-- Map a node's CURRENT signing key to its genesis node_id (latest enroll per node).
-- This is identity RESOLUTION, deliberately independent of peer TRUST: the `revoke`
-- op is a *peer-trust* revocation (subject = an un-trusted peer), NOT a node
-- decommission, so node_current intentionally still resolves an unpeered node's key
-- to its node_id; whether that node is an active peer is trust_peer's job, checked
-- separately by the admission gate. For v1 there is exactly one enroll per node_id.
CREATE OR REPLACE VIEW node_current AS
SELECT DISTINCT ON (ne.subject_node_id)
       ne.subject_node_id AS node_id, ne.signer_key_id, ne.recorded_at
FROM node_event ne
WHERE ne.op = 'enroll'
ORDER BY ne.subject_node_id, ne.recorded_at DESC;

-- This node's own identity (singleton). Set once by submit_node_event on genesis enroll.
CREATE TABLE IF NOT EXISTS local_node (
    id       BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    node_id  BYTEA NOT NULL,
    signer_key_id TEXT NOT NULL,
    address  TEXT
);
-- Additive-only evolution (ADR-0012): CREATE TABLE IF NOT EXISTS does not add a
-- column to an already-existing local_node, so patch it forward for nodes
-- provisioned before `address` existed.
ALTER TABLE local_node ADD COLUMN IF NOT EXISTS address TEXT;

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_node') THEN
        CREATE ROLE cairn_node NOLOGIN;
    END IF;
END $$;

-- The ONE local authoring door for node/peering events. Verifies in-DB, derives
-- op from event_type, and enforces: enroll is once-only and self; peer/revoke are
-- authored only by THIS node's current key. Every rejection is legible.
CREATE OR REPLACE FUNCTION submit_node_event(p_signed BYTEA)
RETURNS UUID
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public
AS $$
DECLARE
    b JSONB; v_type TEXT; v_op TEXT; v_ca BYTEA; v_eid UUID;
    v_local_node BYTEA; v_local_key TEXT; v_signer TEXT; v_payload JSONB;
BEGIN
    IF NOT cairn_verify(p_signed) THEN
        RAISE EXCEPTION 'submit_node_event: signature verification failed (unsigned or malformed)';
    END IF;
    b := cairn_body(p_signed);
    IF b IS NULL THEN
        RAISE EXCEPTION 'submit_node_event: body could not be parsed after verify';
    END IF;
    v_type   := b ->> 'event_type';
    v_eid    := (b ->> 'event_id')::uuid;
    v_signer := b ->> 'signer_key_id';
    v_payload := b -> 'payload';
    v_ca     := '\x1220'::bytea || digest(p_signed, 'sha256');
    v_op := CASE v_type
        WHEN 'node.enrolled' THEN 'enroll'
        WHEN 'peer.added'    THEN 'peer'
        WHEN 'peer.revoked'  THEN 'revoke'
        ELSE NULL END;
    IF v_op IS NULL THEN
        RAISE EXCEPTION 'submit_node_event: unknown node event_type % (fail closed)', v_type;
    END IF;

    SELECT node_id, signer_key_id INTO v_local_node, v_local_key FROM local_node WHERE id;

    IF v_op = 'enroll' THEN
        IF v_local_node IS NOT NULL THEN
            RAISE EXCEPTION 'submit_node_event: this node is already enrolled (genesis is once-only)';
        END IF;
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (v_eid, 'enroll', v_ca, v_ca, v_signer,
            (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
            b -> 'hlc' ->> 'node_origin', p_signed, v_ca);
        INSERT INTO local_node (id, node_id, signer_key_id, address) VALUES (TRUE, v_ca, v_signer, v_payload ->> 'address');
        RETURN v_eid;
    END IF;

    -- peer / revoke: authored only by this node's own current key.
    IF v_local_node IS NULL THEN
        RAISE EXCEPTION 'submit_node_event: node not yet enrolled; cannot author peering';
    END IF;
    IF v_signer <> v_local_key THEN
        RAISE EXCEPTION 'submit_node_event: peering may be authored only by this node (signer % != local %)', v_signer, v_local_key;
    END IF;
    -- subject_node_id is NOT NULL; a missing peer_node_id_hex would otherwise surface
    -- as an opaque constraint error rather than a legible rejection.
    IF v_payload ->> 'peer_node_id_hex' IS NULL THEN
        RAISE EXCEPTION 'submit_node_event: % missing peer_node_id_hex in payload', v_type;
    END IF;

    INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
        signer_key_id, peer_pubkey, fingerprint, role, scope_hint, target_event_id,
        hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
    VALUES (v_eid, v_op, v_local_node,
        decode(v_payload ->> 'peer_node_id_hex','hex'),
        v_signer, v_payload ->> 'peer_pubkey', v_payload ->> 'fingerprint',
        v_payload ->> 'role', v_payload ->> 'scope_hint',
        NULLIF(v_payload ->> 'target_event_id','')::uuid,
        (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
        b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
    ON CONFLICT (node_event_id) DO NOTHING;
    RETURN v_eid;
END;
$$;

-- The grant floor. This binds ONLY a connection that is NOT a superuser/table
-- owner: a superuser bypasses GRANT/REVOKE entirely and can raw-INSERT around the
-- submit/admission gate. So the "enforced in Postgres" guarantee holds iff the
-- RUNTIME connects as an unprivileged role — `cairn_node` is NOLOGIN, so deploy a
-- login role granted `cairn_node` and point the daemon at it. `init` (DDL) is the
-- only step that needs ownership. `status` reports whether the connected role can
-- still raw-INSERT (db_floor ENFORCED vs BYPASSABLE). (PR #28 review, finding 2.)
REVOKE INSERT, UPDATE, DELETE ON node_event FROM PUBLIC;
REVOKE INSERT, UPDATE, DELETE ON node_event FROM cairn_node;
REVOKE INSERT, UPDATE, DELETE ON local_node FROM PUBLIC, cairn_node;
REVOKE EXECUTE ON FUNCTION submit_node_event(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION submit_node_event(bytea) TO cairn_node;
GRANT SELECT ON node_event, node_current, local_node TO cairn_node;

-- The local node's trust set: peer assertions IT authored, graded active/revoked by
-- the latest op per subject. Read by the admission gate (Task 8) and the mTLS
-- cert-pin verifier (Task 9). A revoked peer is retained, never deleted (principle 2).
CREATE OR REPLACE VIEW trust_peer AS
SELECT DISTINCT ON (ne.subject_node_id)
       ne.subject_node_id AS peer_node_id,
       ne.peer_pubkey, ne.fingerprint, ne.role, ne.scope_hint,
       CASE ne.op WHEN 'revoke' THEN 'revoked' ELSE 'active' END AS status,
       ne.hlc_wall, ne.hlc_counter
FROM node_event ne
WHERE ne.op IN ('peer','revoke')
  AND ne.author_node_id = (SELECT node_id FROM local_node WHERE id)
ORDER BY ne.subject_node_id, ne.hlc_wall DESC, ne.hlc_counter DESC, ne.recorded_at DESC;

GRANT SELECT ON trust_peer TO cairn_node;

-- The federation admission seam (ADR-0017 §8): the one safety-critical gate. An
-- inbound, peer-authored node event enters the log only if it verifies AND its
-- author is an out-of-band-confirmed, currently-active peer. Reject is legible.
CREATE OR REPLACE FUNCTION apply_remote_node_event(p_signed BYTEA)
RETURNS UUID
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public
AS $$
DECLARE
    b JSONB; v_type TEXT; v_op TEXT; v_ca BYTEA; v_eid UUID; v_signer TEXT;
    v_payload JSONB; v_author_node BYTEA;
BEGIN
    IF NOT cairn_verify(p_signed) THEN
        RAISE EXCEPTION 'apply_remote_node_event: signature verification failed';
    END IF;
    b := cairn_body(p_signed);
    v_type := b ->> 'event_type'; v_eid := (b ->> 'event_id')::uuid;
    v_signer := b ->> 'signer_key_id'; v_payload := b -> 'payload';
    v_ca := '\x1220'::bytea || digest(p_signed, 'sha256');
    v_op := CASE v_type WHEN 'node.enrolled' THEN 'enroll' WHEN 'peer.added' THEN 'peer'
                        WHEN 'peer.revoked' THEN 'revoke' ELSE NULL END;
    IF v_op IS NULL THEN
        RAISE EXCEPTION 'apply_remote_node_event: unknown node event_type % (fail closed)', v_type;
    END IF;

    IF v_op = 'enroll' THEN
        -- The genesis must match an active, out-of-band-confirmed peer: its
        -- content-address is the node_id we trust, and its key is the pubkey we pinned.
        IF NOT EXISTS (SELECT 1 FROM trust_peer
                       WHERE peer_node_id = v_ca AND status = 'active' AND peer_pubkey = v_signer) THEN
            RAISE EXCEPTION 'apply_remote_node_event: genesis from an un-trusted or mismatched node (deny-all default)';
        END IF;
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (v_eid, 'enroll', v_ca, v_ca, v_signer,
            (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
            b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
        ON CONFLICT (node_event_id) DO NOTHING;
        RETURN v_eid;
    END IF;

    -- peer/revoke: the author must be a currently-trusted peer (resolved by key).
    SELECT node_id INTO v_author_node FROM node_current WHERE signer_key_id = v_signer;
    IF v_author_node IS NULL THEN
        RAISE EXCEPTION 'apply_remote_node_event: author key % maps to no known node', v_signer;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM trust_peer WHERE peer_node_id = v_author_node AND status = 'active') THEN
        RAISE EXCEPTION 'apply_remote_node_event: author % is not an active peer (deny-all)', encode(v_author_node,'hex');
    END IF;
    -- Mirror the local door's legible guard: a trusted-but-malformed peer event
    -- (missing peer_node_id_hex) is rejected, not stored with a \x00 subject.
    IF v_payload ->> 'peer_node_id_hex' IS NULL THEN
        RAISE EXCEPTION 'apply_remote_node_event: % from % missing peer_node_id_hex in payload', v_type, encode(v_author_node,'hex');
    END IF;
    INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
        signer_key_id, peer_pubkey, fingerprint, role, scope_hint, target_event_id,
        hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
    VALUES (v_eid, v_op, v_author_node,
        decode(v_payload ->> 'peer_node_id_hex','hex'),
        v_signer, v_payload ->> 'peer_pubkey', v_payload ->> 'fingerprint',
        v_payload ->> 'role', v_payload ->> 'scope_hint',
        NULLIF(v_payload ->> 'target_event_id','')::uuid,
        (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
        b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
    ON CONFLICT (node_event_id) DO NOTHING;
    RETURN v_eid;
END;
$$;

REVOKE EXECUTE ON FUNCTION apply_remote_node_event(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION apply_remote_node_event(bytea) TO cairn_node;

COMMIT;
