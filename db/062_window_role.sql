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
-- The narrow grants below the role's creation each carry a one-line reason.

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_window') THEN
        CREATE ROLE cairn_window NOLOGIN;
    END IF;
END $$;

-- The doors and projections the window already uses are cairn_agent's.
GRANT cairn_agent TO cairn_window;

-- The window shows which node and schema it is on; both are small, non-secret tables.
GRANT SELECT ON local_node, node_schema TO cairn_window;

-- ---------------------------------------------------------------------------
-- What the window's own commands need beyond cairn_agent (#898, the grant inventory).
-- Found by running every cairn_node entry point the window calls as a login granted this
-- role (crates/cairn-node/tests/window_role_grants.rs); each is the narrowest form that works.
-- ---------------------------------------------------------------------------

-- Every window write (registration, a link judgement, a medication act) stamps its HLC through
-- this SECURITY DEFINER tick; the table it advances stays unwritable.
-- Residual, stated plainly: the tick takes hlc_state's one row FOR UPDATE (db/007), so a window
-- login that holds a transaction open after a tick stalls EVERY writer's next tick (the daemon's
-- sync included) until it lets go. That is a denial of service by someone who already holds the
-- window's login, never a bypass; provision-window-role bounds it by giving each window login
-- `idle_in_transaction_session_timeout` (crates/cairn-node/src/window_access.rs).
GRANT EXECUTE ON FUNCTION node_hlc_tick() TO cairn_window;

-- The window's start-up check of its own node key ("Retired" vs "never enrolled",
-- actor_enrolment::device_actor_standing) asks only whether the key ever appeared.
GRANT SELECT (signing_key_id) ON actor_event TO cairn_window;

-- Medication writes check the node has an unwrap key before sealing (ensure_unwrap_key asks
-- only that a row exists); one non-key column is enough, the public key itself is not needed.
GRANT SELECT (singleton) ON node_unwrap_key TO cairn_window;

-- The "Same person" / review panels' name read (patient::compare::chart_facts) keeps every
-- retained name but strikes the repudiated ones, so it anti-joins on (subject, value). Those two
-- columns only: `reason` is forensic free text db/025 deliberately grants to nobody (ADR-0006).
GRANT SELECT (subject, value) ON name_repudiation TO cairn_window;

-- The §1.2 gesture timer (ui_timing::record_gesture) upserts one aggregate cell per sign-off,
-- stop, hold and release. db/044 keeps this table from cairn_agent on purpose (an agent could
-- poison the paper-parity evidence); the window is the one writer it names. No DELETE, and an
-- UPDATE may move only the aggregate, never re-key a cell.
GRANT SELECT, INSERT ON ui_gesture_timing TO cairn_window;
GRANT UPDATE (n, p50_ms, p95_ms) ON ui_gesture_timing TO cairn_window;
