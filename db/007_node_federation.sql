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
    role            TEXT,                           -- vocabulary: see node_event_role_check below
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

-- The peer-role vocabulary, in ONE place (#621). Both the table's CHECK and the doors'
-- refusal read it here, because a list written twice is a list that drifts — and the two
-- directions of drift are not symmetric. A door that accepts a role the CHECK rejects raises
-- `23514`, which the node puller cannot tell from a deadlock, so it freezes that peer's cursor
-- permanently; a door that refuses a role the CHECK would accept merely refuses, legibly.
--
-- Widening this function is how the vocabulary grows (additive-only, principle 11). An OLDER
-- node meeting the new value then refuses it with P0001 and SKIPS it — re-offered on every full
-- sweep and admitted the day that node is upgraded — instead of wedging the link. That is the
-- whole reason the vocabulary is enforced at the door at all.
-- THE TWO CLAUSES ARE DELIBERATE, and so is the fact that they are the same two every other
-- helper in db/001 and db/007 carries (PR #627 review). `SET search_path` is not needed by this
-- body — an array of literals resolves nothing — but it is kept for two reasons beyond habit:
-- this function is resident inside a CHECK constraint on an append-only table, which is the last
-- place in the tree you want a loosely-specified function, and whatever the vocabulary GROWS into
-- (ADR-0074 makes widening it the growth path) inherits whatever is written here. The `SET` also
-- blocks SQL-function inlining, which matters: without it a zero-arg IMMUTABLE SQL function is
-- constant-folded into the cached constraint expression, so a backend could in principle hold the
-- OLD array after a widening and reject at the CHECK what the door just accepted — a `23514`,
-- which is precisely the code this whole slice exists to stop the doors producing. (The window is
-- nil today, because the DROP/ADD pair below ships a relcache invalidation at the same moment the
-- function is replaced. Do not remove that pair behind an "it already exists" guard without
-- re-reading this paragraph.)
--
-- The REVOKE is safe because nothing can reach this function as a non-owner: `node_event` grants
-- `cairn_node` SELECT only (see the grants at the foot of this file), and all three writers are
-- SECURITY DEFINER, so the CHECK always evaluates as the owner. It would NOT be safe if a
-- non-owner role were ever granted INSERT on node_event directly — a CHECK constraint executes
-- its functions as the INSERTING user, so the raw-SQL floor's honest `23514` would become a
-- `42501`, which the puller reads as this node's own fault and FREEZES on. If you ever widen the
-- grants on node_event, revisit this REVOKE first.
CREATE OR REPLACE FUNCTION cairn_node_roles()
RETURNS TEXT[] LANGUAGE sql IMMUTABLE
SET search_path = public, pg_temp
AS $$
    SELECT ARRAY['upstream', 'downstream', 'peer'];
$$;
REVOKE EXECUTE ON FUNCTION cairn_node_roles() FROM PUBLIC;

-- Re-point the CHECK at the vocabulary function, idempotently — db/009's `op` DROP/ADD pair is
-- the precedent. `CREATE TABLE IF NOT EXISTS` above cannot change an existing table's constraint,
-- so an in-place constraint change needs this paired ALTER (#207) or it lands only on fresh
-- databases.
--
-- NOT VALID, and that word is load-bearing (PR #627 review, finding 4). connect_and_load_schema
-- replays EVERY migration on EVERY connect, and a plain ADD CONSTRAINT re-scans the whole table
-- each time. The inline CHECK it replaces never re-validated anything after CREATE TABLE, so a
-- validating pair would give node_event a property it never had: a single stored row that does not
-- satisfy today's vocabulary makes this statement raise, db/007 abort, and the node refuse to
-- START — before an operator can reach the database to widen cairn_node_roles() again or drop the
-- constraint. (Those two ARE the repair, and they are one line each; the earlier claim here that
-- the only way out was disabling the append-only trigger and deleting a signed event was wrong —
-- PR #627 review. The decision does not rest on it. Refusing to START is not a state a fleet node
-- may enter over a vocabulary it once admitted, however easy the repair is to type, because the
-- node is what an operator would be typing it into.) That row is exactly what a downgrade after a
-- vocabulary widening leaves behind, and widening cairn_node_roles() IS how the vocabulary grows
-- (ADR-0074). NOT VALID still checks every NEW row, which is the whole
-- point of keeping the constraint (principle 12's floor); it only declines to re-litigate history
-- this node already admitted.
ALTER TABLE node_event DROP CONSTRAINT IF EXISTS node_event_role_check;
ALTER TABLE node_event ADD CONSTRAINT node_event_role_check
    CHECK (role IS NULL OR role = ANY (cairn_node_roles())) NOT VALID;

-- A peer role, or a LEGIBLE P0001 refusal — the raising face of the vocabulary above (#621).
-- NULL passes: role is optional on the wire, and the CHECK says so too.
--
-- THE VALUE IS ECHOED IN FULL, which is the deliberate exception to cairn_value_glimpse's habit
-- (PR #627 review, finding 6). Be precise about WHY, because the tempting reason is circular: it
-- is NOT that the vocabulary is closed and public. The string echoed here is by definition one
-- that is NOT in the vocabulary — it is arbitrary peer-controlled text out of
-- `v_payload ->> 'role'`. The justification is that a wire-level ROUTING LABEL carries nothing
-- secret by construction (unlike the key, token or wrapped DEK the glimpse habit exists to
-- protect), and that 64 characters is too small to be an exfiltration or log-flooding channel.
-- Note it reaches the PostgreSQL server log verbatim, newlines included, so a peer can forge a
-- line THERE; the puller's own copy is flattened by Rust's `one_line()`. Glimpsing it produced
-- `role "ups..." is not one of ... (upstream, downstream, peer)`, which hides the operator's own
-- typo and, in the cross-version case this guard exists for, hides WHICH new vocabulary member
-- the peer is using. Bounded at 64 characters so a hostile 8 MB field cannot fill the log, and the
-- ellipsis is added only when something really was cut.
CREATE OR REPLACE FUNCTION cairn_node_role_or_raise(p_role TEXT, p_door TEXT)
RETURNS TEXT
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    IF p_role IS NOT NULL AND NOT (p_role = ANY (cairn_node_roles())) THEN
        RAISE EXCEPTION '%: role "%" is not one of this node''s known peer roles (%)',
            p_door,
            left(p_role, 64) || CASE WHEN length(p_role) > 64 THEN '...' ELSE '' END,
            array_to_string(cairn_node_roles(), ', ');
    END IF;
    RETURN p_role;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_node_role_or_raise(text, text) FROM PUBLIC;

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
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp
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
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp
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
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp
AS $$
DECLARE
    b JSONB; v_type TEXT; v_op TEXT; v_ca BYTEA; v_eid UUID;
    v_local_node BYTEA; v_local_key TEXT; v_signer TEXT; v_payload JSONB;
    v_found BYTEA;
BEGIN
    -- Size ceiling (review fix A7a): an oversized event would wedge the read-capped wire.
    IF octet_length(p_signed) > cairn_max_event_bytes() THEN
        RAISE EXCEPTION 'submit_node_event: event is % bytes, over the % -byte admission ceiling',
            octet_length(p_signed), cairn_max_event_bytes();
    END IF;
    IF NOT cairn_verify(p_signed) THEN
        -- Legible reason as DETAIL (issue #109): tells a wire-format skew apart from tampering.
        RAISE EXCEPTION 'submit_node_event: signature verification failed (unsigned or malformed)'
            USING DETAIL = coalesce(cairn_verify_error(p_signed), 'unknown');
    END IF;
    b := cairn_body(p_signed);
    IF b IS NULL THEN
        RAISE EXCEPTION 'submit_node_event: body could not be parsed after verify';
    END IF;
    v_type   := b ->> 'event_type';
    -- Every field this door CASTS out of caller-supplied bytes is validated through a helper that
    -- raises P0001 (#621). A bare `::uuid` here raises 22P02, which the node puller reads as "the
    -- door never decided" and answers by FREEZING that peer's cursor forever.
    --
    -- THE PREMISE UNDER THE REMAINING CASTS, since the sentence above used to overclaim (PR #627
    -- review, finding 2): `(b -> 'hlc' ->> 'wall')::bigint`, `… 'counter')::int` and the NOT NULL
    -- `node_origin` / `signer_key_id` are safe because `cairn_body` (extensions/cairn_pgx) parses
    -- the body THROUGH the typed `cairn_event::EventBody`, whose `Hlc` types those fields
    -- `i64`/`i32`/`String` with no serde default — so a body that cannot produce them fails CBOR
    -- decode, fails verification, and never reaches here.
    --
    -- That is a guarantee TWO CRATES away, and the fragile half is not the field types: it is
    -- `cairn_body`'s TYPED parse. A future forward-compatible parse to a generic CBOR value
    -- (ADR-0012 additive evolution, ADR-0056 admit-uninterpreted — both live themes) would break
    -- these casts with `Hlc` untouched. If either half moves, these casts need `cairn_*_or_raise`
    -- too. `node_door_input_guards.rs::the_hlc_casts_rest_on_cairn_events_types` pins both halves.
    v_eid    := cairn_uuid_or_raise('event_id', b ->> 'event_id', 'submit_node_event');
    v_signer := b ->> 'signer_key_id';
    v_payload := b -> 'payload';
    v_ca     := '\x1220'::bytea || digest(p_signed, 'sha256');
    -- The clock, before the INSERT meets node_event_hlc_nonneg (a 23514 CHECK violation, which
    -- freezes the same way). The drift ceiling in the remote door bounds the wall from ABOVE
    -- only, so a negative clock is a different question and gets its own guard.
    PERFORM cairn_hlc_nonneg_or_raise((b -> 'hlc' ->> 'wall')::bigint,
                                      (b -> 'hlc' ->> 'counter')::int, 'submit_node_event');
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

    -- peer / revoke / supersede: authored only by this node's own current key.
    IF v_local_node IS NULL THEN
        RAISE EXCEPTION 'submit_node_event: node not yet enrolled; cannot author peering';
    END IF;
    IF v_signer <> v_local_key THEN
        RAISE EXCEPTION 'submit_node_event: peering may be authored only by this node (signer % != local %)', v_signer, v_local_key;
    END IF;
    -- supersede (ADR-0026 slice C): a restored node records that it succeeds a dead node.
    -- Authored by THIS node's current (new) key; subject = the superseded (dead) node-id.
    -- A distinct payload field (superseded_node_id_hex, not peer_node_id_hex) keeps the
    -- intent legible — the superseded node is NOT a peer. Lineage REPLICATES: peers
    -- admit this event through apply_remote_node_event's supersede arm (issue #201),
    -- so the claim reaches every node holding the restored node's history.
    IF v_op = 'supersede' THEN
        IF v_payload ->> 'superseded_node_id_hex' IS NULL THEN
            RAISE EXCEPTION 'submit_node_event: node.superseded missing superseded_node_id_hex in payload';
        END IF;
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (v_eid, 'supersede', v_local_node,
            cairn_decode_hex_or_raise('superseded_node_id_hex',
                v_payload ->> 'superseded_node_id_hex', 'submit_node_event'),
            v_signer, (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
            b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
        ON CONFLICT (node_event_id) DO NOTHING;
    ELSE
        -- subject_node_id is NOT NULL; a missing peer_node_id_hex would otherwise surface
        -- as an opaque constraint error rather than a legible rejection. The sibling case —
        -- present but MALFORMED — is caught inside cairn_decode_hex_or_raise below (issue
        -- #228). This guard is kept rather than folded into the helper because it names
        -- v_type, which the helper cannot see.
        IF v_payload ->> 'peer_node_id_hex' IS NULL THEN
            RAISE EXCEPTION 'submit_node_event: % missing peer_node_id_hex in payload', v_type;
        END IF;

        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, peer_pubkey, fingerprint, role, scope_hint, target_event_id,
            hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (v_eid, v_op, v_local_node,
            cairn_decode_hex_or_raise('peer_node_id_hex',
                v_payload ->> 'peer_node_id_hex', 'submit_node_event'),
            v_signer, v_payload ->> 'peer_pubkey', v_payload ->> 'fingerprint',
            cairn_node_role_or_raise(v_payload ->> 'role', 'submit_node_event'),
            v_payload ->> 'scope_hint',
            -- Optional: absent or empty stays NULL, and only a PRESENT value is validated —
            -- which is why cairn_uuid_or_raise is not STRICT and the emptiness test is here.
            CASE WHEN NULLIF(v_payload ->> 'target_event_id','') IS NULL THEN NULL
                 ELSE cairn_uuid_or_raise('target_event_id',
                        v_payload ->> 'target_event_id', 'submit_node_event') END,
            (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
            b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
        ON CONFLICT (node_event_id) DO NOTHING;
    END IF;

    -- SUBSTITUTION REFUSAL (#619, ADR-0073). Both arms above insert ON CONFLICT DO NOTHING, which
    -- is right for a REPEAT of the same event (set-union) and silently wrong for a DIFFERENT event
    -- under an id already held: the rival vanished and this door returned the id as if it had
    -- succeeded — for a peer.revoked, the node kept trusting a peer it had revoked. The comparison
    -- is the shared cairn_refuse_substitution (db/053, IS DISTINCT FROM), never an inline copy
    -- (substitution_guard_is_single_source.rs).
    --
    -- Two placement rules, each of which a later edit might "tidy" away:
    --   * AFTER the IF/ELSE, never above it. Above the branch nothing is held yet, v_found is
    --     NULL, and IS DISTINCT FROM refuses — every clean write would be refused.
    --   * The read is UNCONDITIONAL — no GET DIAGNOSTICS ROW_COUNT. The node plane carries tens
    --     of events, and a ROW_COUNT check is only correct while each INSERT stays the last
    --     statement of its arm; a later edit would disarm it silently (db/009's rule, ADR-0072).
    -- The genesis arm above needs neither: it has no ON CONFLICT, so a colliding id raises.
    SELECT content_address INTO v_found FROM node_event WHERE node_event_id = v_eid;
    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'submit_node_event');
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
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp
AS $$
DECLARE
    b JSONB; v_type TEXT; v_op TEXT; v_ca BYTEA; v_eid UUID; v_signer TEXT;
    v_payload JSONB; v_author_node BYTEA;
    v_found BYTEA;
BEGIN
    -- Size ceiling (review fix A7a): refuse an oversized remote event at the gate; the
    -- server-side stream skips any legacy oversized row, but the admission door is the floor.
    IF octet_length(p_signed) > cairn_max_event_bytes() THEN
        RAISE EXCEPTION 'apply_remote_node_event: event is % bytes, over the % -byte admission ceiling',
            octet_length(p_signed), cairn_max_event_bytes();
    END IF;
    IF NOT cairn_verify(p_signed) THEN
        -- Legible reason as DETAIL (issue #109): tells a wire-format skew apart from tampering.
        RAISE EXCEPTION 'apply_remote_node_event: signature verification failed'
            USING DETAIL = coalesce(cairn_verify_error(p_signed), 'unknown');
    END IF;
    b := cairn_body(p_signed);
    v_type := b ->> 'event_type';
    -- P0001 for every malformed field, never PostgreSQL's own cast/CHECK code (#621): this is
    -- THE door the puller talks to, so a 22P02 or 23514 here is a permanently frozen link.
    v_eid := cairn_uuid_or_raise('event_id', b ->> 'event_id', 'apply_remote_node_event');
    v_signer := b ->> 'signer_key_id'; v_payload := b -> 'payload';
    v_ca := '\x1220'::bytea || digest(p_signed, 'sha256');
    PERFORM cairn_hlc_nonneg_or_raise((b -> 'hlc' ->> 'wall')::bigint,
                                      (b -> 'hlc' ->> 'counter')::int, 'apply_remote_node_event');
    v_op := CASE v_type WHEN 'node.enrolled' THEN 'enroll' WHEN 'peer.added' THEN 'peer'
                        WHEN 'peer.revoked' THEN 'revoke'
                        WHEN 'node.superseded' THEN 'supersede'  -- issue #201: lineage replicates
                        ELSE NULL END;
    IF v_op IS NULL THEN
        RAISE EXCEPTION 'apply_remote_node_event: unknown node event_type % (fail closed)', v_type;
    END IF;

    -- Clock-drift ceiling (issue #102): refuse a verified event whose asserted HLC wall is
    -- implausibly far in OUR future, so a broken/hostile peer cannot ratchet this node's
    -- clock — nor trust_peer's `ORDER BY hlc_wall DESC` — forward without bound. See
    -- cairn_max_hlc_drift_ms() (db/001) for the bound and its rationale. This is a REJECTION
    -- (not the clinical door's clamp): the node plane STORES hlc_wall in node_event and
    -- trust_peer orders on it, so a poison value that got admitted would win "latest peer
    -- state" forever — it must never enter the table. A bare RAISE (P0001) lands in the pull
    -- loop's self-healing deny-all class (cairn-node sync.rs): the event is skip-and-advanced
    -- and re-offered on the next full sweep, so a modestly-future event (transient skew) is
    -- admitted once local time catches up, while an absurd one is skipped forever, poisoning
    -- nothing. Measured against clock_timestamp() (our own wall clock), never the
    -- possibly-already-advanced hlc_state, so the bound cannot itself be ratcheted.
    IF (b -> 'hlc' ->> 'wall')::bigint
           > (extract(epoch FROM clock_timestamp()) * 1000)::bigint + cairn_max_hlc_drift_ms() THEN
        RAISE EXCEPTION 'apply_remote_node_event: HLC wall % ms is more than % ms ahead of local time — clock-drift ceiling (issue #102)',
            (b -> 'hlc' ->> 'wall')::bigint, cairn_max_hlc_drift_ms();
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
    ELSE
        -- peer/revoke/supersede: the author must be a currently-trusted peer (resolved by key).
        SELECT node_id INTO v_author_node FROM node_current WHERE signer_key_id = v_signer;
        IF v_author_node IS NULL THEN
            RAISE EXCEPTION 'apply_remote_node_event: author key % maps to no known node', v_signer;
        END IF;
        IF NOT EXISTS (SELECT 1 FROM trust_peer WHERE peer_node_id = v_author_node AND status = 'active') THEN
            RAISE EXCEPTION 'apply_remote_node_event: author % is not an active peer (deny-all)', encode(v_author_node,'hex');
        END IF;

        -- supersede (issue #201, ADR-0026 slice C): a restored node's lineage claim
        -- REPLICATES like every other node event — the submit door emits it and the
        -- restore door applies it, so an apply door without this arm left a peer pulling
        -- a restored node's history refusing the event on every full sweep FOREVER (a
        -- permanent set-union exclusion on the node plane). Admitting it is trust-bounded
        -- exactly like peer/revoke: the author must be an active peer (checked above),
        -- and the claim feeds ONLY the advisory node_lineage view — node_current resolves
        -- keys from `enroll` rows alone and trust_peer reads only `peer`/`revoke`, so a
        -- false supersede from a hostile-but-trusted peer can hijack neither key
        -- resolution nor peer trust; it is an attributable, signed claim (principle 2).
        IF v_op = 'supersede' THEN
            -- Mirror the local door's legible guard: name the missing field, never store
            -- a NULL/garbage subject.
            IF v_payload ->> 'superseded_node_id_hex' IS NULL THEN
                RAISE EXCEPTION 'apply_remote_node_event: node.superseded from % missing superseded_node_id_hex in payload', encode(v_author_node,'hex');
            END IF;
            INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
                signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
            VALUES (v_eid, 'supersede', v_author_node,
                cairn_decode_hex_or_raise('superseded_node_id_hex',
                    v_payload ->> 'superseded_node_id_hex', 'apply_remote_node_event'),
                v_signer, (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
                b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
            ON CONFLICT (node_event_id) DO NOTHING;
        ELSE
            -- Mirror the local door's legible guard: a trusted-but-malformed peer event
            -- (missing peer_node_id_hex) is rejected, not stored with a \x00 subject. Present but
            -- MALFORMED is caught by cairn_decode_hex_or_raise below (issue #228). Both guards
            -- stay here rather than moving into the helper: they can name the AUTHOR, which is
            -- what tells the operator which peer to go and fix, and the helper cannot see it.
            IF v_payload ->> 'peer_node_id_hex' IS NULL THEN
                RAISE EXCEPTION 'apply_remote_node_event: % from % missing peer_node_id_hex in payload', v_type, encode(v_author_node,'hex');
            END IF;
            INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
                signer_key_id, peer_pubkey, fingerprint, role, scope_hint, target_event_id,
                hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
            VALUES (v_eid, v_op, v_author_node,
                cairn_decode_hex_or_raise('peer_node_id_hex',
                    v_payload ->> 'peer_node_id_hex', 'apply_remote_node_event'),
                v_signer, v_payload ->> 'peer_pubkey', v_payload ->> 'fingerprint',
                cairn_node_role_or_raise(v_payload ->> 'role', 'apply_remote_node_event'),
                v_payload ->> 'scope_hint',
                CASE WHEN NULLIF(v_payload ->> 'target_event_id','') IS NULL THEN NULL
                     ELSE cairn_uuid_or_raise('target_event_id',
                            v_payload ->> 'target_event_id', 'apply_remote_node_event') END,
                (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
                b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
            ON CONFLICT (node_event_id) DO NOTHING;
        END IF;
    END IF;

    -- SUBSTITUTION REFUSAL (#619, ADR-0073) — the federation admission gate's copy of the
    -- submit_node_event tail. Every arm above inserts ON CONFLICT DO NOTHING; without this, a
    -- trusted peer's SECOND, different event under an id already held vanished, the function
    -- returned normally, the puller counted it admitted and advanced past it: every later full
    -- sweep re-offered it only to drop it again in silence — two nodes holding different bytes
    -- under one id, forever. A dropped rival GENESIS is the sharpest case — that peer's key
    -- would never resolve here.
    -- The refusal is P0001 like every other (db/001's contract); the node puller tells it from a
    -- routine deny-all by STATE, not by this text (crates/cairn-node/src/sync/substitution.rs).
    -- Same two placement rules as submit_node_event: AFTER the IF/ELSE, and an UNCONDITIONAL read.
    SELECT content_address INTO v_found FROM node_event WHERE node_event_id = v_eid;
    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'apply_remote_node_event');

    -- Clock never falls behind an event we accepted (HLC invariant A3, mirrors cairn-sync).
    -- The clock-drift REJECTION near the top of this function is this door's ceiling; the helper
    -- (db/001) is the pure merge. ONE merge for all three arms since #619 folded them into this
    -- tail (it used to be three copies), and AFTER the guard, so a refused rival never advances
    -- this node's clock (the RAISE would roll it back regardless; the order says what is meant).
    PERFORM cairn_node_hlc_merge((b -> 'hlc' ->> 'wall')::bigint,
                                 (b -> 'hlc' ->> 'counter')::int);
    RETURN v_eid;
END;
$$;

REVOKE EXECUTE ON FUNCTION apply_remote_node_event(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION apply_remote_node_event(bytea) TO cairn_node;

COMMIT;
