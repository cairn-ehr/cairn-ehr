\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/072 — the in-DB mirror for the supervision doors: the PURE address each signed act is bound
-- to (its exact preimage, restated here so a change to the formula is a failing line), and the
-- grants a raw-SQL client meets. The Rust suite (crates/cairn-node/tests/supervision_doors.rs)
-- drives the doors and their matrix end to end.
--
-- Wrapped in a transaction that ROLLS BACK: nothing here commits.

BEGIN;

-- 1. The address: '\x1220' || sha256 of newline-joined fields, staff IDs normalised, the
--    supervisor '' for an op that has none, and the family's prior seq last.
DO $$
BEGIN
    ASSERT cairn_supervision_address('link-supervisor', ' BWong ', ' JSmith ', 7) = '\x1220'::bytea
        || sha256(convert_to('cairn-supervision' || E'\n' || 'link-supervisor' || E'\n' || 'bwong'
                             || E'\n' || 'jsmith' || E'\n' || '7', 'UTF8')),
        'the link preimage';
    ASSERT cairn_supervision_address('require-cosign', 'bwong', NULL, 0) = '\x1220'::bytea
        || sha256(convert_to('cairn-supervision' || E'\n' || 'require-cosign' || E'\n' || 'bwong'
                             || E'\n' || '' || E'\n' || '0', 'UTF8')),
        'no supervisor is the empty field';
    ASSERT cairn_supervision_address('link-supervisor', 'bwong', 'jsmith', 0)
        <> cairn_supervision_address('link-supervisor', 'bwong', 'jsmith', 1),
        'a token is good for the state it changes, and only that';
    ASSERT cairn_supervision_address('link-supervisor', 'bwong', 'jsmith', 0)
        <> cairn_supervision_address('unlink-supervisor', 'bwong', 'jsmith', 0),
        'the op is bound';
    ASSERT cairn_supervision_address(NULL, 'bwong', NULL, 0) IS NULL,
        'a NULL field makes no address, which no token verifies against';
    ASSERT has_function_privilege('public', 'cairn_supervision_address(text, text, text, bigint)', 'EXECUTE'),
        'the pure address reads nothing, so PUBLIC keeps it';
END $$;

-- 2. The grants: the window reaches the doors it calls; the console door and the helpers are the
--    owner's alone; PUBLIC runs no definer.
DO $$
DECLARE f text;
BEGIN
    FOREACH f IN ARRAY ARRAY[
        'cairn_supervision_act(text, text, text, text, bytea)',
        'cairn_supervision_prior(text, text, text)',
        'cairn_supervision_staff()'
    ] LOOP
        ASSERT has_function_privilege('cairn_agent', f, 'EXECUTE'), format('the window must reach %s', f);
        ASSERT NOT has_function_privilege('public', f, 'EXECUTE'), format('%s is revoked from PUBLIC', f);
    END LOOP;
    ASSERT has_function_privilege('cairn_window', 'cairn_supervision_record()', 'EXECUTE')
       AND NOT has_function_privilege('cairn_agent', 'cairn_supervision_record()', 'EXECUTE')
       AND NOT has_function_privilege('public', 'cairn_supervision_record()', 'EXECUTE'),
        'the record is the window''s, as db/071''s reader doors are';
    FOREACH f IN ARRAY ARRAY[
        'cairn_supervision_console_act(text, text, text)',
        'cairn_supervision_approver(text, bytea, bytea)',
        'cairn_supervision_apply(text, text, text, text, bytea, boolean)'
    ] LOOP
        ASSERT NOT has_function_privilege('cairn_agent', f, 'EXECUTE'), format('%s is the owner''s alone', f);
        ASSERT NOT has_function_privilege('public', f, 'EXECUTE'), format('%s is revoked from PUBLIC', f);
    END LOOP;
END $$;

ROLLBACK;
