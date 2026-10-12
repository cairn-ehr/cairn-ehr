-- db/058_schema_replay_stamp.sql
-- Cairn — the loader records WHAT it replayed, so an unchanged schema is never replayed (#726).
--
-- WHY: cairn-node's loader used to re-run every db/*.sql on every connect. Each statement is
-- idempotent, but idempotent is not lock-free: a no-op replay was measured taking ACCESS
-- EXCLUSIVE on event_log and twenty other tables plus 34 views (CREATE OR REPLACE VIEW, DROP
-- TRIGGER IF EXISTS + CREATE TRIGGER, ALTER TABLE … ADD COLUMN IF NOT EXISTS), and SHARE /
-- SHARE ROW EXCLUSIVE on more (CREATE INDEX IF NOT EXISTS, CREATE OR REPLACE TRIGGER). Each
-- such lock waits behind any open reader — the matcher worker's multi-second read, say — and
-- PostgreSQL queues every later clinical write behind the waiting lock. Starting a node
-- process could therefore stall charting. The matcher reads committed data and blocks no
-- writer; it was the replay's DDL that turned an ordinary read into a write stall.
--
-- WHAT: after a successful replay the loader stamps this column with a sha2-256 of exactly
-- the migration texts it replayed (cairn_event::schema_generation::schema_content_hash), and
-- skips the whole file replay on a later connect whose generation, loader_build AND hash all
-- match. A content hash rather than the generation number, because an in-place edit of an
-- existing function body changes no generation and must still reach every database.
--
-- NULL means "replay next time": a database stamped before this column existed, one stamped
-- by cairn-sync (whose ON CONFLICT update does not touch this column, and whose loader_build
-- differs anyway), or one whose stamp a test deliberately cleared (db::force_schema_replay).
-- Node-LOCAL operational record like the rest of node_schema: never signed, never on the wire.
--
-- Not in cairn-sync's subset: cairn-sync never reads or writes the column. Idempotent.
-- (This ALTER itself takes ACCESS EXCLUSIVE on node_schema — a one-row table only loaders
-- read — and from now on runs only when the schema actually changed.)

BEGIN;

ALTER TABLE node_schema ADD COLUMN IF NOT EXISTS schema_hash bytea;

COMMIT;
