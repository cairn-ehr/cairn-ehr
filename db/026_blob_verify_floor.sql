-- Cairn — the blob self-verification floor (ADR-0013 point 11; closes the honest
-- gap recorded in db/003).
--
-- Until this file, the blob tier's self-verifying property — bytes marked
-- present = TRUE actually BLAKE3-hash to the blob_address that names them — was an
-- L2 promise (cairn-sync verifies before flipping present) that the database could
-- not restate: pgcrypto has no BLAKE3. A client with raw SQL access could store
-- arbitrary bytes as any named blob, and every honest consumer downstream (viewers,
-- renditions, exports, the swarm-serving path) would serve the wrong bytes under a
-- signed-event-referenced content address — exactly the failure ADR-0013 point 11
-- names as this tier's one safety-critical seam ("a wrong-hash blob must never be
-- served as the named one"). cairn_pgx >= 0.3.0 provides cairn_blob_verify — a
-- thin wrapper over the SAME cairn-event blob_address implementation L2 uses (one
-- implementation, never two) — so the floor now holds in-DB, unbypassably
-- (principle 12: "even a client talking raw SQL cannot break it").
--
-- Mechanism: a TRIGGER, not a SECURITY DEFINER door + REVOKE. blob_store
-- legitimately receives raw DML from the byte tier (the do_blobd assembly flip,
-- put-blob) and from blob_note_reference; a trigger binds every writer — including
-- a bypassing one — with zero call-site churn.
--
-- Honest limits (deliberate; see docs/superpowers/specs/2026-07-05-blob-verify-floor-design.md):
--   * blob_chunk rows are NOT in-DB verified: bao slice verification needs the
--     wire encoding's interleaved tree nodes, which are not stored. Wrong chunks
--     can only ever assemble into a whole-blob flip that FAILS here — they waste
--     space, never serve wrong bytes under a named address.
--   * outboard is NOT verified: a wrong outboard yields slices the FETCHING peer's
--     bao decode rejects against the signed address root (self-verifying fetch,
--     zero trust in the source — ADR-0013 point 4). Availability degradation,
--     never an integrity hole; in-DB verification would cost a full bao re-encode
--     per write for no integrity gain.
--   * reference-only rows (present = FALSE) stay untouched and cheap: the floor
--     prices only the present-flip (one BLAKE3 pass over the bytes — multi-GB/s
--     even on Cortex-A76, Bet B4).
--   * a superuser can drop the trigger — the same standing as every other floor
--     piece (grants, CHECKs): the floor binds applications and agents, not the DBA.

BEGIN;

-- Load-time dependency gate. The guard below is PL/pgSQL, so its call to
-- cairn_blob_verify is LATE-BOUND: against a stale (pre-0.3.0) cairn_pgx this
-- file would otherwise load CLEANLY and the failure would surface only at the
-- first present-flip write, as an illegible `undefined function` from whatever
-- writer happened to trip the trigger. Refuse the load itself, legibly, for
-- EVERY loader (cairn-node, cairn-sync, raw psql) — the migration declares its
-- own dependency rather than trusting each daemon's connect-time version gate.
DO $$
BEGIN
    IF to_regprocedure('cairn_blob_verify(bytea, bytea)') IS NULL THEN
        RAISE EXCEPTION 'db/026 requires cairn_pgx >= 0.3.0: cairn_blob_verify(bytea, bytea) is not installed'
            USING HINT = 'The installed extension library is stale. Rebuild + reinstall it '
                         || '(`cargo pgrx install` against this cluster''s PostgreSQL), then re-run init.';
    END IF;
END;
$$;

-- The guard: a row may only ever sit present = TRUE if its bytes hash to the
-- address that names them. Raises with the legible cairn_blob_verify_error
-- diagnostic as DETAIL so an operator sees WHY (wrong bytes vs. malformed
-- address), mirroring the cairn_verify / cairn_verify_error door pattern.
CREATE OR REPLACE FUNCTION cairn_blob_present_guard()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.content IS NULL THEN
        RAISE EXCEPTION 'blob_store: present = TRUE requires content bytes (blob %)',
            encode(NEW.blob_address, 'hex');
    END IF;
    -- IS DISTINCT FROM TRUE, not NOT(...): the pgrx function is STRICT, so a NULL
    -- argument yields SQL NULL, and `IF NOT NULL-boolean` silently passes. Both
    -- arguments are non-NULL here (content pre-checked, blob_address is the PK),
    -- but the floor stays fail-closed even if that ever changes.
    IF cairn_blob_verify(NEW.blob_address, NEW.content) IS DISTINCT FROM TRUE THEN
        RAISE EXCEPTION 'blob_store: content does not hash to blob_address (blob %)',
            encode(NEW.blob_address, 'hex')
            USING DETAIL = cairn_blob_verify_error(NEW.blob_address, NEW.content);
    END IF;
    RETURN NEW;
END;
$$;

-- Any INSERT arriving present must carry verified bytes.
-- CREATE OR REPLACE (PG14+; well under this project's PostgreSQL floor) rather
-- than DROP IF EXISTS + CREATE: replacement takes only SHARE ROW EXCLUSIVE and
-- never opens a trigger-less window, where the DROP would take ACCESS EXCLUSIVE
-- on a write-hot byte-tier table every time init replays this file.
CREATE OR REPLACE TRIGGER blob_present_verify_ins
    BEFORE INSERT ON blob_store
    FOR EACH ROW WHEN (NEW.present)
    EXECUTE FUNCTION cairn_blob_present_guard();

-- Any UPDATE that could change what sits present under an address must
-- re-verify: (a) a flip into present, (b) a content swap under a present row,
-- (c) a re-keying to a different address. Column-level (UPDATE OF): an UPDATE
-- whose SET list touches none of these columns cannot change them, so a
-- metadata-only update (media_type, fetched_at, outboard) neither re-pays the
-- hash NOR evaluates the WHEN clause — that second part matters, because
-- `NEW.content IS DISTINCT FROM OLD.content` on an untouched multi-GB TOASTed
-- column would detoast and memcmp the full bytes on every metadata touch.
-- Accepted caveat: a FUTURE alphabetically-earlier BEFORE trigger mutating
-- NEW.content could slip past a column-level trigger; blob_store has no other
-- triggers, and creating one takes the same table-owner standing as dropping
-- this floor outright (the recorded superuser limit above).
CREATE OR REPLACE TRIGGER blob_present_verify_upd
    BEFORE UPDATE OF content, blob_address, present ON blob_store
    FOR EACH ROW WHEN (NEW.present AND (NOT OLD.present
                       OR NEW.content IS DISTINCT FROM OLD.content
                       OR NEW.blob_address IS DISTINCT FROM OLD.blob_address))
    EXECUTE FUNCTION cairn_blob_present_guard();

COMMIT;
