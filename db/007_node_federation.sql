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

-- Issue #38: a monotonic, node-LOCAL insertion-order key for incremental sync.
-- This is the watermark the puller cursors on (NOT the HLC and NOT recorded_at):
-- a node that newly LEARNS an event inserts it with a fresh high `seq`, so new
-- knowledge always sorts above any puller's cursor and can never be silently
-- skipped. `seq` is sync transport metadata only — never signed, never on the wire
-- core. Additive (ADR-0012): ADD COLUMN IF NOT EXISTS does not fire the append-only
-- row trigger (that fires on UPDATE/DELETE), and IDENTITY is assigned at INSERT so
-- the existing INSERT column lists need no change.
ALTER TABLE node_event ADD COLUMN IF NOT EXISTS seq BIGINT GENERATED ALWAYS AS IDENTITY;
CREATE INDEX IF NOT EXISTS node_event_seq_idx ON node_event (seq);

-- Issue #38: the per-peer pull checkpoint. `last_seq` is the highest serving-node
-- `seq` this node has pulled from `peer_addr`. MUTABLE node-local operational state
-- (not a signed event), so it lives OUTSIDE the append-only trigger. Keyed by peer
-- ADDRESS: the address is known before the connection (no protocol round-trip), and
-- a wrong/stale key can only cause a re-pull or a transient skip — both healed by the
-- full-sweep floor — never an incorrect admission.
CREATE TABLE IF NOT EXISTS sync_cursor (
    peer_addr  TEXT        PRIMARY KEY,
    last_seq   BIGINT      NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- The ONE door that writes sync_cursor. The runtime role gets EXECUTE on this, never
-- raw INSERT/UPDATE — preserving the floor invariant (PR #39): the cairn_node role does
-- zero raw DML, only validated doors. ADVANCE-ONLY (GREATEST): a buggy or hostile caller
-- cannot rewind the cursor to thrash re-pulls. Returns the resulting last_seq so the
-- caller can log/assert. A forward jump can only DELAY a legitimate event (healed by the
-- sweep), never admit an unauthorized one (the admission gate is untouched).
CREATE OR REPLACE FUNCTION checkpoint_sync_cursor(p_peer_addr TEXT, p_observed_seq BIGINT)
RETURNS BIGINT
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public
AS $$
DECLARE v_last BIGINT;
BEGIN
    INSERT INTO sync_cursor (peer_addr, last_seq, updated_at)
    VALUES (p_peer_addr, GREATEST(0, p_observed_seq), clock_timestamp())
    ON CONFLICT (peer_addr) DO UPDATE
        SET last_seq = GREATEST(sync_cursor.last_seq, EXCLUDED.last_seq),
            updated_at = clock_timestamp()
    RETURNING last_seq INTO v_last;
    RETURN v_last;
END;
$$;

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

-- Issue #38 (Gap 4): the node's local Hybrid Logical Clock. Mirrors cairn-sync's
-- hlc_state: a singleton row advanced on every authored event and merged forward on
-- every applied remote event, so the clock never falls behind anything in the log.
-- Replaces the 0/0 genesis placeholder, making trust_peer's HLC ordering real.
CREATE TABLE IF NOT EXISTS hlc_state (
    id          BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    hlc_wall    BIGINT  NOT NULL DEFAULT 0,
    hlc_counter INTEGER NOT NULL DEFAULT 0
);
INSERT INTO hlc_state (id) VALUES (TRUE) ON CONFLICT DO NOTHING;

-- Advance the local clock and return the new stamp. wall = max(prev_wall, now_ms);
-- counter resets to 0 when wall advances on wall-clock time, else increments (the
-- standard HLC tick). SECURITY DEFINER so the unprivileged runtime can tick via the
-- door without direct write to hlc_state.
CREATE OR REPLACE FUNCTION node_hlc_tick()
RETURNS TABLE(wall BIGINT, counter INTEGER)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public
AS $$
DECLARE
    v_now  BIGINT := (extract(epoch FROM clock_timestamp()) * 1000)::bigint;
    v_wall BIGINT; v_counter INTEGER;
BEGIN
    SELECT hlc_wall, hlc_counter INTO v_wall, v_counter FROM hlc_state WHERE id FOR UPDATE;
    IF v_now > v_wall THEN
        v_wall := v_now; v_counter := 0;
    ELSE
        v_counter := v_counter + 1;
    END IF;
    UPDATE hlc_state SET hlc_wall = v_wall, hlc_counter = v_counter WHERE id;
    wall := v_wall; counter := v_counter;
    RETURN NEXT;
END;
$$;

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
        WHEN 'node.superseded' THEN 'supersede'   -- ADR-0026 slice C
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
    -- supersede (ADR-0026 slice C): a restored node records that it succeeds a dead node.
    -- Authored by THIS node's current (new) key; subject = the superseded (dead) node-id.
    -- A distinct payload field (superseded_node_id_hex, not peer_node_id_hex) keeps the
    -- intent legible — the superseded node is NOT a peer.
    IF v_op = 'supersede' THEN
        IF v_payload ->> 'superseded_node_id_hex' IS NULL THEN
            RAISE EXCEPTION 'submit_node_event: node.superseded missing superseded_node_id_hex in payload';
        END IF;
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (v_eid, 'supersede', v_local_node,
            decode(v_payload ->> 'superseded_node_id_hex','hex'),
            v_signer, (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
            b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
        ON CONFLICT (node_event_id) DO NOTHING;
        RETURN v_eid;
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

-- sync_cursor: SELECT (for status/debug) but NO raw DML — writes go through the door.
GRANT SELECT ON sync_cursor TO cairn_node;
REVOKE INSERT, UPDATE, DELETE ON sync_cursor FROM PUBLIC, cairn_node;
REVOKE EXECUTE ON FUNCTION checkpoint_sync_cursor(text, bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION checkpoint_sync_cursor(text, bigint) TO cairn_node;

-- hlc_state: the runtime ticks via the door only — never raw DML on the table.
REVOKE EXECUTE ON FUNCTION node_hlc_tick() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION node_hlc_tick() TO cairn_node;

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
        -- Clock never falls behind an event we accepted (HLC invariant A3, mirrors cairn-sync).
        UPDATE hlc_state SET
            hlc_wall    = GREATEST(hlc_wall, (b -> 'hlc' ->> 'wall')::bigint),
            hlc_counter = CASE
                WHEN (b -> 'hlc' ->> 'wall')::bigint > hlc_wall THEN (b -> 'hlc' ->> 'counter')::int
                WHEN (b -> 'hlc' ->> 'wall')::bigint = hlc_wall THEN GREATEST(hlc_counter, (b -> 'hlc' ->> 'counter')::int)
                ELSE hlc_counter END
            WHERE id;
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
    -- Clock never falls behind an event we accepted (HLC invariant A3, mirrors cairn-sync).
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

REVOKE EXECUTE ON FUNCTION apply_remote_node_event(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION apply_remote_node_event(bytea) TO cairn_node;

COMMIT;
