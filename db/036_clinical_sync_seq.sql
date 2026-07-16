-- db/036_clinical_sync_seq.sql
-- Cairn — clinical-plane incremental-sync cursor keyed on a monotonic, node-LOCAL
-- insertion-order `seq` (issue #196, 2026-07-15 review finding B1). This ports the
-- #38 node-plane treatment (db/007 node_event.seq + sync_cursor) to the clinical
-- pull (cairn-sync do_pull).
--
-- WHY: do_pull cursored on the HLC watermark (sync_state.hlc_wall/hlc_counter) and
-- never swept, so an event landing in a peer's event_log with an HLC BELOW an
-- already-advanced watermark — a multi-hop arrival from a third node, or an L2
-- agent self-stamping an older hlc_wall — was never re-fetched: a silent set-union
-- / convergence violation (the flagship guarantee). A node-LOCAL insertion-order
-- `seq` fixes it: a newly-LEARNED event (whatever its HLC) always gets a fresh high
-- seq, so it always sorts ABOVE the puller's cursor and cannot be skipped. The
-- periodic full sweep (cairn-sync cmd_run, every FULL_SWEEP_EVERY cycles) is the
-- correctness floor for the residual BIGSERIAL out-of-order-commit gap; incremental
-- is the optimization, the sweep is the floor.
--
-- All changes are additive ALTERs (ADR-0012 / principle 11). No CREATE TABLE is
-- widened, so there is no migration_replay_widening.rs `WIDENED` entry to add: each
-- column's SOLE source is the idempotent ALTER here, uniform for fresh and upgraded
-- DBs alike (exactly how db/007 added node_event.seq). connect_and_load_schema
-- re-runs every migration each connect, so every statement below is idempotent.

BEGIN;

-- event_log.seq — the monotonic node-local cursor key. IDENTITY is assigned at
-- INSERT, so the submit_event / apply_remote_event INSERT column lists need no
-- change (they never name seq; GENERATED ALWAYS also forbids an explicit value).
-- Never signed, never on the wire core — sync transport metadata only (principle 12).
-- ADD COLUMN IF NOT EXISTS does not fire the append-only UPDATE/DELETE trigger.
ALTER TABLE event_log ADD COLUMN IF NOT EXISTS seq BIGINT GENERATED ALWAYS AS IDENTITY;
CREATE INDEX IF NOT EXISTS event_log_seq_idx ON event_log (seq);

-- sync_state.last_seq — the per-peer pull checkpoint (highest serving-node seq
-- pulled from `peer`). Advance-only, written by do_pull via a raw GREATEST UPDATE
-- (sync_state is node-local operational state, outside the append-only floor).
--
-- SUPERSEDES sync_state.hlc_wall / hlc_counter (the old HLC watermark), now
-- VESTIGIAL — kept, deprecated-in-place, never dropped. A DROP is the non-additive
-- move ADR-0012 / principle 11 forbid: an older cairn-sync binary reading this DB
-- (an expected fleet state, since schema version is decoupled from binary version)
-- still SELECTs hlc_wall and would break. Adding columns is downgrade-safe; dropping
-- one is not. "Never erase, always overlay."
ALTER TABLE sync_state ADD COLUMN IF NOT EXISTS last_seq BIGINT NOT NULL DEFAULT 0;

-- sync_state.quarantine_floor_seq — the seq re-offer floor: the seq of the first
-- unacked refused event, so incremental pulls fetch from min(last_seq, floor_seq-1)
-- and keep re-offering that slot until it is repaired or acked. A SEPARATE persisted
-- column (recomputed each cycle, cleared on a clean cycle) rather than a value
-- derived from the pen rows: this preserves the clinical self-clearing semantics — a
-- one-time corruption that heals stops re-shipping on the next clean cycle, while the
-- pen row survives as an audit trace (deriving from rows would re-ship forever until
-- a manual ack). SUPERSEDES the vestigial quarantine_floor_wall / _counter (kept).
-- NULL = no unresolved re-offer.
ALTER TABLE sync_state ADD COLUMN IF NOT EXISTS quarantine_floor_seq BIGINT;

-- sync_quarantine.refused_seq — FORENSICS: the serving seq at which an unverifiable
-- event was refused (recorded on INSERT, never overwritten). The re-offer POSITION
-- is driven by sync_state.quarantine_floor_seq above; this column is the durable
-- "at what seq" trace, mirroring db/022 node_event_quarantine.refused_seq. Legacy
-- rows default to 0.
ALTER TABLE sync_quarantine ADD COLUMN IF NOT EXISTS refused_seq BIGINT NOT NULL DEFAULT 0;

COMMIT;
