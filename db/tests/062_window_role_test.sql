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

    ASSERT NOT has_table_privilege('cairn_window', 'vault_member', 'SELECT'),
        'window must not read the sealed key blobs';
    ASSERT NOT has_table_privilege('cairn_window', 'vault_attempt', 'SELECT'), 'no vault_attempt read';
    ASSERT NOT has_table_privilege('cairn_window', 'vault_terminal', 'SELECT'), 'no vault_terminal read';

    ASSERT NOT has_function_privilege('cairn_window', 'cairn_vault_unseal(bytea, text)', 'EXECUTE'),
        'window must not call the unseal door';
    ASSERT NOT has_function_privilege('cairn_window', 'apply_remote_event(bytea, bytea, bytea, bytea)', 'EXECUTE'),
        'window must not call the peer-apply door';
    ASSERT NOT has_function_privilege('cairn_window', 'submit_node_event(bytea)', 'EXECUTE'),
        'window must not call the node-event door';
    ASSERT NOT has_table_privilege('cairn_window', 'event_log', 'INSERT'),
        'window must not write the event log directly';
END $$;

ROLLBACK;
