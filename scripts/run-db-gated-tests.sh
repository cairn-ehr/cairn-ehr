#!/usr/bin/env bash
# scripts/run-db-gated-tests.sh — the DB-substrate slice of the local gate,
# with the connection environment BAKED IN.
#
# WHY THIS EXISTS (2026-07-31, first techdebt-loop run): headless worker
# sessions run under a permission allowlist whose rules are PREFIX matches on
# the command string. A leading env-var assignment defeats every rule —
# `PGHOST=… bash scripts/run-db-sql-tests.sh` and `CAIRN_TEST_PG=… cargo test`
# both start with `VAR=value`, match nothing, and stop the whole run with a
# permission denial. Baking the env into a script gives the gate a single
# allowlistable shape: `scripts/run-db-gated-tests.sh`.
#
# WHAT IT RUNS, in order (first failure exits non-zero):
#   1. the SQL mirrors under db/tests/ via scripts/run-db-sql-tests.sh
#      (throwaway database; see that script's header), then
#   2. the FULL workspace `cargo test` with CAIRN_TEST_PG/PG2/PG3 exported so
#      the DB-gated suites actually run — they self-skip when the env is
#      unset, so a plain `cargo test` is a strict SUBSET of this run.
#
# Since #450 that subset is no longer SILENT: a `cargo test` without the three
# variables fails `db_gate_actually_ran`, naming what is missing, rather than
# skipping and printing `ok`. Running it without a database is still fine — it
# just has to be declared, with CAIRN_ALLOW_DB_SKIP=1. This script never sets
# that: it exists precisely to run the tier the opt-out waives.
#
# Defaults target the standard local rig (PG18 + cairn_pgx on 127.0.0.1:5532,
# role = current user, databases cairn_test/2/3 — docs/HANDOVER.md "Test env").
# Override individual pieces via PGHOST/PGPORT/PGUSER, or set the full
# CAIRN_TEST_PG* strings yourself and they are honored untouched.
set -euo pipefail

cd "$(dirname "$0")/.."   # repo root, same convention as the sibling scripts

# Closed surface: no arguments. The allowlist rule ends in a wildcard, so an
# argument passthrough here would silently widen what a worker can run.
if [ "$#" -ne 0 ]; then
    echo "run-db-gated-tests.sh takes no arguments (env is baked in; override via PGHOST/PGPORT/PGUSER or CAIRN_TEST_PG*)" >&2
    exit 2
fi

export PGHOST="${PGHOST:-127.0.0.1}"
export PGPORT="${PGPORT:-5532}"
PG_ROLE="${PGUSER:-${USER:-$(id -un)}}"
export CAIRN_TEST_PG="${CAIRN_TEST_PG:-host=$PGHOST port=$PGPORT user=$PG_ROLE dbname=cairn_test}"
export CAIRN_TEST_PG2="${CAIRN_TEST_PG2:-host=$PGHOST port=$PGPORT user=$PG_ROLE dbname=cairn_test2}"
export CAIRN_TEST_PG3="${CAIRN_TEST_PG3:-host=$PGHOST port=$PGPORT user=$PG_ROLE dbname=cairn_test3}"

scripts/run-db-sql-tests.sh
cargo test --workspace
