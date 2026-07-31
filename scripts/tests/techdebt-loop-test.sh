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
t_assert_eq "no outcome + no pattern = crashed (post-mortem decides)" "crashed" \
  "$(classify_result "$T_TMP/absent.json" "$T_TMP/plain")"
printf 'not json at all' > "$T_TMP/garbage.json"
t_assert_eq "garbage outcome.json = crashed (post-mortem decides)" "crashed" \
  "$(classify_result "$T_TMP/garbage.json" "$T_TMP/plain")"
# A worker that finished and HONESTLY reported failure is a failure, never a
# crash — the post-mortem must not second-guess a deliberate outcome write.
printf '{"outcome":"failed","issue":11,"pr":null,"step":"gate","detail":"x"}' > "$T_TMP/honest.json"
t_assert_eq "worker-written failed stays failed" "failed" \
  "$(classify_result "$T_TMP/honest.json" "$T_TMP/plain")"
t_teardown

# ---- merged_loop_pr_after (crash post-mortem, issue #320) ----
# The timing filter is load-bearing: a loop/<n>-* PR merged BEFORE this
# iteration began (e.g. a stale in-progress label left by an ancient crash)
# must never be adopted as THIS cycle's success. Start epoch 1000000000 is
# 2001; the two stub mergedAt values straddle it. The stub is a shell
# FUNCTION shadowing the gh binary (no PATH games, no subshell needed);
# unset -f restores the real gh for everything below.
t_setup
# shellcheck disable=SC2329 # invoked indirectly, via merged_loop_pr_after
gh() {
  printf '[{"headRefName":"loop/424242-fix-slug","mergedAt":"%s"}]' "${GH_MERGED_AT:?}"
}
GH_MERGED_AT="2999-01-01T00:00:00Z"
merged_loop_pr_after 424242 1000000000; RC=$?
t_assert_eq "loop/<n> PR merged after iteration start is found" "0" "$RC"
merged_loop_pr_after 4242 1000000000; RC=$?
t_assert_eq "issue number must match exactly (4242 != 424242)" "1" "$RC"
GH_MERGED_AT="2000-01-01T00:00:00Z"
merged_loop_pr_after 424242 1000000000; RC=$?
t_assert_eq "merge older than iteration start is ignored" "1" "$RC"
unset GH_MERGED_AT
unset -f gh
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
# The watcher polls the deadline in 5s chunks, so a 1s timeout fires at
# ~5s; 15 leaves load headroom while still discriminating from the 30s hang.
t_assert_ok "hanging command killed promptly" [ "$ELAPSED" -lt 15 ]
t_teardown

# ---- lock ----
# NOTE: "holder alive" passes because acquire_lock greps the holder's ps
# command line for "techdebt-loop" — and THIS test script's own path
# contains that string. Renaming this test file would break the assertion
# below (a false "stale lock" reclaim), not the lock itself.
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
# Pins ownership semantics only — the two-contender reclaim race that the
# atomic `mv` closes is not deterministically testable from one process.
t_assert_eq "reclaimed lock records the reclaimer's pid" "$$" \
  "$(cat "$LOOP_HOME/lock/pid")"
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

# ---- run_with_timeout: no full-duration watcher sleep may exist ----
# Old bug: the watcher armed itself with one `sleep SECS`; reaping the
# watcher raced bash 3.2 signal delivery and could orphan that sleep for
# up to the full per-iteration timeout (hours) — and a survivor holds the
# caller's stdout pipe open besides. The fix polls the deadline in <=5s
# chunks, so a process whose command line is the FULL duration ("sleep
# 971") must never exist at all — if one appears, someone reverted to a
# long arming sleep. 971 is a distinctive duration for pgrep. The command
# must OUTLIVE the watcher's setup (hence /bin/sleep 1, not `true`): with
# an instant command the old watcher never reached its arming sleep either,
# and this test would pass vacuously against the old code.
t_setup
run_with_timeout 971 /bin/sleep 1
for _ in 1 2 3 4 5 6 7 8 9 10; do
  pgrep -f "sleep 971" >/dev/null || break
  /bin/sleep 0.2
done
t_assert_fail "no full-duration watcher sleep after fast completion" \
  pgrep -fl "sleep 971"
# Insurance against a regression: a survivor would hold this suite's
# stdout PIPE open for 971s whenever the suite runs piped (grep/tee).
pkill -f "sleep 971" >/dev/null 2>&1 || true
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
# Eight: the seven classification/lifecycle labels plus loop:agent-filed, which
# is PROVENANCE and coexists with any of them. The count is pinned so adding a
# label is a deliberate act that updates this line and its sibling below.
t_assert_eq "eight labels created" "8" "$(wc -l < "$GH_CALLS" | tr -d ' ')"
t_assert_eq "all creates are idempotent (--force)" "8" \
  "$(grep -c -- '--force' "$GH_CALLS")"
t_assert_ok "loop:ready label present" grep -q "label create loop:ready" "$GH_CALLS"
t_assert_ok "loop:failed label present" grep -q "label create loop:failed" "$GH_CALLS"
t_assert_ok "loop:agent-filed label present" \
  grep -q "label create loop:agent-filed" "$GH_CALLS"
t_teardown

# ---- main loop scenarios (stubbed claude / gh / sleep / osascript) ----
# Scenario tests bypass the real timeout machinery: its watcher calls
# `sleep`, which the stub turns into an instant no-op, so the watcher would
# TERM the fake claude in a race. run_with_timeout has its own tests
# (Task 2); here we run the worker command directly — but backgrounded and
# explicitly `wait`ed (like the real function), not called synchronously in
# the foreground. Verified empirically: bash only notices a pending TERM/INT
# trap promptly while blocked in the `wait` builtin; a plain foreground
# command defers trap handling until that command itself exits. The
# TERM/INT-terminates-the-loop regression test below depends on the prompt
# behavior to observe a signal reaction within seconds instead of minutes.
#
# ORDERING IS LOAD-BEARING: everything below this override sees the STUB.
# Any new test of the REAL run_with_timeout must go ABOVE this line, or it
# will silently pass against the stub instead of the function under test.
run_with_timeout() {
  shift
  "$@" &
  local child_pid=$!
  wait "$child_pid"
}

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
  hang-real)
    # Uses the REAL sleep (absolute path), not the shimmed no-op that's
    # ahead of it on PATH: this token exists so a worker can genuinely
    # still be running when the TERM/INT signal-handling test below
    # signals the driver, instead of the instant-return other scenarios.
    /bin/sleep 30 ;;
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
  # git is stubbed too: the crash post-mortem's finish_adopted_cycle runs
  # `git worktree remove` / `git branch -D` / `git push origin --delete`,
  # which must never touch the developer's real repo from a test. Records
  # argv; prints nothing (individual tests overwrite it to answer queries).
  cat > "$STUB/git" <<'EOF'
#!/bin/bash
echo "$@" >> "${GIT_CALLS:?}"
EOF
  chmod +x "$STUB/claude" "$STUB/sleep" "$STUB/osascript" "$STUB/gh" "$STUB/git"
  SLEEP_CALLS="$T_TMP/sleep-calls"
  : > "$SLEEP_CALLS"
  CLAUDE_CALLS="$T_TMP/claude-calls"
  : > "$CLAUDE_CALLS"
  GIT_CALLS="$T_TMP/git-calls"
  : > "$GIT_CALLS"
  export STUB_COUNTER="$COUNTER" STUB_SCENARIO="$SCENARIO" SLEEP_CALLS CLAUDE_CALLS GIT_CALLS
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
t_assert_eq "main --setup-labels creates exactly 8 labels" "8" \
  "$(grep -c "label create" "$GH_CALLS")"
t_assert_fail "main --setup-labels never takes the run lock" \
  [ -d "$SETUP_LOOP_HOME/lock" ]
t_teardown

# ---- TERM/INT must actually terminate the loop (review finding, task 7) ----
# Old bug: `trap release_lock EXIT INT TERM` ran release_lock on TERM/INT
# but never exited — bash resumes the interrupted loop afterward, so a
# `kill <pid>` released the lock while the driver kept running and spawned
# another worker; a subsequent /techdebt-loop launch then sees no lock and
# double-launches. This scenario's fake claude genuinely hangs (real sleep,
# not the shimmed no-op) so the worker is still "running" when we signal it.
t_setup; make_stub_env hang-real
(
  PATH="$STUB:$PATH" LOOP_HOME="$T_TMP/loophome" main
) > "$T_TMP/main-out" 2>&1 &
MAIN_BG_PID=$!
# Synchronize on the worker having actually STARTED (its first action is
# the CLAUDE_CALLS append): freshly-written stubs can take seconds to
# first-exec on macOS, and TERMing before the spawn would race the test.
TRIES=0
while [ "$TRIES" -lt 40 ]; do
  [ -s "$CLAUDE_CALLS" ] && break
  /bin/sleep 0.5; TRIES=$((TRIES + 1))
done
kill -TERM "$MAIN_BG_PID" 2>/dev/null
wait "$MAIN_BG_PID" 2>/dev/null
sleep 1   # settle: let the EXIT trap's release_lock finish before we check
t_assert_fail "TERM terminates the loop: lock is released" \
  [ -d "$T_TMP/loophome/lock" ]
t_assert_eq "TERM terminates the loop: no post-kill respawn" "1" \
  "$(cat "$COUNTER")"
# Best-effort cleanup: the fake claude's real `/bin/sleep 30` is orphaned
# once main exits (never signaled itself) and finishes on its own shortly.
pkill -f '/bin/sleep 30' >/dev/null 2>&1 || true
t_teardown

# ---- TERM during a RATE-LIMIT WAIT must terminate promptly (review) ----
# Old bug: the rate-limited arm slept in the FOREGROUND. Bash defers trap
# handling until a foreground command exits, so `kill <pid>` during a
# usage-limit wait (30 min to hours) did nothing until the sleep ran out —
# and the lock stayed held, so a re-launch reported "already running".
# The fix (interruptible_sleep) backgrounds the sleep and blocks in the
# `wait` builtin, where traps fire immediately. To discriminate, this
# scenario's sleep stub sleeps for REAL (30s): the buggy driver outlives
# TERM by that whole sleep, the fixed one dies within the 2s poll below.
# Synchronization matters: freshly-written stub scripts can take seconds
# to FIRST-exec on macOS (new-binary scan), so we wait for the driver's
# own "usage limit hit" log line — proof it is entering the wait — rather
# than signaling after a fixed delay and racing the spawn.
t_setup; make_stub_env "rate-limit:$(( $(date +%s) + 1000 ))"
cat > "$STUB/sleep" <<'EOF'
#!/bin/bash
echo "$1" >> "${SLEEP_CALLS:?}"
exec /bin/sleep 30
EOF
chmod +x "$STUB/sleep"
(
  PATH="$STUB:$PATH" LOOP_HOME="$T_TMP/loophome" main
) > "$T_TMP/main-out" 2>&1 &
MAIN_BG_PID=$!
TRIES=0   # allow up to 20s for the slow first-exec path
while [ "$TRIES" -lt 40 ]; do
  grep -q "usage limit hit" "$T_TMP/main-out" 2>/dev/null && break
  /bin/sleep 0.5; TRIES=$((TRIES + 1))
done
t_assert_ok "driver reached the rate-limit wait" \
  grep -q "usage limit hit" "$T_TMP/main-out"
/bin/sleep 0.3   # let it enter interruptible_sleep's wait builtin
kill -TERM "$MAIN_BG_PID" 2>/dev/null
TRIES=0   # fixed driver dies well within 2s; buggy one lives ~30s more
while [ "$TRIES" -lt 10 ] && kill -0 "$MAIN_BG_PID" 2>/dev/null; do
  /bin/sleep 0.2; TRIES=$((TRIES + 1))
done
t_assert_fail "TERM during rate-limit wait kills the driver promptly" \
  sh -c "kill -0 $MAIN_BG_PID 2>/dev/null"
wait "$MAIN_BG_PID" 2>/dev/null
t_assert_fail "rate-limit wait TERM releases the lock" \
  [ -d "$T_TMP/loophome/lock" ]
# The orphaned real sleep exits on its own; reap best-effort anyway.
pkill -f '/bin/sleep 30' >/dev/null 2>&1 || true
t_teardown

# ---- crash post-mortem scenarios (issue #320) ----
# A worker that ends its turn with a background wait pending dies instantly
# (headless sessions terminate at turn end) and never writes outcome.json —
# even when its actual work SUCCEEDED. The driver must consult GitHub before
# counting such a crash as a failure: two false failures in a row would trip
# the systemic-halt breaker and strand the rest of the run.

# (A) LANDED: the dead worker's PR already auto-merged and the issue closed.
# The cycle is a merge; the driver also finishes the dead worker's label
# cleanup (the stale loop:in-progress would otherwise linger forever).
t_setup; make_stub_env crash dry
GH_CALLS="$T_TMP/gh-calls"; : > "$GH_CALLS"; export GH_CALLS
cat > "$STUB/gh" <<'EOF'
#!/bin/bash
echo "$@" >> "${GH_CALLS:?}"
case "$*" in
  *"issue list"*"--state closed"*) echo "424242" ;;
  *"pr list"*"--state merged"*)
    printf '[{"headRefName":"loop/424242-fix-slug","mergedAt":"2999-01-01T00:00:00Z"}]' ;;
  *) exit 0 ;;
esac
EOF
chmod +x "$STUB/gh"
# git stub answers the branch discovery so the cleanup path is exercised.
cat > "$STUB/git" <<'EOF'
#!/bin/bash
echo "$@" >> "${GIT_CALLS:?}"
case "$*" in
  *"branch --list"*) echo "loop/424242-fix-slug" ;;
esac
EOF
chmod +x "$STUB/git"
run_main
t_assert_eq "crashed-but-landed cycle exits 0" "0" "$MAIN_RC"
t_assert_ok "landed crash counted as merged" grep -q "merged: 1" "$T_TMP/main-out"
t_assert_ok "landed crash adds no failure" grep -q "failed: 0" "$T_TMP/main-out"
t_assert_ok "stale loop:in-progress removed from the adopted issue" \
  grep -q "issue edit 424242 --remove-label loop:in-progress" "$GH_CALLS"
t_assert_ok "adopted worktree removed" grep -q "worktree remove" "$GIT_CALLS"
t_assert_ok "adopted local branch deleted" \
  grep -q -- "branch -D loop/424242-fix-slug" "$GIT_CALLS"
t_assert_ok "adopted remote branch deleted" \
  grep -q -- "push origin --delete loop/424242-fix-slug" "$GIT_CALLS"
t_teardown

# (B) PENDING: the dead worker armed auto-merge and died waiting for CI.
# The driver adopts the CI watch: polls the PR until it merges, then counts
# the cycle merged. (pr view answers OPEN twice, then MERGED.)
t_setup; make_stub_env crash dry
GH_CALLS="$T_TMP/gh-calls"; : > "$GH_CALLS"; export GH_CALLS
GH_PRVIEW_COUNT="$T_TMP/prview-count"; echo 0 > "$GH_PRVIEW_COUNT"; export GH_PRVIEW_COUNT
cat > "$STUB/gh" <<'EOF'
#!/bin/bash
echo "$@" >> "${GH_CALLS:?}"
case "$*" in
  *"issue list"*"--state closed"*) : ;;
  *"issue list"*"--state open"*) echo "424242" ;;
  *"pr list"*"--state open"*)
    printf '[{"number":9999,"headRefName":"loop/424242-fix-slug","autoMergeRequest":{"enabledAt":"2026-01-01T00:00:00Z"}}]' ;;
  *"pr view 9999"*)
    v="$(cat "${GH_PRVIEW_COUNT:?}")"; v=$((v + 1)); echo "$v" > "$GH_PRVIEW_COUNT"
    if [ "$v" -ge 3 ]; then echo "MERGED"; else echo "OPEN"; fi ;;
  *"issue view 424242"*) echo "OPEN" ;;
  *) exit 0 ;;
esac
EOF
chmod +x "$STUB/gh"
run_main
t_assert_eq "crashed-with-armed-automerge cycle exits 0" "0" "$MAIN_RC"
t_assert_ok "adopted CI watch counts the landing as merged" \
  grep -q "merged: 1" "$T_TMP/main-out"
t_assert_ok "the driver actually polled the PR" grep -q "pr view 9999" "$GH_CALLS"
t_assert_ok "the watch used the poll cadence" grep -q "^60$" "$SLEEP_CALLS"
# A PR without a closing keyword merges without closing its issue. Stripping
# the claim label from a still-open issue would leave it with NO loop:*
# label at all — invisible to triage, untouchable by every future worker —
# so the post-mortem must close it, as the worker's own Step 9 would have.
t_assert_ok "adopted-but-unclosed issue is closed by the post-mortem" \
  grep -q "issue close 424242" "$GH_CALLS"
t_teardown

# (C) PENDING, never lands: the adopted watch must give up at the budget —
# the cycle counts failed (accurately: nothing merged) and the next worker's
# crash recovery can still adopt the issue.
t_setup; make_stub_env crash dry
cat > "$STUB/gh" <<'EOF'
#!/bin/bash
case "$*" in
  *"issue list"*"--state closed"*) : ;;
  *"issue list"*"--state open"*) echo "424242" ;;
  *"pr list"*"--state open"*)
    printf '[{"number":9999,"headRefName":"loop/424242-fix-slug","autoMergeRequest":{"enabledAt":"2026-01-01T00:00:00Z"}}]' ;;
  *"pr view 9999"*) echo "OPEN" ;;
  *) exit 0 ;;
esac
EOF
chmod +x "$STUB/gh"
run_main
t_assert_eq "unlanded adopted watch still exits 0 (single failure, then dry)" "0" "$MAIN_RC"
t_assert_ok "unlanded crash counted failed" grep -q "failed: 1" "$T_TMP/main-out"
# Pins the watch budget: exactly ADOPT_WAIT_SECS / ADOPT_POLL_SECS polls.
t_assert_eq "watch gave up after exactly the adopt budget (30 polls)" "30" \
  "$(grep -c '^60$' "$SLEEP_CALLS")"
t_teardown

# (D) An adopted crash RESETS the consecutive-failure breaker: an honest
# failure followed by a crashed-but-landed cycle is one failure + one merge,
# not the two-in-a-row that halts the run.
t_setup; make_stub_env failed crash dry
cat > "$STUB/gh" <<'EOF'
#!/bin/bash
case "$*" in
  *"issue list"*"--state closed"*) echo "424242" ;;
  *"pr list"*"--state merged"*)
    printf '[{"headRefName":"loop/424242-fix-slug","mergedAt":"2999-01-01T00:00:00Z"}]' ;;
  *) exit 0 ;;
esac
EOF
chmod +x "$STUB/gh"
run_main
t_assert_eq "adopted crash resets the consecutive-failure breaker" "0" "$MAIN_RC"
t_assert_ok "ledger shows the merge and the single failure" \
  grep -q "merged: 1, skipped: 0, failed: 1" "$T_TMP/main-out"
t_teardown

# (E) SMOKE guard: a smoke worker touches nothing adoptable by contract
# (SKILL.md smoke mode: "no labels, no issues, no branches"), so a smoke
# crash is a broken environment, full stop. The post-mortem must not scan
# GitHub — stale wreckage from a previous real run (exactly what #320
# leaves behind) could otherwise make a broken plumbing check exit 0.
t_setup; make_stub_env crash
cat > "$STUB/gh" <<'EOF'
#!/bin/bash
case "$*" in
  *"issue list"*"--state closed"*) echo "424242" ;;
  *"pr list"*"--state merged"*)
    printf '[{"headRefName":"loop/424242-fix-slug","mergedAt":"2999-01-01T00:00:00Z"}]' ;;
  *) exit 0 ;;
esac
EOF
chmod +x "$STUB/gh"
run_main --smoke
t_assert_eq "smoke crash never adopts — exits 1" "1" "$MAIN_RC"
t_assert_fail "smoke crash runs no post-mortem" \
  grep -q "post-mortem" "$T_TMP/main-out"
t_teardown

# (F) FORCED-ISSUE scope: with --issue N the driver knows exactly which
# issue the worker owned; wreckage of any OTHER issue must not be adopted —
# the single-shot exit code must reflect the forced issue's fate alone.
t_setup; make_stub_env crash
cat > "$STUB/gh" <<'EOF'
#!/bin/bash
case "$*" in
  *"issue list"*"--state closed"*) echo "424242" ;;
  *"pr list"*"--state merged"*)
    printf '[{"headRefName":"loop/424242-fix-slug","mergedAt":"2999-01-01T00:00:00Z"}]' ;;
  *) exit 0 ;;
esac
EOF
chmod +x "$STUB/gh"
run_main --issue 320
t_assert_eq "forced-issue crash ignores other issues' wreckage — exits 1" "1" "$MAIN_RC"
t_assert_fail "no cross-issue adoption in forced mode" \
  grep -q "adopted issue #424242" "$T_TMP/main-out"
t_teardown

# (G) Once only: an issue adopted by one iteration's post-mortem must never
# be adopted again by a later one — the same wreckage re-matched would
# double-count the merge and mask the later crash's real fate.
t_setup; make_stub_env crash crash dry
cat > "$STUB/gh" <<'EOF'
#!/bin/bash
case "$*" in
  *"issue list"*"--state closed"*) echo "424242" ;;
  *"pr list"*"--state merged"*)
    printf '[{"headRefName":"loop/424242-fix-slug","mergedAt":"2999-01-01T00:00:00Z"}]' ;;
  *) exit 0 ;;
esac
EOF
chmod +x "$STUB/gh"
run_main
t_assert_eq "re-offered wreckage run still exits 0 (dry after two crashes)" "0" "$MAIN_RC"
t_assert_ok "second crash counted failed, not merged again" \
  grep -q "merged: 1, skipped: 0, failed: 1" "$T_TMP/main-out"
t_teardown

# (H) A watch that gave up is not re-armed: the abandoned issue goes on the
# same once-only list, so a second crash fails FAST (no second 30-minute
# watch) and the two-failure breaker still fires.
t_setup; make_stub_env crash crash dry
cat > "$STUB/gh" <<'EOF'
#!/bin/bash
case "$*" in
  *"issue list"*"--state closed"*) : ;;
  *"issue list"*"--state open"*) echo "424242" ;;
  *"pr list"*"--state open"*)
    printf '[{"number":9999,"headRefName":"loop/424242-fix-slug","autoMergeRequest":{"enabledAt":"2026-01-01T00:00:00Z"}}]' ;;
  *"pr view 9999"*) echo "OPEN" ;;
  *) exit 0 ;;
esac
EOF
chmod +x "$STUB/gh"
run_main
t_assert_eq "two pending crashes halt the run" "1" "$MAIN_RC"
t_assert_eq "the second crash never re-armed the watch (still 30 polls)" "30" \
  "$(grep -c '^60$' "$SLEEP_CALLS")"
t_teardown

# ---- label taxonomy: agent-filed provenance + skill/driver drift ----
#
# Two independent silent failures are guarded here.
#
# (1) PROVENANCE. Both skills' author gate admits an issue whose author is
#     `hherb`, the operator. But a worker session runs under the operator's
#     OWN gh credentials, so every issue a worker files is authored by
#     `hherb` too and sails through that gate. That closes a loop with no
#     human in it: an agent files an issue, a later agent picks it up as
#     authoritative, works it, and merges it — green CI the only gate. So a
#     worker that misdiagnoses something can promote its own misdiagnosis
#     into main. `loop:agent-filed` is what breaks the loop, and a label the
#     driver never creates cannot break anything.
#
# (2) DRIFT. `gh issue edit --add-label X` FAILS when label X does not
#     exist, so a skill naming a label the driver never creates kills a
#     worker mid-cycle — after it has already claimed the issue. Assert the
#     labels the skills name are a subset of the labels the driver creates.
#     (A `loop:*` glob in prose yields no match: the pattern requires at
#     least one trailing character.)
t_setup
DRIVER_LABELS="$(declare -f setup_labels | grep -oE 'loop:[a-z-]+' | sort -u)"
t_assert_eq "setup_labels creates loop:agent-filed" "loop:agent-filed" \
  "$(printf '%s\n' "$DRIVER_LABELS" | grep -x 'loop:agent-filed' || true)"

SKILLS_DIR="$SCRIPT_DIR/../../.claude/skills"
SKILL_LABELS="$(cat "$SKILLS_DIR/techdebt-loop/SKILL.md" \
                    "$SKILLS_DIR/techdebt-next/SKILL.md" \
                | grep -oE 'loop:[a-z-]+' | sort -u)"
UNCREATED="$(comm -23 <(printf '%s\n' "$SKILL_LABELS") \
                      <(printf '%s\n' "$DRIVER_LABELS") | tr '\n' ' ')"
t_assert_eq "every loop:* label the skills name is created by the driver" "" \
  "$(printf '%s' "$UNCREATED" | sed 's/ *$//')"

# The gate is only real if the WORKER refuses to claim such an issue — triage
# is an LLM judgment call, the claim step is mechanical. Pin both skills'
# prose so the rule cannot be dropped by a later edit without a red test.
t_assert_ok "worker skill refuses to claim a loop:agent-filed issue" \
  grep -q "loop:agent-filed" "$SKILLS_DIR/techdebt-next/SKILL.md"
t_assert_ok "triage skill withholds eligibility from loop:agent-filed" \
  grep -q "loop:agent-filed" "$SKILLS_DIR/techdebt-loop/SKILL.md"
t_teardown

t_summary
