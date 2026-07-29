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

# ---- run_with_timeout: descendants killed (group kill) ----
t_setup
run_with_timeout 1 sh -c 'sleep 37 & wait'
t_assert_eq "forked descendant killed with parent" "124" "$?"
t_assert_fail "child process reaped" pgrep -f "sleep 37"
t_teardown

# ---- run_with_timeout: elapsed-time timeout (SIGTERM-trap evasion) ----
t_setup
run_with_timeout 1 bash -c 'trap "exit 0" TERM; while :; do sleep 1; done'
t_assert_eq "SIGTERM trap to exit 0 still times out" "124" "$?"
t_teardown

# ---- acquire_lock: empty pid file grace period ----
t_setup
LOOP_HOME="$T_TMP/loophome2"
mkdir -p "$LOOP_HOME/lock"
# No pid file: a race-window stale lock from an in-flight write
t_assert_ok "empty lock dir reclaimed after grace" acquire_lock
release_lock
t_teardown

# ---- acquire_lock: PID reuse safety (unrelated process) ----
t_setup
LOOP_HOME="$T_TMP/loophome3"
mkdir -p "$LOOP_HOME"
# Start a long-lived unrelated process and grab its PID
sleep 41 &
UNRELATED_PID=$!
mkdir -p "$LOOP_HOME/lock"
echo "$UNRELATED_PID" > "$LOOP_HOME/lock/pid"
# acquire_lock should recognize it's not our loop and reclaim the lock
t_assert_ok "unrelated process PID treated as stale" acquire_lock
release_lock
# Clean up the background process
kill "$UNRELATED_PID" 2>/dev/null || true
wait "$UNRELATED_PID" 2>/dev/null || true
t_teardown

# ---- setup_labels (stubbed gh) ----
t_setup
STUB="$T_TMP/bin"
mkdir -p "$STUB"
cat > "$STUB/gh" <<'EOF'
#!/bin/bash
echo "$@" >> "${GH_CALLS:?}"
EOF
chmod +x "$STUB/gh"
GH_CALLS="$T_TMP/gh-calls"
export GH_CALLS
PATH="$STUB:$PATH" setup_labels
t_assert_eq "seven labels created" "7" "$(wc -l < "$GH_CALLS" | tr -d ' ')"
t_assert_eq "all creates are idempotent (--force)" "7" \
  "$(grep -c -- '--force' "$GH_CALLS")"
t_assert_ok "loop:ready label present" grep -q "label create loop:ready" "$GH_CALLS"
t_assert_ok "loop:failed label present" grep -q "label create loop:failed" "$GH_CALLS"
t_teardown

# ---- main loop scenarios (stubbed claude / gh / sleep / osascript) ----
# Scenario tests bypass the real timeout machinery: its watcher calls
# `sleep`, which the stub turns into an instant no-op, so the watcher would
# TERM the fake claude in a race. run_with_timeout has its own tests
# (Task 2); here we run the worker command directly.
run_with_timeout() { shift; "$@"; }

# make_stub_env SCENARIO_LINES... — builds a PATH shim dir with a fake
# `claude` that consumes one scenario line per invocation:
#   merged | skipped | dry | failed | failed-permission | smoke-ok
#     -> writes that outcome to $TECHDEBT_OUTCOME_FILE
#   rate-limit:EPOCH -> prints the CLI limit error, writes nothing
#   crash            -> writes nothing, exits 1
# Also stubs: `sleep` (records requested seconds instead of sleeping),
# `osascript` (no-op), `gh` (records argv, exits 0).
make_stub_env() {
  STUB="$T_TMP/bin"
  mkdir -p "$STUB"
  SCENARIO="$T_TMP/scenario"
  COUNTER="$T_TMP/count"
  : > "$SCENARIO"
  local line
  for line in "$@"; do echo "$line" >> "$SCENARIO"; done
  echo 0 > "$COUNTER"
  cat > "$STUB/claude" <<'EOF'
#!/bin/bash
# Evidence trail (review finding 2): record argv + the env-var contract the
# driver promised the worker, before doing anything else, so a wrong flag
# or wrong var name fails a test instead of passing silently.
echo "argv=$* smoke=${TECHDEBT_SMOKE:-} force=${TECHDEBT_FORCE_ISSUE:-} epics=${TECHDEBT_INCLUDE_EPICS:-}" \
  >> "${CLAUDE_CALLS:?}"
n=$(cat "${STUB_COUNTER:?}"); n=$((n + 1)); echo "$n" > "$STUB_COUNTER"
line=$(sed -n "${n}p" "${STUB_SCENARIO:?}")
case "$line" in
  rate-limit:*)
    echo "Claude AI usage limit reached|${line#rate-limit:}"
    exit 1 ;;
  crash)
    echo "unexpected death"
    exit 1 ;;
  *)
    printf '{"outcome":"%s","issue":11,"pr":null,"step":"test","detail":"denied: cargo test"}' \
      "$line" > "${TECHDEBT_OUTCOME_FILE:?}"
    exit 0 ;;
esac
EOF
  cat > "$STUB/sleep" <<'EOF'
#!/bin/bash
echo "$1" >> "${SLEEP_CALLS:?}"
EOF
  cat > "$STUB/osascript" <<'EOF'
#!/bin/bash
exit 0
EOF
  cat > "$STUB/gh" <<'EOF'
#!/bin/bash
exit 0
EOF
  chmod +x "$STUB/claude" "$STUB/sleep" "$STUB/osascript" "$STUB/gh"
  SLEEP_CALLS="$T_TMP/sleep-calls"
  : > "$SLEEP_CALLS"
  CLAUDE_CALLS="$T_TMP/claude-calls"
  : > "$CLAUDE_CALLS"
  export STUB_COUNTER="$COUNTER" STUB_SCENARIO="$SCENARIO" SLEEP_CALLS CLAUDE_CALLS
}

# run_main FLAGS... — run main with the stub PATH in a subshell so each
# scenario is isolated; captures exit code in MAIN_RC.
run_main() {
  ( PATH="$STUB:$PATH" LOOP_HOME="$T_TMP/loophome" main "$@" ) \
    > "$T_TMP/main-out" 2>&1
  MAIN_RC=$?
}

t_setup; make_stub_env merged merged dry
run_main
t_assert_eq "merged,merged,dry exits 0" "0" "$MAIN_RC"
t_assert_ok "summary reports 2 merged" grep -q "merged: 2" "$T_TMP/main-out"
t_teardown

t_setup; make_stub_env failed failed
run_main
t_assert_eq "two consecutive failures exit 1" "1" "$MAIN_RC"
t_teardown

t_setup; make_stub_env failed merged failed failed
run_main
t_assert_eq "failure counter resets on success" "1" "$MAIN_RC"
t_assert_ok "ran all four scenario calls" grep -q "^4$" "$COUNTER"
t_teardown

t_setup; make_stub_env failed-permission
run_main
t_assert_eq "permission denial exits 2 immediately" "2" "$MAIN_RC"
t_assert_ok "denied command surfaced" grep -q "denied: cargo test" "$T_TMP/main-out"
t_teardown

t_setup; make_stub_env "rate-limit:$(( $(date +%s) + 1000 ))" merged dry
run_main
t_assert_eq "rate limit then recovery exits 0" "0" "$MAIN_RC"
WAITED=$(head -1 "$SLEEP_CALLS")
t_assert_ok "slept roughly delta+300 (1250..1350)" \
  sh -c "[ \"$WAITED\" -ge 1250 ] && [ \"$WAITED\" -le 1350 ]"
t_teardown

t_setup; make_stub_env "rate-limit:$(( $(date +%s) + 90000 ))"
run_main --max-wait 1
t_assert_eq "wait beyond --max-wait exits 4" "4" "$MAIN_RC"
t_teardown

t_setup; make_stub_env crash dry
run_main
t_assert_eq "crash without limit pattern counts as failed, loop continues" "0" "$MAIN_RC"
t_assert_ok "summary reports 1 failed" grep -q "failed: 1" "$T_TMP/main-out"
t_teardown

t_setup; make_stub_env merged merged merged
run_main --max-issues 2
t_assert_eq "--max-issues 2 stops after 2" "0" "$MAIN_RC"
t_assert_ok "only two claude calls made" grep -q "^2$" "$COUNTER"
t_teardown

t_setup; make_stub_env smoke-ok
run_main --smoke
t_assert_eq "--smoke exits 0 on smoke-ok" "0" "$MAIN_RC"
t_assert_ok "only one claude call in smoke" grep -q "^1$" "$COUNTER"
t_teardown

t_setup; make_stub_env skipped dry
run_main
t_assert_eq "skipped continues without failure" "0" "$MAIN_RC"
t_assert_ok "summary reports 1 skipped" grep -q "skipped: 1" "$T_TMP/main-out"
t_teardown

# ---- single-shot mode exit code reflects the sole attempt's outcome ----
# (review finding 1: a scripting caller invoking --issue N or --smoke needs
# a non-zero exit when that one attempt failed, not a blanket exit 0.)
t_setup; make_stub_env failed
run_main --issue 11
t_assert_eq "single-shot failed exits 1" "1" "$MAIN_RC"
t_assert_ok "summary still reports 1 failed" grep -q "failed: 1" "$T_TMP/main-out"
t_teardown

t_setup; make_stub_env merged
run_main --issue 11
t_assert_eq "single-shot merged exits 0" "0" "$MAIN_RC"
t_assert_ok "--issue 11 passes force=11 to the worker" grep -q "force=11" "$CLAUDE_CALLS"
t_teardown

# ---- driver-contract fidelity (review finding 2): assert on the fake
# claude's recorded argv/env, not just on the outcome it produces, so a
# wrong flag name or wrong env var would fail these tests. ----
t_setup; make_stub_env merged dry
run_main
t_assert_ok "default invocation calls the /techdebt-next skill" \
  grep -q -- "-p /techdebt-next" "$CLAUDE_CALLS"
t_assert_ok "default invocation uses --permission-mode acceptEdits" \
  grep -q -- "--permission-mode acceptEdits" "$CLAUDE_CALLS"
t_assert_fail "default invocation never bypasses permissions" \
  grep -q -- "dangerously" "$CLAUDE_CALLS"
t_teardown

t_setup; make_stub_env merged dry
run_main --bypass
t_assert_ok "--bypass uses --dangerously-skip-permissions" \
  grep -q -- "--dangerously-skip-permissions" "$CLAUDE_CALLS"
t_assert_fail "--bypass does not also pass acceptEdits" \
  grep -q -- "acceptEdits" "$CLAUDE_CALLS"
t_teardown

t_setup; make_stub_env smoke-ok
run_main --smoke
t_assert_ok "--smoke sets TECHDEBT_SMOKE=1 for the worker" grep -q "smoke=1" "$CLAUDE_CALLS"
t_teardown

t_setup; make_stub_env merged dry
run_main --include-epics
t_assert_ok "--include-epics sets TECHDEBT_INCLUDE_EPICS=1 for the worker" \
  grep -q "epics=1" "$CLAUDE_CALLS"
t_teardown

# ---- parse_args direct unit test (review finding 3a): --timeout/--max-wait/
# --max-issues had no direct coverage; run parse_args itself rather than
# only inferring its effect through a full main() scenario. ----
t_setup
parse_args --timeout 5 --max-wait 2 --max-issues 3
t_assert_eq "parse_args sets TIMEOUT_HOURS" "5" "$TIMEOUT_HOURS"
t_assert_eq "parse_args sets MAX_WAIT_HOURS" "2" "$MAX_WAIT_HOURS"
t_assert_eq "parse_args sets MAX_ISSUES" "3" "$MAX_ISSUES"
# Reset config globals to their script defaults: this call runs in the
# top-level test shell (not inside a run_main subshell), so leaving these
# mutated would leak into later scenarios that rely on the defaults.
TIMEOUT_HOURS=3; MAX_WAIT_HOURS=6; MAX_ISSUES=0
t_teardown

# ---- main --setup-labels dispatch (review finding 3b): exits 0, creates
# all 7 labels via the stubbed gh, and never takes the run lock. ----
t_setup
STUB="$T_TMP/bin"
mkdir -p "$STUB"
cat > "$STUB/gh" <<'EOF'
#!/bin/bash
echo "$@" >> "${GH_CALLS:?}"
EOF
cat > "$STUB/claude" <<'EOF'
#!/bin/bash
exit 0
EOF
chmod +x "$STUB/gh" "$STUB/claude"
GH_CALLS="$T_TMP/gh-calls"
: > "$GH_CALLS"
export GH_CALLS
SETUP_LOOP_HOME="$T_TMP/loophome-setup"
( PATH="$STUB:$PATH" LOOP_HOME="$SETUP_LOOP_HOME" main --setup-labels ) \
  > "$T_TMP/setup-labels-out" 2>&1
SETUP_RC=$?
t_assert_eq "main --setup-labels exits 0" "0" "$SETUP_RC"
t_assert_eq "main --setup-labels creates exactly 7 labels" "7" \
  "$(grep -c "label create" "$GH_CALLS")"
t_assert_fail "main --setup-labels never takes the run lock" \
  [ -d "$SETUP_LOOP_HOME/lock" ]
t_teardown

t_summary
