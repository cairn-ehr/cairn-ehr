#!/bin/bash
# Tests for scripts/pg-target.sh — the PostgreSQL target resolver and schema-floor guard.
#
# WHY THIS FILE EXISTS. The failure it guards happened, on 2026-09-07, and it cost a wrong
# diagnosis that reached a pull-request body. `scripts/run-db-sql-tests.sh` connected through
# the plain libpq defaults, which on that machine reached a LEGACY PostgreSQL 16 cluster
# rather than the PG18 rig. Nothing checked the server version, so the run failed 22
# migrations deep — first on a stale extension, then, once that was updated, on
# `max(bytea)`, an aggregate PostgreSQL only gained in 17. Neither error mentions the
# actual problem, which is that the server is below the floor `db/001_envelope.sql` states.
#
# The decision half is PURE — it is handed candidate lines and returns a verdict — so every
# case below runs with no PostgreSQL of any kind. That is deliberate: a guard whose tests
# need the very substrate it is guarding cannot be trusted to fail correctly when that
# substrate is wrong, which is exactly the situation it exists for.
#
# Run: bash scripts/tests/pg_target_test.sh
set -u

TESTS_RUN=0
TESTS_FAILED=0
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Source the resolver for its pure functions. The script guards its own entry point with
# `BASH_SOURCE[0] = $0`, so sourcing it defines the functions WITHOUT probing anything.
# shellcheck source=/dev/null
. "$SCRIPT_DIR/../pg-target.sh"

t_ok() {
  TESTS_RUN=$((TESTS_RUN + 1))
  echo "ok   - $1"
}

t_fail() {
  TESTS_RUN=$((TESTS_RUN + 1))
  TESTS_FAILED=$((TESTS_FAILED + 1))
  echo "FAIL - $1"
}

t_assert_eq() {
  if [ "$2" = "$3" ]; then t_ok "$1"; else t_fail "$1: expected [$2] got [$3]"; fi
}

t_assert_contains() {
  case "$3" in
    *"$2"*) t_ok "$1" ;;
    *) t_fail "$1: [$2] not found in [$3]" ;;
  esac
}

# Case-insensitive, for assertions about a WORD an operator must see rather than about the
# typography of a section heading — `UNREACHABLE` and `unreachable` both satisfy the intent,
# and pinning the casing makes the test fail on a purely cosmetic edit.
t_assert_contains_ci() {
  local hay lower
  hay="$(printf '%s' "$3" | tr '[:upper:]' '[:lower:]')"
  lower="$(printf '%s' "$2" | tr '[:upper:]' '[:lower:]')"
  case "$hay" in
    *"$lower"*) t_ok "$1" ;;
    *) t_fail "$1: [$2] not found (case-insensitively) in [$3]" ;;
  esac
}

# decide CANDIDATES... — run pg_target_decide over newline-separated `host port version`
# triples, capturing stdout, stderr and status separately. Each candidate's version is a
# `server_version_num` (160013, 180001, …) or the literal `unreachable`.
decide() {
  DECIDE_OUT=""
  DECIDE_ERR=""
  DECIDE_STATUS=0
  local errfile
  errfile="$(mktemp)"
  DECIDE_OUT="$(printf '%s\n' "$@" | pg_target_decide 2>"$errfile")" || DECIDE_STATUS=$?
  DECIDE_ERR="$(cat "$errfile")"
  rm -f "$errfile"
}

# ---------------------------------------------------------------------------
# The happy path, and the anti-vacuity floor for every refusal below.
# ---------------------------------------------------------------------------

decide "127.0.0.1 5532 180001"
t_assert_eq "one qualifying candidate is chosen" "127.0.0.1 5532" "$DECIDE_OUT"
t_assert_eq "and it exits 0" "0" "$DECIDE_STATUS"

# `>=`, not `>`. A cluster running exactly the floor major is the SUPPORTED case, and an
# off-by-one here would refuse the very version the project targets.
decide "127.0.0.1 5532 ${CAIRN_PG_FLOOR_MAJOR}0000"
t_assert_eq "exactly the floor major is accepted" "127.0.0.1 5532" "$DECIDE_OUT"

# ---------------------------------------------------------------------------
# The defect this guard was written for: a below-floor server must be REFUSED, and the
# refusal must name the version rather than letting a migration fail obscurely later.
# ---------------------------------------------------------------------------

decide "127.0.0.1 5432 160013"
t_assert_eq "a below-floor server is refused" "1" "$DECIDE_STATUS"
t_assert_contains "the refusal names the version found" "16" "$DECIDE_ERR"
t_assert_contains "and the floor it fell short of" "$CAIRN_PG_FLOOR_MAJOR" "$DECIDE_ERR"
t_assert_contains "and the port, so the operator knows WHICH cluster" "5432" "$DECIDE_ERR"
t_assert_eq "and prints no connection on stdout" "" "$DECIDE_OUT"

# ---------------------------------------------------------------------------
# Ambiguity is a refusal, never a silent pick. Two qualifying clusters is a question only
# the operator can answer, and guessing is how this script would drop a database on the
# wrong one — the same instinct as the rest of this codebase: never auto-resolve a clash.
# ---------------------------------------------------------------------------

decide "127.0.0.1 5532 180001" "127.0.0.1 5544 180004"
t_assert_eq "two qualifying candidates refuse rather than guess" "1" "$DECIDE_STATUS"
t_assert_contains "naming the first" "5532" "$DECIDE_ERR"
t_assert_contains "and the second" "5544" "$DECIDE_ERR"
t_assert_contains "and the way to decide it" "PGPORT" "$DECIDE_ERR"

# ---------------------------------------------------------------------------
# Unreachable is NOT the same verdict as too-old, and this project treats a wrong remedy
# to an operator as a defect in its own right. "Start your server" and "your server is too
# old" are opposite instructions.
# ---------------------------------------------------------------------------

decide "127.0.0.1 5432 unreachable"
t_assert_eq "an unreachable candidate refuses" "1" "$DECIDE_STATUS"
t_assert_contains_ci "and says so in those terms" "unreachable" "$DECIDE_ERR"

# A too-old server alongside an unreachable one must still report BOTH, so the operator is
# not sent to fix the wrong one first.
decide "127.0.0.1 5432 160013" "127.0.0.1 5599 unreachable"
t_assert_contains "a mixed rejection names the too-old one" "5432" "$DECIDE_ERR"
t_assert_contains "and the unreachable one" "5599" "$DECIDE_ERR"

# ---------------------------------------------------------------------------
# No candidates at all — the empty-input case, which must not read as success.
# ---------------------------------------------------------------------------

DECIDE_STATUS=0
DECIDE_OUT="$(printf '' | pg_target_decide 2>/dev/null)" || DECIDE_STATUS=$?
t_assert_eq "no candidates refuses" "1" "$DECIDE_STATUS"
t_assert_eq "and prints nothing" "" "$DECIDE_OUT"

# ---------------------------------------------------------------------------
# A qualifying candidate beside a rejected one is still chosen — the rejection list must
# not poison an otherwise-unambiguous answer.
# ---------------------------------------------------------------------------

decide "127.0.0.1 5432 160013" "127.0.0.1 5532 180001"
t_assert_eq "one qualifying beside one too-old is chosen" "127.0.0.1 5532" "$DECIDE_OUT"
t_assert_eq "and exits 0" "0" "$DECIDE_STATUS"

# ---------------------------------------------------------------------------
# LINE SELECTION — which `ps` lines are postmasters at all. Pure, and tested against the
# real shapes because the first version got it wrong in a way no parser test could catch:
# it required a space before `-D`, which is absent whenever `-D` is the FIRST argument —
# exactly how Postgres.app launches. The result was that discovery matched nothing, fell
# through to its libpq default, and probed the WRONG cluster while looking like it had
# searched. A guard that silently searches nothing is worse than no guard.
# ---------------------------------------------------------------------------

PS_SAMPLE="$(cat <<'PSEOF'
/Applications/Postgres.app/Contents/MacOS/Postgres
/Applications/Postgres.app/Contents/MacOS/PostgresMenuHelper.app/Contents/MacOS/PostgresMenuHelper
/Applications/Postgres 2.app/Contents/Versions/18/bin/postgres -D /Users/h/Library/Application Support/Postgres/var-18 -p 5532 -c shared_preload_libraries=x
/Applications/Postgres.app/Contents/Versions/16/bin/postgres -D /Users/h/Library/Application Support/Postgres/var-16 -p 5432 -c shared_preload_libraries=x
/usr/lib/postgresql/18/bin/postgres -D /var/lib/postgresql/18/main -c config_file=/etc/postgresql/18/main/postgresql.conf
postgres: io worker 0
postgres: checkpointer
postgres: background writer
grep -E postgres -D something
PSEOF
)"

SELECTED="$(printf '%s\n' "$PS_SAMPLE" | pg_target_postmaster_lines)"

t_assert_eq "three real postmasters are selected" "3" "$(printf '%s\n' "$SELECTED" | grep -c . )"
t_assert_contains "the macOS PG18 postmaster, whose -D is its FIRST argument" "var-18" "$SELECTED"
t_assert_contains "the macOS PG16 postmaster" "var-16" "$SELECTED"
t_assert_contains "the Debian-style postmaster" "/var/lib/postgresql/18/main" "$SELECTED"

# The noise must be excluded, and each of these is a shape actually present on the rig.
for noise in "MenuHelper" "io worker" "checkpointer" "background writer"; do
  case "$SELECTED" in
    *"$noise"*) t_fail "worker/GUI noise must not be taken for a postmaster: $noise" ;;
    *) t_ok "excluded: $noise" ;;
  esac
done

# End to end through the parsers, since selection feeding the wrong line into a correct
# parser is still a wrong answer.
PORTS="$(printf '%s\n' "$SELECTED" | while IFS= read -r l; do pg_target_port_from_args "$l"; done | sort -u | tr '\n' ' ')"
t_assert_eq "the selected lines yield exactly the ports they carry" "5432 5532 " "$PORTS"

# ---------------------------------------------------------------------------
# COMMAND-LINE PARSING, and the bug that made it a separate function. Discovery reads the
# process table; on the maintainer's Mac the postmasters run out of
# `/Users/…/Library/Application Support/Postgres/var-18` — a path WITH SPACES — and the
# first version of this took `-D` up to the first space, produced
# `/Users/hherb/Library/Application`, found no `postmaster.pid` there and silently dropped
# the only cluster that qualified. Silently: the PG18 rig was running the whole time and the
# resolver reported that no PG18 cluster existed, which is precisely the class of wrong
# answer this whole script was written to stop giving.
# ---------------------------------------------------------------------------

PGA="/Applications/Postgres 2.app/Contents/Versions/18/bin/postgres -D /Users/h/Library/Application Support/Postgres/var-18 -p 5532 -c shared_preload_libraries=x"
t_assert_eq "an explicit -p is read straight off the command line" \
    "5532" "$(pg_target_port_from_args "$PGA")"

# The -p form is preferred because it needs no filesystem access at all, but it is not
# universal (Debian/Ubuntu packaging puts the port in a config file), so the datadir path
# stays available for the postmaster.pid fallback — WITH its spaces intact.
t_assert_eq "the datadir survives spaces in the path" \
    "/Users/h/Library/Application Support/Postgres/var-18" \
    "$(pg_target_datadir_from_args "$PGA")"

NOPORT="/usr/lib/postgresql/18/bin/postgres -D /var/lib/postgresql/18/main -c config_file=/etc/postgresql/18/main/postgresql.conf"
t_assert_eq "no -p yields no port, rather than a wrong one" "" "$(pg_target_port_from_args "$NOPORT")"
t_assert_eq "and the datadir is still recovered for the pid-file fallback" \
    "/var/lib/postgresql/18/main" "$(pg_target_datadir_from_args "$NOPORT")"

# A datadir at the very end of the line, with no trailing flags to stop at.
TRAILING="/usr/lib/postgresql/18/bin/postgres -D /var/lib/postgresql/18/main"
t_assert_eq "a trailing datadir is not truncated" \
    "/var/lib/postgresql/18/main" "$(pg_target_datadir_from_args "$TRAILING")"

# `-p` must not match a longer flag that merely starts with p — a real hazard given
# postmasters carry `-c` settings whose values contain almost anything.
CONFUSABLE="/usr/lib/postgresql/18/bin/postgres -D /var/lib/pg -c port_hint=9999 -p 5544"
t_assert_eq "a -c value that looks like a port is not mistaken for one" \
    "5544" "$(pg_target_port_from_args "$CONFUSABLE")"

# ---------------------------------------------------------------------------
# THE FLOOR HAS TWO HOMES AND THIS BINDS THEM. `db/001_envelope.sql` is the source of
# truth for the schema's PostgreSQL floor; this script repeats the number because a shell
# script cannot read a SQL comment reliably. A repeated constant that nothing checks is
# how the twin-registry counts drifted (#182), so it is checked.
# ---------------------------------------------------------------------------

ENVELOPE_FLOOR="$(grep -oE 'PostgreSQL >= [0-9]+' "$REPO_ROOT/db/001_envelope.sql" | head -1 | grep -oE '[0-9]+$')"
t_assert_eq "db/001 states a floor this test could read" "1" "$([ -n "$ENVELOPE_FLOOR" ] && echo 1 || echo 0)"
t_assert_eq "and pg-target.sh agrees with it" "$ENVELOPE_FLOOR" "$CAIRN_PG_FLOOR_MAJOR"

# ---------------------------------------------------------------------------
# Candidate parsing: duplicates collapse. Discovery can legitimately find the same cluster
# twice (a running postmaster AND the libpq default pointing at it), and reporting that as
# "ambiguous" would refuse a perfectly unambiguous rig.
# ---------------------------------------------------------------------------

decide "127.0.0.1 5532 180001" "127.0.0.1 5532 180001"
t_assert_eq "the same cluster found twice is not ambiguous" "127.0.0.1 5532" "$DECIDE_OUT"
t_assert_eq "and exits 0" "0" "$DECIDE_STATUS"

echo
echo "ran $TESTS_RUN, failed $TESTS_FAILED"
[ "$TESTS_FAILED" -eq 0 ]
