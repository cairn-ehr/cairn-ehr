-- SQL mirror of crates/cairn-node/tests/sensitivity_* (see db/tests/README.md).
-- DESTRUCTIVE: runs only against a database marked disposable (#169).
\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql

DO $$
BEGIN
    ASSERT cairn_sensitivity_rank('routine') = 0, 'routine ranks 0';
    ASSERT cairn_sensitivity_rank('sensitive') < cairn_sensitivity_rank('restricted'),
        'the ladder is ordered';
    ASSERT cairn_sensitivity_rank('restricted') < cairn_sensitivity_rank('sequestered'),
        'the ladder is ordered';
    ASSERT cairn_sensitivity_rank('grade:protected-witness') = 2147483647,
        'an unrecognised grade ranks MAX (inverting db/040 deliberately — ADR-0062)';
    ASSERT cairn_sensitivity_rank(NULL) = 2147483647, 'NULL lands on the safe side';
END $$;
