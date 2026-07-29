#!/bin/bash
# Tests for scripts/techdebt-loop.sh — the tech-debt loop driver.
#
# How this works: the driver ends with a guard so that when TECHDEBT_TEST=1
# it defines its functions but does not run main. We source it here and test
# each function directly. Later tasks add scenario tests that stub the
# `claude` / `gh` / `sleep` binaries via a PATH shim directory.
#
# Run: bash scripts/tests/techdebt-loop-test.sh
set -u

TESTS_RUN=0
TESTS_FAILED=0
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DRIVER="$SCRIPT_DIR/../techdebt-loop.sh"

# t_setup: fresh scratch dir per test so tests never share state.
t_setup() {
  T_TMP="$(mktemp -d)"
}

t_teardown() {
  rm -rf "$T_TMP"
}

# t_assert_eq DESC EXPECTED ACTUAL — the workhorse assertion.
t_assert_eq() {
  TESTS_RUN=$((TESTS_RUN + 1))
  if [ "$2" = "$3" ]; then
    echo "ok   - $1"
  else
    TESTS_FAILED=$((TESTS_FAILED + 1))
    echo "FAIL - $1: expected [$2] got [$3]"
  fi
}

# t_assert_ok / t_assert_fail DESC CMD... — assert a command's exit status.
t_assert_ok() {
  local desc="$1"; shift
  TESTS_RUN=$((TESTS_RUN + 1))
  if "$@"; then echo "ok   - $desc"; else
    TESTS_FAILED=$((TESTS_FAILED + 1)); echo "FAIL - $desc: expected success"; fi
}
t_assert_fail() {
  local desc="$1"; shift
  TESTS_RUN=$((TESTS_RUN + 1))
  if "$@"; then
    TESTS_FAILED=$((TESTS_FAILED + 1)); echo "FAIL - $desc: expected failure"
  else echo "ok   - $desc"; fi
}

t_summary() {
  echo "----"
  echo "$TESTS_RUN tests, $TESTS_FAILED failed"
  [ "$TESTS_FAILED" -eq 0 ]
}

# Source the driver in test mode: functions defined, main not run.
# shellcheck disable=SC2034 # TECHDEBT_TEST passed to driver to control guard
TECHDEBT_TEST=1
# shellcheck source=../techdebt-loop.sh
# shellcheck disable=SC1091 # source directive above tells shellcheck to follow
. "$DRIVER"

# ---- parse_reset_epoch ----
t_setup
printf 'blah\nClaude AI usage limit reached|1753900000\nmore\n' > "$T_TMP/t1"
t_assert_eq "parse_reset_epoch extracts epoch" "1753900000" "$(parse_reset_epoch "$T_TMP/t1")"
printf 'no limits here\n' > "$T_TMP/t2"
t_assert_eq "parse_reset_epoch empty when absent" "" "$(parse_reset_epoch "$T_TMP/t2")"
t_teardown

# ---- is_rate_limited ----
t_setup
printf 'Claude AI usage limit reached|1753900000\n' > "$T_TMP/a"
printf "You've hit your session limit. resets 4pm\n" > "$T_TMP/b"
printf 'normal transcript text\n' > "$T_TMP/c"
t_assert_ok  "is_rate_limited matches epoch form" is_rate_limited "$T_TMP/a"
t_assert_ok  "is_rate_limited matches prose form" is_rate_limited "$T_TMP/b"
t_assert_fail "is_rate_limited ignores normal text" is_rate_limited "$T_TMP/c"
t_teardown

# ---- classify_result ----
t_setup
printf '{"outcome":"merged","issue":227,"pr":301,"step":"cleanup","detail":""}' > "$T_TMP/out.json"
printf 'Claude AI usage limit reached|1753900000\n' > "$T_TMP/rl"
printf 'irrelevant\n' > "$T_TMP/plain"
t_assert_eq "outcome.json wins over transcript" "merged" \
  "$(classify_result "$T_TMP/out.json" "$T_TMP/rl")"
t_assert_eq "no outcome + limit pattern = rate-limited" "rate-limited" \
  "$(classify_result "$T_TMP/absent.json" "$T_TMP/rl")"
t_assert_eq "no outcome + no pattern = failed" "failed" \
  "$(classify_result "$T_TMP/absent.json" "$T_TMP/plain")"
printf 'not json at all' > "$T_TMP/garbage.json"
t_assert_eq "garbage outcome.json = failed" "failed" \
  "$(classify_result "$T_TMP/garbage.json" "$T_TMP/plain")"
t_teardown

# ---- compute_wait_secs ----
t_assert_eq "future epoch: delta + 5min buffer" "1300" "$(compute_wait_secs 1000001000 1000000000)"
t_assert_eq "no epoch: 30min fallback" "1800" "$(compute_wait_secs "" 1000000000)"
t_assert_eq "past epoch: 30min fallback" "1800" "$(compute_wait_secs 999999000 1000000000)"

# ---- run_with_timeout ----
t_setup
run_with_timeout 5 true
t_assert_eq "fast command exit 0 passes through" "0" "$?"
run_with_timeout 5 sh -c 'exit 7'
t_assert_eq "fast command exit code passes through" "7" "$?"
START=$(date +%s)
run_with_timeout 1 sleep 30
RC=$?
ELAPSED=$(( $(date +%s) - START ))
t_assert_eq "hanging command killed with 124" "124" "$RC"
t_assert_ok "hanging command killed promptly" [ "$ELAPSED" -lt 10 ]
t_teardown

# ---- lock ----
t_setup
LOOP_HOME="$T_TMP/loophome"
mkdir -p "$LOOP_HOME"
t_assert_ok "first acquire succeeds" acquire_lock
t_assert_fail "second acquire fails while holder alive" acquire_lock
release_lock
t_assert_ok "acquire after release succeeds" acquire_lock
release_lock
# stale lock: a PID that cannot exist is dead; the lock must be reclaimed
mkdir -p "$LOOP_HOME/lock"
echo 999999 > "$LOOP_HOME/lock/pid"
t_assert_ok "stale lock (dead pid) is reclaimed" acquire_lock
release_lock
t_teardown

t_summary
