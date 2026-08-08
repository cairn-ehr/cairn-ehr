#!/usr/bin/env bash
# Reproducible Bet B5 + leakage-guard runner for the dual-identifier discipline
# (ADR-0031 / data-model §3.18). It loads ONLY the projection-plane schema this
# needs (001 envelope + 002 projection + 008 surrogate projection) — no pgrx, no
# cairn_verify — then runs the mechanical leakage/interning guard and the
# size/read benchmark. See PI-RUNBOOK.md "B5" for running it on the Pi.
#
# Usage: db/bench/run_b5.sh <psql-conn> [patients] [notes_per]
#   e.g. db/bench/run_b5.sh "host=/var/run/postgresql dbname=cairn_b5 user=postgres" 5000 100
#
# DESTRUCTIVE: the benchmark TRUNCATEs the log and projections. Point it at a
# throwaway bench database, never a real node.
set -euo pipefail

CONN="${1:?usage: run_b5.sh <psql-conn> [patients] [notes_per]}"
PATIENTS="${2:-2000}"
NOTES="${3:-50}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"   # repo root

echo "== loading projection-plane schema (001 + 002 + 008) =="
for f in 001_envelope 002_projection 008_surrogate_projection; do
    psql "$CONN" -v ON_ERROR_STOP=1 -qf "$ROOT/db/$f.sql"
done

echo "== leakage / interning guard (db/tests/008_surrogate_test.sql) =="
psql "$CONN" -v ON_ERROR_STOP=1 -f "$ROOT/db/tests/008_surrogate_test.sql"

echo "== B5 size/read benchmark (db/bench/b5_surrogate.sql) =="
psql "$CONN" -v ON_ERROR_STOP=1 \
    -v patients="$PATIENTS" -v notes_per="$NOTES" \
    -f "$ROOT/db/bench/b5_surrogate.sql"
