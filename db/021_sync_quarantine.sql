-- db/021_sync_quarantine.sql
-- Cairn — durable quarantine + re-offer floor for unverifiable replicated
-- events (issue #108, revised by the PR #110 review).
--
-- WHY: the clinical-plane pull loop (cairn-sync `do_pull`) cannot apply an
-- event whose signature does not verify — a corrupt frame, or a peer still
-- serving pre-ADR-0040 uncontextualized signatures (ADR-0040 made that trigger
-- universal: a mixed-version peer fails verification for its entire pre-upgrade
-- history, not the odd corrupt frame). Before this table the only evidence of a
-- skip was a transient stderr line and an in-memory counter, while later
-- verifiable events advanced the watermark past the skipped ones: a *silent*
-- permanent set-union violation (the exact class the A1 watermark fix exists to
-- prevent).
--
-- WHAT: a node-local (never synced, like `sync_state`) holding pen. Every event
-- the pull loop refuses as unverifiable is persisted here VERBATIM — signed
-- bytes plus any attestation that travelled with it — together with the legible
-- verify-failure reason (ContextMismatch vs BadSignature etc., ADR-0040) so an
-- operator can (a) see exactly what was excluded and why, and (b) re-process it
-- through the real apply door (`cairn-sync requeue`) after fixing the cause
-- (e.g. a version-skewed daemon binary).
--
-- THE RE-OFFER FLOOR (the load-bearing rule, revised): a durable trace alone is
-- NOT a license to advance past an event — a peer that later re-signs its
-- history would re-serve the fixed bytes at the same HLCs, and a watermark that
-- had moved past them would never fetch them again (permanent exclusion; the
-- PR #110 review confirmed this on the mixed legacy+new batch). So quarantining
-- an event also pins `sync_state.quarantine_floor_*` (added below) at the
-- contiguous-applied position BELOW the refused slot; every later pull fetches
-- from min(watermark, floor), so the slot keeps being re-offered (deduping onto
-- its row here) while the watermark still advances for valid events — progress
-- without loss, "delayed, never lost" made true. The floor clears itself when a
-- full cycle from the floor applies with zero refusals (the peer was fixed), or
-- when a human explicitly licenses the exclusion by setting `acked = TRUE` on
-- the row (policy-neutral mechanism: the skip is a recorded human decision,
-- never an automatic one). While any unacked refusal exists, the pull FAILS
-- LOUDLY every cycle.
--
-- This is operational daemon state, not clinical content: rows here are
-- *candidate* events that never passed the floor, so the table sits beside
-- `sync_state`, outside the append-only event plane, and DELETE (on successful
-- requeue) is legitimate — removing a quarantine row never removes anything
-- from the record, because admission happens only through `apply_remote_event`.
-- Note: deleting a row does NOT stop a peer that still serves those bytes from
-- re-offering them (the row simply reappears); `acked` is the honest way to
-- accept a permanent exclusion.

BEGIN;

CREATE TABLE IF NOT EXISTS sync_quarantine (
    -- Content address (sha2-256 multihash) of the signed bytes — the same
    -- addressing the event plane uses, so a row is trivially cross-referenced
    -- against event_log and re-offers of the same bytes dedupe here. The
    -- dedupe is content-addressed and GLOBAL: identical bytes offered by two
    -- peers share one row (`peer` records the first shipper), and an `acked`
    -- license covers the BYTES, whichever link re-offers them — the exclusion
    -- decision is about content, not about a link.
    content_digest BYTEA PRIMARY KEY,
    signed_bytes   BYTEA NOT NULL,
    -- The attestation pair that travelled with the event on the wire (db/020):
    -- preserved so a requeued suppressing event can still face the token gate.
    -- A re-offer that carries a token a previous offer lacked enriches the row
    -- (COALESCE in the dedupe UPDATE) — a token once seen is never dropped.
    attestation    BYTEA,
    attester_key   BYTEA,
    -- Which link shipped it (sync_state.peer naming) — the mixed-version
    -- diagnosis ("peer X appears pre-ADR-0040") groups on this.
    peer           TEXT        NOT NULL,
    -- The legible verify-failure reason AT QUARANTINE TIME (EventError Display /
    -- cairn_verify_error vocabulary): 'signing-context mismatch …' vs 'signature
    -- verification failed' vs 'COSE: …' — an operator must be able to tell
    -- wire-format skew from tampering without a debugger (principle 4: honest,
    -- legible failure). Never overwritten by requeue — the door's CURRENT
    -- refusal goes to last_requeue_error so the original forensics survive.
    reason         TEXT        NOT NULL,
    first_seen     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    last_seen      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    -- How many pull cycles have re-offered these bytes (re-offer bumps
    -- last_seen/seen_count rather than duplicating the row).
    seen_count     INTEGER     NOT NULL DEFAULT 1,
    -- The apply door's refusal from the most recent `cairn-sync requeue`, kept
    -- SEPARATE from `reason` (see above) so a transient DB error during requeue
    -- can never destroy the original diagnosis.
    last_requeue_error TEXT,
    last_requeue_at    TIMESTAMPTZ,
    -- Human license to exclude: an operator who has inspected the row and
    -- accepts that these bytes will never enter the record sets this TRUE
    -- (`UPDATE sync_quarantine SET acked = TRUE WHERE content_digest = …`).
    -- An acked row no longer pins the re-offer floor and no longer fails the
    -- pull — the exclusion is now a recorded, attributable human decision.
    acked          BOOLEAN     NOT NULL DEFAULT FALSE
);

-- Additive upgrades for a pen created by an earlier revision of this file
-- (CREATE TABLE IF NOT EXISTS above no-ops on an existing table, so new
-- columns must also arrive as idempotent ALTERs — principle 11 applied to our
-- own migration).
ALTER TABLE sync_quarantine ADD COLUMN IF NOT EXISTS last_requeue_error TEXT;
ALTER TABLE sync_quarantine ADD COLUMN IF NOT EXISTS last_requeue_at    TIMESTAMPTZ;
ALTER TABLE sync_quarantine ADD COLUMN IF NOT EXISTS acked BOOLEAN NOT NULL DEFAULT FALSE;

-- Backs the per-peer quota probes (`WHERE peer = … AND NOT acked`, issue #197):
-- acked rows are retained indefinitely (a resolved human decision, never
-- auto-deleted) and are the one part of the pen the quota no longer bounds, so
-- without this the probes' scan set grows with every acked flood. Partial on
-- the probes' own predicate keeps them O(unacked rows).
CREATE INDEX IF NOT EXISTS sync_quarantine_peer_unacked_idx
    ON sync_quarantine (peer) WHERE NOT acked;

-- The re-offer floor itself lives beside the watermark it constrains (additive,
-- idempotent — sync_state is created in db/001). NULL = no unresolved
-- quarantine for this peer; when set, pulls fetch from min(watermark, floor).
ALTER TABLE sync_state ADD COLUMN IF NOT EXISTS quarantine_floor_wall    BIGINT;
ALTER TABLE sync_state ADD COLUMN IF NOT EXISTS quarantine_floor_counter INTEGER;

-- Explicit floor (the A6 pattern): nothing implicit for PUBLIC; the sync
-- runtime role gets exactly the DML the pull/requeue loop needs. DELETE is
-- granted deliberately — see the header — because releasing a row after a
-- successful requeue is the table's whole lifecycle, and admission itself is
-- still gated by apply_remote_event.
REVOKE ALL ON sync_quarantine FROM PUBLIC;
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_node') THEN
        CREATE ROLE cairn_node NOLOGIN;
    END IF;
END $$;
GRANT SELECT, INSERT, UPDATE, DELETE ON sync_quarantine TO cairn_node;

COMMIT;
