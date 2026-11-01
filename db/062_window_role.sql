-- db/062 — cairn_window: the reference window's own least-privilege group role (#898).
--
-- WHY: the window used to work on the schema OWNER's connection, which can read the vault's
-- sealed key blobs (vault_member) and call every door. cairn_window is NOLOGIN; a login
-- (made by `cairn-node provision-window-role`) is granted it, and the window runs as that login.
--
-- WHY IT INHERITS cairn_agent AND NEVER cairn_node: cairn_agent is the runtime/advisory role —
-- it may call the validated submit doors and read projections, with no direct write to the event
-- log. cairn_node additionally holds the peer-apply doors (apply_remote_event, submit_node_event),
-- which a clinician's window must never reach.
-- Design: docs/superpowers/specs/2026-11-01-window-connection-posture-898-899-design.md (D1).
--
-- Replay-safe: this file re-runs on every connect (guarded CREATE, idempotent GRANTs).
-- Task 3 appends further narrow grants BELOW, each with a one-line reason.

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_window') THEN
        CREATE ROLE cairn_window NOLOGIN;
    END IF;
END $$;

-- The doors and projections the window already uses are cairn_agent's.
GRANT cairn_agent TO cairn_window;

-- The window shows which node and schema it is on; both are small, non-secret tables.
GRANT SELECT ON local_node, node_schema TO cairn_window;
