-- db/tests/_scratch_database_guard.sql — shared preamble for every db/tests/*.sql mirror (#169).
--
-- WHY THIS EXISTS. The DB-gated Rust suites share the `cairn_test` / `cairn_test2` / `cairn_test3`
-- databases, and each of their tests TRUNCATEs the write tables *before* it runs but not after the
-- last one. So a finished `cargo test` leaves `event_log` / `actor_event` rows behind. That residue
-- is harmless to the Rust suites themselves (they serialize cluster-wide and truncate on entry),
-- but a mirror run BY HAND against the same database — `psql -d cairn_test -f db/tests/004_...` —
-- collides with it, and the collision surfaces as a baffling failure inside an unrelated floor
-- guard several files later. That is exactly how a spurious #152-guard collision appeared during
-- the #166 work.
--
-- WHAT IT DOES. It refuses, loudly and early, to let a mirror touch a shared rig database, and
-- names the runner that already does the right thing. This is the mirror image of the refusal
-- scripts/run-db-sql-tests.sh carries in the other direction: that script DROPs its target, so it
-- declines to be pointed at a `cairn_test*` database. Between the two, the shared databases and the
-- throwaway one can never be confused for each other in either direction.
--
-- WHY A REFUSAL AND NOT A `TRUNCATE`. Opening each mirror with an idempotent TRUNCATE would also
-- close the collision, but TRUNCATE does not fire the row-level append-only triggers — so seventeen
-- tracked files each starting with `TRUNCATE event_log, actor_event, patient_chart CASCADE` would
-- be a loaded gun in the repo of an append-only clinical record, one mistyped `-d` away from
-- silently erasing one. A refusal costs the same and cannot destroy anything.
--
-- HOW TO USE IT. Each mirror includes it with psql's include-relative directive, near the top and
-- before any statement that writes:
--
--     \ir _scratch_database_guard.sql
--
-- `\ir` resolves against the *including file's* directory, so a mirror runs correctly from any CWD.
-- The leading underscore in this filename keeps it out of the runner's `db/tests/[0-9]*.sql` glob:
-- it is a preamble, never itself a test.
--
-- This file is deliberately PURE SQL (no psql backslash commands), so the same bytes are executable
-- by psql and by a plain SQL client — which is how crates/cairn-node/tests/db_sql_mirror_scratch_guard.rs
-- pins the behaviour against the real file rather than against a copy of its text.

DO $$
DECLARE
    db text := current_database();
BEGIN
    -- Prefix match, not equality: cairn_test, cairn_test2 and cairn_test3 are all shared.
    IF starts_with(db, 'cairn_test') THEN
        RAISE EXCEPTION
            'refusing to run a db/tests mirror against the shared rig database "%": it carries '
            'residue from finished cargo test runs (issue #169), which collides with these '
            'fixtures. Run the mirrors against a throwaway database via '
            'scripts/run-db-sql-tests.sh (it creates and drops cairn_sqltest for you).', db;
    END IF;
END $$;
