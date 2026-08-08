-- db/tests/_scratch_database_guard.sql — shared preamble for every db/tests/*.sql mirror (#169).
--
-- WHY THIS EXISTS. The mirrors are destructive fixtures, and two different accidents can point
-- them at a database that should never see them:
--
--   * THE NUISANCE (issue #169, what prompted this). The DB-gated Rust suites share the
--     `cairn_test` / `cairn_test2` / `cairn_test3` databases, and each of their tests TRUNCATEs the
--     write tables *before* it runs but not after the last one. So a finished `cargo test` leaves
--     `event_log` / `actor_event` rows behind. That residue is harmless to the Rust suites (they
--     serialize cluster-wide and truncate on entry), but a mirror run by hand against the same
--     database — `psql -d cairn_test -f db/tests/004_...` — collides with it, and the collision
--     surfaces as a baffling failure inside an unrelated floor guard several files later. That is
--     exactly how a spurious #152-guard collision appeared during the #166 work.
--
--   * THE DAMAGE. Eight mirrors have no transaction wrapper and therefore COMMIT (005, 008, 009,
--     017, 020, 021, 022, 040), and 017 goes as far as `ALTER TABLE … DROP CONSTRAINT` and replaying
--     a migration. Pointed at a real node — one mistyped `-d` — that is a mutilated clinical record,
--     not a confusing test failure.
--
-- WHY AN ALLOW-LIST. Refusing a known-bad list of database *names* would close the first case and
-- leave the second wide open, since the dangerous target is precisely the one nobody thought to
-- name. So the polarity is inverted: a mirror refuses to run ANYWHERE unless the database carries an
-- explicit marker saying "I am a throwaway, feel free to wreck me". Both sanctioned runners stamp
-- that marker on the disposable database they create — scripts/run-db-sql-tests.sh (cairn_sqltest)
-- and db/bench/run_b5.sh (a bench database) — so the normal paths need no thought, and every other
-- database in the cluster, shared rig and real node alike, is refused by default. Fail closed.
--
-- WHY A REFUSAL AND NOT A `TRUNCATE`. Issue #169 suggested opening each mirror with an idempotent
-- TRUNCATE instead. That would also close the nuisance, but TRUNCATE does not fire the row-level
-- append-only triggers — so seventeen tracked files each starting with
-- `TRUNCATE event_log, actor_event, patient_chart CASCADE` would carry the damage case *inside*
-- them, one mistyped `-d` away from silently erasing a real record. A refusal costs the same and
-- cannot destroy anything.
--
-- HOW TO USE IT. Each mirror includes it near the top, before any statement that writes:
--
--     \set ON_ERROR_STOP on
--     \ir _scratch_database_guard.sql
--
-- `\ir` resolves against the *including file's* directory, so the include survives being run from
-- any working directory (individual mirrors may still carry their own CWD-relative `\i` lines).
-- `\set ON_ERROR_STOP on` must come first: without it psql prints the refusal and then carries on,
-- which is worse than no guard because it looks like one.
--
-- RUN MIRRORS WITH `-f`, NOT STDIN. `\ir` degrades to CWD-relative `\i` when psql reads the script
-- from stdin, so `psql … < db/tests/004_...sql` fails to find this file. That failure is closed, not
-- open — but use `-f` (as both runners do) and the question does not arise.
--
-- The leading underscore in this filename keeps it out of the runner's `db/tests/[0-9]*.sql` glob:
-- it is a preamble, never itself a test. The file is deliberately PURE SQL (no psql backslash
-- commands), so the same bytes are executable by psql and by a plain SQL client — which is how
-- crates/cairn-node/tests/db_sql_mirror_scratch_guard.rs pins the behaviour against the real file
-- rather than against a copy of its text.

DO $$
BEGIN
    -- `to_regclass` returns NULL rather than raising when the name does not resolve, which is what
    -- makes this a plain existence test and not itself a source of errors.
    IF to_regclass('public.cairn_scratch_database') IS NULL THEN
        RAISE EXCEPTION
            'refusing to run a db/tests mirror against database "%": it is not marked as a '
            'throwaway. These mirrors are destructive — several commit, and one drops constraints '
            'and replays a migration — so they run only where the marker table '
            '"public.cairn_scratch_database" exists (issue #169). Use scripts/run-db-sql-tests.sh, '
            'which creates, marks and drops cairn_sqltest for you.', current_database();
    END IF;
END $$;
