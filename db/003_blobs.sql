-- Cairn walking skeleton — the content-addressed blob tier (Spike 0001 §4.4, Bet A4).
--
-- Attachments are referenced eagerly (the reference rides in event_log.attachments
-- on the clinical plane) but their BYTES are lazy: they arrive on a separate,
-- preemptible, separately-budgeted tier that must NEVER starve clinical sync
-- (ADR-0013, the availability floor). Blobs are content-addressed by BLAKE3 whose
-- internal tree lets chunks be verified independently — the property that makes
-- chunked, resumable, multi-source swarm fetch self-verifying (§4.4).

BEGIN;

-- Local content-addressed store. A node may hold a blob's BYTES or only know it
-- exists (present = FALSE) and fetch on legitimate need. The skeleton keeps bytes
-- inline in BYTEA for simplicity; the real tier is a chunked object store.
CREATE TABLE IF NOT EXISTS blob_store (
    -- Self-describing BLAKE3 multihash (prefix 0x1e 0x20 = blake3, 32 bytes).
    blob_address BYTEA   PRIMARY KEY,
    media_type   TEXT    NOT NULL,
    byte_len     BIGINT,                 -- known from the reference before bytes arrive
    content      BYTEA,                  -- NULL until fetched; verified before present := TRUE
    outboard     BYTEA,                  -- bao verified-streaming tree; set with content, serves slices
    present      BOOLEAN NOT NULL DEFAULT FALSE,
    first_seen   TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    fetched_at   TIMESTAMPTZ,

    -- The blob is self-verifying: when bytes are present, their BLAKE3 must equal the
    -- content address. That BLAKE3-vs-address check is performed by cairn-sync (L2) before
    -- it flips present := TRUE — pgcrypto has no BLAKE3, so the DB CANNOT restate it here.
    -- This CHECK is only the length-consistency floor it CAN enforce; it does NOT prove
    -- the bytes hash to the address (a right-length wrong-bytes blob could sit present=TRUE
    -- if L2 were bypassed). See issue: BLAKE3 verification belongs in cairn_pgx to make the
    -- self-verifying property a true in-DB floor rather than an L2 promise.
    CONSTRAINT blob_length_consistent
        CHECK (NOT present OR (content IS NOT NULL AND byte_len = octet_length(content)))
);

-- Persistent partial-fetch state (§8.2 resumability). Each VERIFIED slice lands
-- here as it arrives — out of order, from any swarm source — keyed by its index
-- (offset / SLICE_BYTES). ON CONFLICT DO NOTHING makes chunk apply idempotent
-- set-union, exactly like the event plane. When every index for a blob is present
-- the byte tier assembles, whole-blob-verifies, fills blob_store, and deletes
-- these rows. A restart therefore resumes by fetching only the missing indexes.
CREATE TABLE IF NOT EXISTS blob_chunk (
    blob_address BYTEA       NOT NULL,
    chunk_index  INT         NOT NULL,
    content      BYTEA       NOT NULL,   -- verified bytes for this slice
    received_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (blob_address, chunk_index)
);

-- A node learns blobs exist from event references it applies. This helper upserts
-- a *reference-only* row (present = FALSE) — the eager half of reference-eager /
-- byte-lazy. Bytes are filled later by the lazy tier in cairn-sync.
CREATE OR REPLACE FUNCTION blob_note_reference(addr BYTEA, mt TEXT, len BIGINT)
RETURNS void LANGUAGE sql AS $$
    INSERT INTO blob_store (blob_address, media_type, byte_len)
    VALUES (addr, mt, len)
    ON CONFLICT (blob_address) DO NOTHING;
$$;

COMMIT;
