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
    present      BOOLEAN NOT NULL DEFAULT FALSE,
    first_seen   TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    fetched_at   TIMESTAMPTZ,

    -- The blob is self-verifying: when bytes are present, their BLAKE3 must equal
    -- the address. cairn-sync checks this before flipping present := TRUE; the DB
    -- restates the invariant so a wrong-hash blob can never masquerade as present.
    CONSTRAINT blob_self_verifying
        CHECK (NOT present OR (content IS NOT NULL AND byte_len = octet_length(content)))
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
