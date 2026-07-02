-- db/021_sync_quarantine.sql
-- Cairn — durable quarantine for unverifiable replicated events (issue #108).
--
-- WHY: the clinical-plane pull loop (cairn-sync `do_pull`) skips an event whose
-- signature does not verify, so a corrupt frame — or a peer still serving
-- pre-ADR-0040 uncontextualized signatures — cannot wedge the link forever.
-- Before this table the only evidence of a skip was a transient stderr line and
-- an in-memory counter, while later verifiable events still advanced the
-- watermark past the skipped ones: a *silent* permanent set-union violation
-- (the exact class the A1 watermark fix exists to prevent). ADR-0040 made the
-- trigger universal: a mixed-version peer now fails verification for its entire
-- pre-upgrade history, not the odd corrupt frame.
--
-- WHAT: a node-local (never synced, like `sync_state`) holding pen. Every event
-- the pull loop skips as unverifiable is persisted here VERBATIM — signed bytes
-- plus any attestation that travelled with it — together with the legible
-- verify-failure reason (ContextMismatch vs BadSignature etc., ADR-0040) so an
-- operator can (a) see exactly what was excluded and why, and (b) re-process it
-- through the real apply door (`cairn-sync requeue`) after fixing the cause
-- (e.g. a version-skewed daemon binary). The pull loop may advance the
-- watermark past a skipped event ONLY once its durable trace exists here; if
-- this INSERT fails, the watermark freezes exactly as for a valid-but-unapplied
-- event (A1 discipline: delayed, never lost).
--
-- This is operational daemon state, not clinical content: rows here are
-- *candidate* events that never passed the floor, so the table sits beside
-- `sync_state`, outside the append-only event plane, and DELETE (on successful
-- requeue) is legitimate — removing a quarantine row never removes anything
-- from the record, because admission happens only through `apply_remote_event`.

BEGIN;

CREATE TABLE IF NOT EXISTS sync_quarantine (
    -- Content address (sha2-256 multihash) of the signed bytes — the same
    -- addressing the event plane uses, so a row is trivially cross-referenced
    -- against event_log and re-offers of the same bytes dedupe here.
    content_digest BYTEA PRIMARY KEY,
    signed_bytes   BYTEA NOT NULL,
    -- The attestation pair that travelled with the event on the wire (db/020):
    -- preserved so a requeued suppressing event can still face the token gate.
    attestation    BYTEA,
    attester_key   BYTEA,
    -- Which link shipped it (sync_state.peer naming) — the mixed-version
    -- diagnosis ("peer X appears pre-ADR-0040") groups on this.
    peer           TEXT        NOT NULL,
    -- The legible verify-failure reason (EventError Display / cairn_verify_error
    -- vocabulary): 'signing-context mismatch …' vs 'signature verification
    -- failed' vs 'COSE: …' — an operator must be able to tell wire-format skew
    -- from tampering without a debugger (principle 4: honest, legible failure).
    reason         TEXT        NOT NULL,
    first_seen     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    last_seen      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    -- How many pull cycles have re-offered these bytes (re-offer bumps
    -- last_seen/seen_count rather than duplicating the row).
    seen_count     INTEGER     NOT NULL DEFAULT 1
);

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
