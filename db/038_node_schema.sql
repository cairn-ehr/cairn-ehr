-- db/038_node_schema.sql
-- Cairn — the node's recorded schema generation (issue #188, 2026-07-15 review
-- finding D1): the first brick of the ADR-0012 code plane.
--
-- WHY: the loaders (cairn-node connect_and_load_schema on EVERY connect; cairn-sync
-- `init`) re-run every embedded db/*.sql against the database. `CREATE OR REPLACE`
-- makes that replay a silent DOWNGRADE when the binary is older than the database:
-- an older binary overwrites newer function bodies — including the in-DB safety-floor
-- checks — with no error and no trace. Two binary versions touching one DB (a pilot
-- mid-upgrade, a GUI sidecar, any second tool linking the loader) is all it takes.
--
-- This table records which schema generation last touched the database, so a loader
-- can REFUSE to replay when the record is newer than its own embedded list. The
-- generation is the numeric prefix of the newest migration a loader carries (this
-- file makes it 38); the row is stamped by the loader AFTER a successful replay.
-- No seed row here: an absent row means "generation unknown" (e.g. a rig loaded by
-- hand via psql) and the loaders proceed — the guard exists to stop a KNOWN
-- downgrade, not to lock out hand-built test rigs. This is a node-LOCAL operational
-- record (like sync_state): never signed, never on the wire core (principle 12).
--
-- connect_and_load_schema re-runs every migration each connect, so every statement
-- below is idempotent.

BEGIN;

CREATE TABLE IF NOT EXISTS node_schema (
    id           boolean     PRIMARY KEY DEFAULT TRUE CHECK (id),  -- singleton row
    version      integer     NOT NULL,
    loaded_at    timestamptz NOT NULL DEFAULT now(),
    loader_build text        NOT NULL
);

-- The runtime role may inspect the generation (status surfaces); only the schema
-- owner (the loader's connection) writes it. This file is part of cairn-sync's
-- SUBSET load too, which skips db/007 — so guard the role's existence here exactly
-- as db/020/021/022 (the other subset members) do.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_node') THEN
        CREATE ROLE cairn_node NOLOGIN;
    END IF;
END $$;
GRANT SELECT ON node_schema TO cairn_node;

COMMIT;
