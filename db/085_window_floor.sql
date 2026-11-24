-- db/085_window_floor.sql
-- Cairn — the loader records the window FLOOR beside the generation (#909 slice C1).
--
-- WHY: until now an older reference-UI window refused to run against any database whose
-- generation differed from its own, so every upgrade disturbed every open window. The floor
-- is the oldest window generation the stamped schema still serves; a window at or above it
-- keeps working, and the loader raises it only when a change would make an old window
-- misbehave (cairn_event::schema_generation::WINDOW_FLOOR_LEDGER holds the history).
--
-- WHAT: three columns on the one-row node_schema stamp: the floor, the reason an old window
-- would miss something (shown to the clinician), and window_floor_for — the generation the
-- floor was written FOR. A reader trusts the floor only when window_floor_for equals the
-- stamp's version: cairn-sync stamps node_schema with its own ON CONFLICT and never touches
-- these columns, so after it runs they describe an older generation and must read as absent.
--
-- Node-LOCAL operational record like the rest of node_schema: never signed, never on the wire.
-- Not in cairn-sync's subset: cairn-sync never reads or writes these columns. Idempotent.

BEGIN;

ALTER TABLE node_schema
    ADD COLUMN IF NOT EXISTS window_floor int,
    ADD COLUMN IF NOT EXISTS window_floor_reason text,
    ADD COLUMN IF NOT EXISTS window_floor_for int;

COMMIT;
