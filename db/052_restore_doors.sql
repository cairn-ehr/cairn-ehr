-- Cairn — the two doors a disaster recovery needs (#554 slice 2d).
--
-- WHY THIS FILE EXISTS. Slice 2c made the backup medium carry the clinical record; nothing
-- gave it back to a node. Reading it back needs two things the floor did not have:
--
--   1. `restore_actor_registry` — a way to reinstate `actor_event`, because EVERY clinical
--      apply door gates on `actor_current`. Without it a restored node refuses its own
--      history, and "restored" means a node with no patients.
--   2. `cairn_quarantine_event` — ONE quarantine pen, callable from both `cairn-sync` (which
--      has always had one, in Rust) and `cairn-node` (which cannot call it: `cairn-sync` is a
--      binary-only crate with no `lib.rs`). The alternative was to copy the INSERT into a
--      second crate, forking the quota and dedupe logic across two implementations of a
--      safety floor. ADR-0001 says where it goes instead: the database.
--
-- Plus one additive column on the pen, so a restore-penned sealed event keeps its key.
--
-- ⚠️ PRIVILEGE. `restore_actor_registry` writes the trust anchor every clinical apply door
-- gates on, and it deliberately bypasses `enroll_actor`'s collision guards (see its own doc
-- below). It is the highest-value new privilege in this slice, so it is granted to
-- `cairn_node` and explicitly NOT to `cairn_agent` — an advisory actor must never be able to
-- re-authorise itself by "restoring" a registry. That grant is a TESTED property
-- (`crates/cairn-node/tests/…`), not a comment: the #430/#431 shape is a decoy path around a
-- floor that looks correct at its own site.

BEGIN;

-- ---------------------------------------------------------------------------
-- 1. The actor-registry restore door
-- ---------------------------------------------------------------------------

-- Reinstate a dead node's append-only actor registry from its sealed local-state export.
--
-- IT TAKES THE WHOLE SET IN ONE CALL, not one row per call. That is what makes it a
-- transaction boundary, and it is what makes the RESUME path expressible at all: a per-row
-- door cannot tell a partial prior attempt from a foreign registry, because a single row
-- carries no evidence about the set it belongs to. It returns the number of rows it actually
-- inserted, so a resumed restore reports honestly rather than re-claiming the whole set.
--
-- FENCED TWICE, and the two fences fail for different reasons — an operator must be told
-- which:
--
--   1. `local_node` must be EMPTY. A live node is never a restore target; this is
--      `restore_node_event`'s fence and it fails closed on any enrolled node, forever.
--   2. `actor_event` must hold no row whose `actor_event_id` is absent from `p_rows`.
--
-- Fence 2 is the set-shaped form of "an empty table", and it is strictly better than it. It
-- is still structurally unable to inject into a populated registry — one foreign actor row
-- and the whole call is refused — but a registry that is a SUBSET of what is being restored
-- is the signature of an interrupted restore, and this door completes it instead of refusing
-- it. So: a fresh database gets everything; an interrupted restore re-run gets only the
-- missing rows and is told how many; a half-provisioned node that enrolled its own actor is
-- refused, because that row is not in `p_rows`.
--
-- That resumability is not a convenience. Design §3 moves `finalize_identity` to LAST so the
-- whole restore runs inside the un-enrolled fence, which means a clinical apply that fails
-- catastrophically leaves a database with no genesis — still legitimately restorable, from
-- the same medium, into the same database. But the registry is installed BEFORE that apply,
-- so the failed attempt leaves `local_node` empty and `actor_event` POPULATED. A door that
-- refused on "any row present" would turn away the very re-run the ordering exists to
-- enable, and the only recovery would be a fresh database — the outcome it was avoiding.
--
-- WHY NOT A BLANKET `TRUNCATE actor_event` ON THE UN-ENROLLED FENCE. Because `enroll_actor`
-- contains ZERO `local_node` references: a populated `actor_event` on an un-enrolled database
-- is not provably the residue of a failed restore. It could be a half-provisioned node that
-- enrolled an actor before minting identity, and truncating would destroy its registry.
--
-- ORDERING. Rows are inserted in ascending source-`seq` order and `seq BIGINT GENERATED
-- ALWAYS AS IDENTITY` assigns FRESH values. The restored relative order is therefore exact,
-- without `OVERRIDING SYSTEM VALUE` (which db/004's own comment asks to keep loud in review)
-- and without leaving the identity counter behind the restored maximum — the bug an
-- explicit-`seq` restore would plant for the NEXT `enroll_actor`, which would then collide.
-- A resumed call inserts its remainder after the rows already present, so relative order
-- survives the interruption too.
--
-- `recorded_at` IS CARRIED VERBATIM AND IS REQUIRED. It is not an audit detail: `actor_current`
-- resolves the trust anchor with `ORDER BY ae.actor_id, ae.recorded_at DESC, ae.seq DESC` and
-- compares revocations with `(r.recorded_at, r.seq) >= (ae.recorded_at, ae.seq)`, so
-- `recorded_at` is the PRIMARY ordering key deciding who may author. A row whose timestamp
-- defaulted to `clock_timestamp()` would outrank a genuine older `revoke` and silently
-- re-authorise a recalled actor — arriving through the door built to restore the registry.
--
-- IT DELIBERATELY BYPASSES `enroll_actor`'s COLLISION GUARDS (#152 / #166), and this is the
-- one thing about it that needs saying out loud. Those guards refuse a FRESH enroll that
-- would silently merge two actors (`cairn_actor_id_key_conflict`) or resurrect a retired one
-- (its NULL-key case). A restore is replaying a history that already passed them on the dead
-- node; re-running them would refuse this node's own legitimate `revoke` and `supersede`
-- rows — every one of which trips the actor-id/key conflict guard by construction, because
-- prior registration history is exactly what they are. So this door validates SHAPE and
-- replays; it does not re-adjudicate.
--
-- WHAT AUTHENTICATES THESE ROWS: nothing per-row. They arrive inside the `CAIRNL1` export,
-- authenticated by that container's AEAD and nothing else — THE ONE PART OF A RESTORE THAT IS
-- NOT VERIFY-ON-APPLY. The clinical events around them are each individually
-- signature-verified by `apply_remote_event`; the registry is not. Accepted deliberately:
-- whoever holds the export AND its passphrase or recovery code already controls the restored
-- node completely, so refusing here costs the record and buys nothing. It is recorded in
-- ADR-0067 and PRINTED TO THE OPERATOR at restore time, because a limitation that lives only
-- in a design doc is a limitation nobody will find.
CREATE OR REPLACE FUNCTION restore_actor_registry(p_rows JSONB)
RETURNS INTEGER
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_row       JSONB;
    v_inserted  INTEGER := 0;
    v_hit       INTEGER;
    v_foreign   TEXT;
    v_op        TEXT;
    v_recorded  TEXT;
BEGIN
    -- Fence 1.
    IF EXISTS (SELECT 1 FROM local_node) THEN
        RAISE EXCEPTION 'restore_actor_registry: this database already has an enrolled node — '
            'a live node is never a restore target. Restore into a fresh, un-enrolled database.';
    END IF;

    IF p_rows IS NULL OR jsonb_typeof(p_rows) <> 'array' THEN
        RAISE EXCEPTION 'restore_actor_registry: p_rows must be a JSON array of registry rows, got %',
            COALESCE(jsonb_typeof(p_rows), 'NULL');
    END IF;

    -- Fence 2. Named, never counted: an operator seeing "a foreign row is present" cannot
    -- act on it, and the id is what tells a half-provisioned node from a partial restore.
    SELECT ae.actor_event_id::text INTO v_foreign
      FROM actor_event ae
     WHERE NOT EXISTS (
             SELECT 1 FROM jsonb_array_elements(p_rows) r
              WHERE r ->> 'actor_event_id' = ae.actor_event_id::text)
     LIMIT 1;
    IF v_foreign IS NOT NULL THEN
        RAISE EXCEPTION 'restore_actor_registry: actor_event already holds row % which is not in '
            'the registry being restored. That is a half-provisioned node (one that enrolled its '
            'own actor), not the residue of an interrupted restore — completing it would merge '
            'two nodes'' trust anchors. Restore into a fresh database.', v_foreign;
    END IF;

    FOR v_row IN
        SELECT r FROM jsonb_array_elements(p_rows) r ORDER BY (r ->> 'seq')::BIGINT
    LOOP
        -- SHAPE validation only (see the header: this door replays, it does not
        -- re-adjudicate). Each check names its field, because a restore mid-disaster is the
        -- worst possible moment for an illegible refusal.
        v_op := v_row ->> 'op';
        IF v_op IS NULL OR v_op NOT IN ('enroll', 'supersede', 'revoke') THEN
            RAISE EXCEPTION 'restore_actor_registry: row % carries op %, not one of '
                'enroll/supersede/revoke (the closed actor-event algebra, ADR-0011)',
                COALESCE(v_row ->> 'actor_event_id', '(no actor_event_id)'),
                COALESCE(v_op, 'NULL');
        END IF;

        -- REQUIRED, never defaulted — see the header for the resurrection hazard.
        v_recorded := v_row ->> 'recorded_at';
        IF v_recorded IS NULL OR v_recorded = '' THEN
            RAISE EXCEPTION 'restore_actor_registry: row % carries no recorded_at. It is '
                'actor_current''s PRIMARY ordering key, so defaulting it here could outrank a '
                'genuine older revoke and silently re-authorise a recalled actor.',
                COALESCE(v_row ->> 'actor_event_id', '(no actor_event_id)');
        END IF;

        INSERT INTO actor_event
            (actor_event_id, actor_id, op, kind, pinned, signing_key_id, superseded_by, recorded_at)
        VALUES (
            (v_row ->> 'actor_event_id')::UUID,
            cairn_decode_hex_or_raise('actor_id', v_row ->> 'actor_id', 'restore_actor_registry'),
            v_op,
            v_row ->> 'kind',
            CASE WHEN v_row ->> 'pinned' IS NULL THEN NULL ELSE (v_row ->> 'pinned')::JSONB END,
            v_row ->> 'signing_key_id',
            CASE WHEN v_row ->> 'superseded_by' IS NULL THEN NULL
                 ELSE cairn_decode_hex_or_raise('superseded_by', v_row ->> 'superseded_by',
                                                'restore_actor_registry') END,
            v_recorded::TIMESTAMPTZ
        )
        -- Idempotent by row identity, which is what makes a resume safe to re-run any number
        -- of times. Fence 2 has already established that every row present belongs to this
        -- set, so a conflict here is a row this same restore already placed.
        ON CONFLICT (actor_event_id) DO NOTHING;

        GET DIAGNOSTICS v_hit = ROW_COUNT;
        v_inserted := v_inserted + v_hit;
    END LOOP;

    RETURN v_inserted;
END;
$$;

REVOKE EXECUTE ON FUNCTION restore_actor_registry(JSONB) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION restore_actor_registry(JSONB) TO cairn_node;

-- ---------------------------------------------------------------------------
-- 2. The shared quarantine pen
-- ---------------------------------------------------------------------------

-- The pen's additive custody column (#554 design §2.2).
--
-- `sync_quarantine` stored `signed_bytes`, `attestation`, `attester_key` — never a DEK — and
-- the requeue path passed `None` on the stated reasoning that "a re-queued sealed event is
-- admitted structurally without custody; its DEK rides a later normal pull once the peer
-- serves it."
--
-- THAT REASONING IS SOUND FOR SYNC AND FALSE FOR RESTORE. A restored solo node has no peer.
-- The medium is the only carrier of that key, and `finalize_identity` fences the restore door
-- behind the operator. Penning a sealed clinical event as-is would preserve the bytes,
-- silently drop the key, and a later requeue would admit permanently-unopenable ciphertext at
-- exit 0 — #500's own shape one layer down, inside the slice built to end it.
--
-- This strictly improves the SYNC path too: a penned sealed event whose peer is later
-- decommissioned becomes recoverable instead of lost. Nullable, because an unsealed event —
-- and every row penned before this migration — legitimately has none.
--
-- ADDITIVE HERE, NOT WIDENED IN db/021 (issue #207): `connect_and_load_schema` re-runs every
-- db/*.sql on every connect, so a widened `CREATE TABLE` in db/021 plus this ALTER would be
-- two spellings of one column. If a later tidy-up widens it, that pair owes an entry in
-- `crates/cairn-node/tests/migration_replay_widening.rs`.
ALTER TABLE sync_quarantine ADD COLUMN IF NOT EXISTS dek_wrapped BYTEA;

COMMENT ON COLUMN sync_quarantine.dek_wrapped IS
    'The refused event''s wrapped DEK, preserved so a requeue can restore custody. NULL for '
    'an unsealed event and for every row penned before db/052. Load-bearing on the RESTORE '
    'path, which has no peer to re-serve the key (#554).';

-- Pen one refused event. THE one implementation, called by `cairn-sync` (the pull and requeue
-- loops) and by `cairn-node` (a restore).
--
-- Lifted out of `cairn-sync`'s Rust in #554 slice 2d, verbatim in behaviour, for a reason that
-- is not tidiness: `cairn-sync` is a BINARY-ONLY crate (no `lib.rs`, no `[lib]` target) and
-- `cairn-node` does not depend on it, so a restore had no implementation available to it and
-- the obvious repair — copy the INSERT — forks the quota and dedupe logic across two crates.
-- ADR-0001's direction (fat Postgres, thin daemons) applied to a floor that is already about
-- refusing things safely.
--
-- Returns TRUE if the bytes are already ACKED — an operator's recorded decision that they
-- will never enter the record — so the caller can distinguish "penned" from "penned and
-- already excluded".
--
-- THE QUOTA IS CALLER-SUPPLIED POLICY, and NULL on either parameter means UNBOUNDED. That is
-- a deliberate carve-out, not a loosening, and it is what the restore path passes:
--
--   * The quota exists to stop a HOSTILE OR BROKEN PEER from filling local disk with refused
--     bytes. A restore's input is the operator's own medium, already on local disk, already
--     `verify-backup`-checked. There is no adversary to bound and no unbounded stream — the
--     medium is finite and known.
--   * The bytes it would refuse are bytes the node is ABOUT TO LOSE PERMANENTLY. A resource
--     budget that trades a clinic's record for disk it has already spent is the wrong trade,
--     and it is the exact trade this project rejects for torn media.
--   * Its own refusal text promises "the watermark freezes instead (delayed, never lost)".
--     That is a SYNC-PATH guarantee: it needs a cursor to freeze and a peer that will
--     re-serve. A restore has neither — it pins no floor by design, there is no peer, and
--     `finalize_identity` fences the node immediately afterwards. Inheriting it unexamined
--     would LOSE those events and their custody at exit.
--
-- "Unbounded" must not mean "unreported": the restore caller reports the pen's row count and
-- byte total in its operator summary and says so explicitly when it exceeds the ordinary
-- per-peer quota. A bound the operator can see beats a bound that silently drops the record.
--
-- `p_peer` is NOT NULL upstream and the per-peer quota probes filter on it, so a restore
-- passes the explicit sentinel `(restore)` rather than an empty string: a restore-penned row
-- must be identifiable as one rather than blend into an unnamed link.
CREATE OR REPLACE FUNCTION cairn_quarantine_event(
    p_digest       BYTEA,
    p_signed       BYTEA,
    p_attestation  BYTEA,
    p_attester_key BYTEA,
    p_peer         TEXT,
    p_refused_seq  BIGINT,
    p_reason       TEXT,
    p_dek_wrapped  BYTEA    DEFAULT NULL,
    p_max_rows     INTEGER  DEFAULT NULL,
    p_max_bytes    BIGINT   DEFAULT NULL
) RETURNS BOOLEAN
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_acked    BOOLEAN;
    v_inserted INTEGER;
BEGIN
    -- Dedupe FIRST: a re-offer of known bytes always succeeds (it does not grow the pen),
    -- even when the peer is over quota. `reason` is never overwritten — it is the
    -- verify-time forensics, and a later refusal goes to `last_requeue_error` so a transient
    -- fault during requeue can never destroy the original diagnosis. A token once seen is
    -- never dropped (COALESCE), and neither is custody: a re-offer that carries a DEK a
    -- previous offer lacked enriches the row.
    UPDATE sync_quarantine
       SET last_seen    = clock_timestamp(),
           seen_count   = seen_count + 1,
           attestation  = COALESCE(attestation, p_attestation),
           attester_key = COALESCE(attester_key, p_attester_key),
           dek_wrapped  = COALESCE(dek_wrapped, p_dek_wrapped)
     WHERE content_digest = p_digest
    RETURNING acked INTO v_acked;
    IF FOUND THEN
        RETURN v_acked;
    END IF;

    -- New bytes: admit within the caller's quota, INCLUDING this event's own size so one
    -- huge frame cannot overshoot the byte budget. The aggregate probes ride the same
    -- statement so the check and the insert cannot disagree; a concurrent writer can still
    -- overshoot by one row — a resource budget, not an exact invariant. Only UNACKED rows
    -- count (#197): an acked row is a resolved human decision, retained as the record of it
    -- and never auto-deleted — if it still consumed quota, "ack the held rows" (this
    -- function's own documented remedy) could never free the pen.
    --
    -- A NULL cap is UNBOUNDED, expressed as `p_max_rows IS NULL OR …` rather than a
    -- sentinel value: a very large number would still be a bound, and a bound nobody chose
    -- is how the sync quota came to apply to a restore in the first place.
    INSERT INTO sync_quarantine
        (content_digest, signed_bytes, attestation, attester_key, peer, refused_seq, reason,
         dek_wrapped)
    SELECT p_digest, p_signed, p_attestation, p_attester_key, p_peer, p_refused_seq, p_reason,
           p_dek_wrapped
     WHERE (p_max_rows IS NULL
            OR (SELECT count(*) FROM sync_quarantine WHERE peer = p_peer AND NOT acked) < p_max_rows)
       AND (p_max_bytes IS NULL
            OR (SELECT COALESCE(sum(octet_length(signed_bytes)), 0) FROM sync_quarantine
                 WHERE peer = p_peer AND NOT acked) + octet_length(p_signed) <= p_max_bytes)
    ON CONFLICT (content_digest) DO NOTHING;

    GET DIAGNOSTICS v_inserted = ROW_COUNT;
    IF v_inserted > 0 THEN
        RETURN FALSE;
    END IF;

    -- Zero rows means EITHER over-quota OR a concurrent writer penned the same bytes first
    -- (ON CONFLICT DO NOTHING). Distinguish them — a false "quota" diagnosis on the safety
    -- path would send an operator chasing a condition that does not exist.
    SELECT acked INTO v_acked FROM sync_quarantine WHERE content_digest = p_digest;
    IF FOUND THEN
        RETURN v_acked;  -- lost a benign race: the trace exists
    END IF;

    RAISE EXCEPTION 'quarantine pen for peer ''%'' is at its quota of unacked rows '
        '(% rows / % bytes) — refusing to grow it; the watermark freezes instead (delayed, '
        'never lost). Inspect with `cairn-sync quarantine` and fix or ack the held rows.',
        p_peer, COALESCE(p_max_rows::TEXT, 'unbounded'), COALESCE(p_max_bytes::TEXT, 'unbounded');
END;
$$;

REVOKE EXECUTE ON FUNCTION cairn_quarantine_event(BYTEA, BYTEA, BYTEA, BYTEA, TEXT, BIGINT,
                                                  TEXT, BYTEA, INTEGER, BIGINT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_quarantine_event(BYTEA, BYTEA, BYTEA, BYTEA, TEXT, BIGINT,
                                                 TEXT, BYTEA, INTEGER, BIGINT) TO cairn_node;

COMMIT;
