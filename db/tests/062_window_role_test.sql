\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/062 — the window's own group role, asserted by EFFECTIVE privilege.
--
-- WHY: the reference window used to run on the schema OWNER's connection and so could read the
-- vault's sealed key blobs (#898). cairn_window must hold only what the window needs. A role
-- inherits its parent's grants, so has_*_privilege tests exactly what a login granted it can do.
-- Nothing here commits.

BEGIN;

DO $$
BEGIN
    ASSERT (SELECT NOT rolcanlogin FROM pg_roles WHERE rolname = 'cairn_window'),
        'cairn_window is a NOLOGIN group role; logins are granted it';
    ASSERT pg_has_role('cairn_window', 'cairn_agent', 'MEMBER'),
        'cairn_window must inherit cairn_agent (the advisory-actor role), never cairn_node';
    ASSERT NOT pg_has_role('cairn_window', 'cairn_node', 'MEMBER'),
        'cairn_window must NOT be a cairn_node member (that role holds the peer-apply doors)';

    ASSERT has_table_privilege('cairn_window', 'local_node', 'SELECT'), 'window reads local_node';
    ASSERT has_table_privilege('cairn_window', 'node_schema', 'SELECT'), 'window reads node_schema';

    -- has_any_column_privilege, not has_table_privilege: a grant of ONE column must fail too.
    ASSERT NOT has_any_column_privilege('cairn_window', 'vault_member', 'SELECT'),
        'window must not read the sealed key blobs';
    ASSERT NOT has_table_privilege('cairn_window', 'vault_attempt', 'SELECT'), 'no vault_attempt read';
    ASSERT NOT has_table_privilege('cairn_window', 'vault_terminal', 'SELECT'), 'no vault_terminal read';

    ASSERT NOT has_function_privilege('cairn_window', 'cairn_vault_unseal(bytea, text)', 'EXECUTE'),
        'window must not call the unseal door';
    ASSERT NOT has_function_privilege('cairn_window', 'apply_remote_event(bytea, bytea, bytea, bytea)', 'EXECUTE'),
        'window must not call the peer-apply door';
    ASSERT NOT has_function_privilege('cairn_window', 'submit_node_event(bytea)', 'EXECUTE'),
        'window must not call the node-event door';
    ASSERT NOT has_any_column_privilege('cairn_window', 'event_log', 'INSERT'),
        'window must not write the event log directly';
    ASSERT NOT has_schema_privilege('cairn_window', 'public', 'CREATE'),
        'window must not create in public: every vault door resolves names there (db/061)';
    ASSERT NOT EXISTS (SELECT 1 FROM pg_namespace n, aclexplode(n.nspacl) a
                        WHERE n.nspname = 'public' AND a.grantee = 0 AND a.privilege_type = 'CREATE'),
        'PUBLIC must not create in public (a cluster upgraded from PG14 or older still grants it)';

    -- The grant inventory's narrow grants stay as narrow as they were written.
    ASSERT NOT has_column_privilege('cairn_window', 'actor_event', 'actor_id', 'SELECT'),
        'the window reads one actor_event column, signing_key_id, and no other';
    ASSERT NOT has_column_privilege('cairn_window', 'node_unwrap_key', 'unwrap_pub', 'SELECT'),
        'the window asks only that an unwrap key exists, never reads it';
    ASSERT NOT has_column_privilege('cairn_window', 'ui_gesture_timing', 'gesture_kind', 'UPDATE')
       AND NOT has_column_privilege('cairn_window', 'ui_gesture_timing', 'size_bucket', 'UPDATE'),
        'an UPDATE may move a gesture aggregate, never re-key its cell';
    ASSERT NOT has_column_privilege('cairn_window', 'name_repudiation', 'reason', 'SELECT'),
        'a repudiation''s reason is forensic free text no runtime role reads (ADR-0006)';
    ASSERT NOT has_table_privilege('cairn_window', 'ui_gesture_timing', 'DELETE'),
        'the window may add to the gesture aggregates, never erase them';
    ASSERT NOT has_table_privilege('cairn_agent', 'ui_gesture_timing', 'INSERT'),
        'the window''s gesture grant must not leak to the advisory agents (db/044)';
END $$;

ROLLBACK;
