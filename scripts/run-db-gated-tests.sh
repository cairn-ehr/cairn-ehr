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
# The CLUSTER is resolved by scripts/pg-target.sh rather than assumed. This script used to
# default to `PGPORT:-5532`, which is true of exactly one machine in the world and silently
# wrong everywhere else; the resolver discovers a running cluster meeting the schema's
# PostgreSQL floor, refuses one below it, and refuses to guess when several qualify. Name a
# cluster with PGPORT to skip discovery, or set the full CAIRN_TEST_PG* strings yourself and
# they are honored untouched (role defaults to the current user; databases cairn_test/2/3 —
# docs/HANDOVER.md "Test env").
set -euo pipefail

cd "$(dirname "$0")/.."   # repo root, same convention as the sibling scripts

# Closed surface: no arguments. The allowlist rule ends in a wildcard, so an
# argument passthrough here would silently widen what a worker can run.
if [ "$#" -ne 0 ]; then
    echo "run-db-gated-tests.sh takes no arguments (env is baked in; override via PGHOST/PGPORT/PGUSER or CAIRN_TEST_PG*)" >&2
    exit 2
fi

# Only resolve when the caller has not already supplied the full connection strings: if all
# three are set there is nothing left to discover, and probing would refuse a rig the caller
# has legitimately pointed somewhere this script cannot see (a remote cluster, a tunnel).
if [ -z "${CAIRN_TEST_PG:-}" ] || [ -z "${CAIRN_TEST_PG2:-}" ] || [ -z "${CAIRN_TEST_PG3:-}" ]; then
    PG_TARGET="$(scripts/pg-target.sh)" || exit 1
    read -r PGHOST PGPORT <<<"$PG_TARGET"
fi
export PGHOST="${PGHOST:-127.0.0.1}"
export PGPORT="${PGPORT:-5432}"
PG_ROLE="${PGUSER:-${USER:-$(id -un)}}"
export CAIRN_TEST_PG="${CAIRN_TEST_PG:-host=$PGHOST port=$PGPORT user=$PG_ROLE dbname=cairn_test}"
export CAIRN_TEST_PG2="${CAIRN_TEST_PG2:-host=$PGHOST port=$PGPORT user=$PG_ROLE dbname=cairn_test2}"
export CAIRN_TEST_PG3="${CAIRN_TEST_PG3:-host=$PGHOST port=$PGPORT user=$PG_ROLE dbname=cairn_test3}"

# Exporting PGHOST/PGPORT above means the child takes the explicit-wins branch and probes
# only the cluster already chosen here, rather than repeating discovery — and it prints the
# target itself, so this script does not echo the same line twice.
scripts/run-db-sql-tests.sh

echo "== cargo test --workspace against ${CAIRN_TEST_PG}"
cargo test --workspace
