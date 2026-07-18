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
# Connection: standard libpq environment (PGHOST/PGPORT/PGUSER/…), so CI and local
# rigs differ only in env. The role must be allowed to CREATE DATABASE and CREATE
# EXTENSION cairn_pgx (CI uses the cluster superuser; so does a local rig).
#
# Usage:
#   PGHOST=127.0.0.1 PGPORT=5532 scripts/run-db-sql-tests.sh [dbname]
#   dbname defaults to cairn_sqltest.

set -euo pipefail

cd "$(dirname "$0")/.."   # repo root: db/ paths below are relative to it

DBNAME="${1:-cairn_sqltest}"

echo "== recreating throwaway database ${DBNAME}"
dropdb --if-exists "$DBNAME"
createdb "$DBNAME"
psql -d "$DBNAME" -v ON_ERROR_STOP=1 -q -c "CREATE EXTENSION cairn_pgx;"

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
