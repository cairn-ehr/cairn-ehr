-- db/022_node_event_quarantine.sql
-- Cairn — durable quarantine + re-offer floor for DETERMINISTICALLY-refused
-- node_events on the federation plane (issue #111). The node-plane sibling of
-- db/021's clinical-plane pen.
--
-- WHY: cairn-node's node_event pull loop (sync.rs::pull_into) applies each
-- pulled event through apply_remote_node_event (db/007). Before this, ANY
-- refusal was logged to stderr, bumped an in-memory counter, and the seq cursor
-- advanced past it — re-offered only on the FULL_SWEEP_EVERY cadence and
-- re-refused forever, with no durable, inspectable trace. For an UNVERIFIABLE
-- event (a corrupt frame, or a peer still serving pre-ADR-0040 uncontextualized
-- signatures) that is a *silent, permanent* set-union exclusion — exactly the A1
-- class db/021 exists to prevent, just on the other plane.
--
-- WHAT IS PENNED vs SKIPPED (the node plane differs from the clinical plane):
-- the node plane's steady state is deny-all — a serving peer streams events
-- authored by nodes THIS puller does not (yet) trust, and refusing them is
-- normal and self-healing (a later peer.added + a full sweep admits them). So
-- pull_into pens ONLY the events that will NEVER apply without repair —
-- signature/context-unverifiable bytes — and keeps skip-and-sweep for a
-- verifiable-but-refused event (untrusted author, or an event type this node has
-- no code for yet under the two-plane model: both resolve on a later sweep). A
-- transient DB/transport error freezes the cursor instead (retried next cycle).
--
-- THE RE-OFFER FLOOR (derived, not stored): quarantining an event records the
-- serving-node `seq` it was refused at (`refused_seq`). Every subsequent pull
-- fetches from min(cursor.last_seq, MIN(refused_seq) over this peer's UNACKED
-- rows), so a penned slot keeps being re-offered (deduping onto its row here)
-- while the cursor still advances for valid events — "delayed, never lost". The
-- floor is DERIVED from the pen (no separate column to keep in sync): it self-
-- heals the moment the cause is fixed — a re-offered event that now applies is
-- DELETEd from the pen on success (auto-requeue), and an operator who accepts a
-- permanent exclusion sets `acked = TRUE` (a recorded human decision, policy-
-- neutral mechanism). While any UNACKED row exists for a peer, the pull logs a
-- LOUD integrity line every cycle. No manual `requeue` command is needed — the
-- floor + full-sweep already re-offer, and success auto-releases.
--
-- This is operational daemon state, not clinical content (like db/021 and
-- sync_cursor): rows are *candidate* events that never passed the in-DB floor,
-- so DELETE is legitimate — admission itself happens only through
-- apply_remote_node_event. Deleting a row does not stop a peer that still serves
-- those bytes from re-offering them (the row simply reappears); `acked` is the
-- honest way to accept a permanent exclusion.
--
-- Node-plane deltas from db/021: keyed off the seq-ordered node plane, not the
-- HLC watermark, so the floor is a `refused_seq` on the row rather than a
-- sync_state column; and node_events carry no attestation concept, so there are
-- no attestation/attester_key columns. A separate table (not a reuse of
-- sync_quarantine) keeps the requeue door unambiguous — a node-plane row is only
-- ever re-applied through apply_remote_node_event, never the clinical door.

BEGIN;

CREATE TABLE IF NOT EXISTS node_event_quarantine (
    -- Content address (sha2-256 multihash) of the signed bytes — the same
    -- addressing node_event uses, so a re-offer of the same bytes dedupes here
    -- (content-addressed, global across links).
    content_digest BYTEA PRIMARY KEY,
    signed_bytes   BYTEA NOT NULL,
    -- Which link shipped it (the peer ADDRESS the node plane keys its cursor on).
    peer           TEXT        NOT NULL,
    -- The serving-node `seq` this event was refused at — the re-offer floor is
    -- MIN(refused_seq) over this peer's unacked rows. Lets a later pull fetch
    -- from just below the earliest still-unresolved refusal.
    refused_seq    BIGINT      NOT NULL,
    -- The legible refusal reason AT QUARANTINE TIME (apply_remote_node_event's
    -- message / cairn_verify_error vocabulary): an operator must be able to see
    -- WHY the bytes were excluded without a debugger (principle 4).
    reason         TEXT        NOT NULL,
    first_seen     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    last_seen      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    -- How many pull cycles have re-offered these bytes (re-offer bumps
    -- last_seen/seen_count rather than duplicating the row).
    seen_count     INTEGER     NOT NULL DEFAULT 1,
    -- Human license to exclude: an operator who has inspected the row and accepts
    -- that these bytes will never enter this node's record sets this TRUE
    -- (`cairn-node ack-quarantine <digest>`). An acked row no longer pins the
    -- re-offer floor and no longer fails the pull loudly.
    acked          BOOLEAN     NOT NULL DEFAULT FALSE
);

-- Explicit floor (the A6 pattern): nothing implicit for PUBLIC; the node runtime
-- role gets exactly the DML the pull loop needs. DELETE is granted deliberately
-- — auto-releasing a row once its cause is fixed (the bytes now apply) is the
-- table's whole lifecycle, and admission itself is still gated by
-- apply_remote_node_event.
REVOKE ALL ON node_event_quarantine FROM PUBLIC;
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_node') THEN
        CREATE ROLE cairn_node NOLOGIN;
    END IF;
END $$;
GRANT SELECT, INSERT, UPDATE, DELETE ON node_event_quarantine TO cairn_node;

COMMIT;
