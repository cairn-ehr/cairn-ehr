#!/usr/bin/env bash
# scripts/run-db-sql-tests.sh — run the SQL mirrors under db/tests/ (issue #212).
#
# WHY: db/tests/*.sql previously executed NOWHERE — not in CI, not via cargo test —
# so the SQL mirrors of Rust-side guards could drift silently (exactly how the
# twin-registry row-count drifted in #182, caught only by luck in #183). This script
# is the single entry point CI and a local rig share: same load, same order, same
# failure semantics (first failing file exits non-zero via ON_ERROR_STOP).
#
# The tests run against a THROWAWAY database (re-created on every run), never the
# cairn_test* databases the Rust/matcher suites share:
#   * several test files insert residue as the table owner by design;
#   * db/tests/008_surrogate_test.sql needs db/008_surrogate_projection.sql, a
#     spike-only migration the product loaders deliberately skip (issue #67) — it
#     may exist here precisely because this database is disposable.
#
# Connection: resolved by scripts/pg-target.sh, which discovers a running cluster meeting
# the schema's PostgreSQL floor and REFUSES one below it. Setting PGPORT names a cluster
# explicitly and it is used (still floor-checked); leaving it unset lets the resolver find
# one, and refuse rather than guess if several qualify. The role must be allowed to CREATE
# DATABASE and CREATE EXTENSION cairn_pgx (CI uses the cluster superuser; so does a local
# rig).
#
# ⚠️ WHY THE RESOLVER EXISTS, IN ONE SENTENCE: this script used to connect through the plain
# libpq defaults, which on a multi-cluster machine is whichever server the default socket
# happens to reach — on 2026-09-07 that was a PostgreSQL 16 instance, and the run failed 22
# migrations deep on a missing `max(bytea)` instead of saying "that server is too old".
#
# Usage:
#   scripts/run-db-sql-tests.sh [dbname]            # resolver picks the cluster
#   PGPORT=5532 scripts/run-db-sql-tests.sh [dbname] # or name one
#   dbname defaults to cairn_sqltest.

set -euo pipefail

cd "$(dirname "$0")/.."   # repo root: db/ paths below are relative to it

DBNAME="${1:-cairn_sqltest}"

# The database is DROPPED and recreated below — refuse the names the Rust and
# matcher suites share (cairn_test, cairn_test2, …) so a mistyped argument cannot
# nuke a standing rig.
case "$DBNAME" in
    cairn_test*)
        echo "refusing to run against '${DBNAME}': cairn_test* databases belong to the" >&2
        echo "Rust/matcher suites and this script DROPS its target. Use the default" >&2
        echo "(cairn_sqltest) or another throwaway name." >&2
        exit 2
        ;;
esac

# Resolve and floor-check the target BEFORE anything is created. A too-old or ambiguous
# cluster must be refused here, not discovered from a confusing failure once a database
# exists on it — and `pg-target.sh` prints the diagnosis, so this only has to relay the
# exit status.
PG_TARGET="$(scripts/pg-target.sh)" || exit 1
read -r PGHOST PGPORT <<<"$PG_TARGET"
export PGHOST PGPORT
echo "== using PostgreSQL at ${PGHOST}:${PGPORT}"

echo "== recreating throwaway database ${DBNAME}"
dropdb --if-exists "$DBNAME"
createdb "$DBNAME"
psql -d "$DBNAME" -v ON_ERROR_STOP=1 -q -c "CREATE EXTENSION cairn_pgx;"

# Mark the database disposable. Every mirror opens with db/tests/_scratch_database_guard.sql,
# which refuses to run unless this marker exists (issue #169) — an allow-list, so a mirror
# pointed at a shared rig database or, far worse, at a real node refuses by default rather than
# committing its fixtures there. The marker is the ONLY thing that makes a database eligible, and
# stamping it is trustworthy HERE because the two lines above just dropped and recreated this exact
# database — the marker states a fact this script itself made true, rather than one a caller
# asserted. (The database is recreated at the START of each run, so one is left standing in between;
# the mirrors' residue is why it is never reused without that recreate.)
psql -d "$DBNAME" -v ON_ERROR_STOP=1 -q \
    -c "CREATE TABLE IF NOT EXISTS cairn_scratch_database ();" \
    -c "COMMENT ON TABLE cairn_scratch_database IS 'db/tests mirrors may run here; see db/tests/_scratch_database_guard.sql';"

# Load EVERY migration in numeric order — including the spike-only 008 (see header).
# The db/*.sql prefixes are zero-padded, so lexicographic glob order IS numeric order.
echo "== loading db/*.sql"
for f in db/[0-9]*.sql; do
    psql -d "$DBNAME" -v ON_ERROR_STOP=1 -q -f "$f"
done

# Run the mirrors in numeric order. ON_ERROR_STOP makes psql exit non-zero on the
# first failed statement, and set -e stops the loop there — first failure is THE
# failure, with psql's error naming the file and line.
status=0
for t in db/tests/[0-9]*.sql; do
    echo "== ${t}"
    if ! psql -d "$DBNAME" -v ON_ERROR_STOP=1 -q -f "$t"; then
        echo "FAILED: ${t}" >&2
        status=1
        break
    fi
done

if [ "$status" -eq 0 ]; then
    echo "== all db/tests/*.sql passed"
fi
exit "$status"
