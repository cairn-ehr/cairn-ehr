#!/bin/bash
# techdebt-loop.sh — driver for the tech-debt elimination loop.
#
# WHAT THIS IS: a deliberately dumb outer loop. It spawns one fresh headless
# Claude Code session per GitHub issue (`claude -p "/techdebt-next"`), then
# classifies how that session ended and decides: continue, wait for a usage-
# limit reset, or stop. All intelligence (picking issues, fixing, reviewing,
# merging) lives in the worker skill; all state lives in GitHub labels/PRs.
#
# Design: docs/superpowers/specs/2026-07-29-techdebt-loop-skill-design.md
# Compatible with macOS /bin/bash 3.2 — no bash-4 features.
#
# Usage: scripts/techdebt-loop.sh [--max-issues N] [--include-epics]
#          [--issue N] [--bypass] [--max-wait H] [--timeout H]
#          [--smoke] [--setup-labels]
set -u  # unset variables are bugs. NOT set -e: the loop's job is to
        # classify failures, not die on the first one.

# ---- configuration defaults (overridden by flags in parse_args) ----
LOOP_HOME="${TECHDEBT_LOOP_HOME:-$HOME/.cairn-loop}"  # logs, lock, outcomes
MAX_ISSUES=0        # stop after N merged/failed/skipped cycles; 0 = unlimited
MAX_WAIT_HOURS=6    # cap on cumulative rate-limit waiting; 0 = wait forever
TIMEOUT_HOURS=3     # per-iteration wall-clock cap for one worker session
INCLUDE_EPICS=0     # 1 = worker may also pick loop:epic issues
BYPASS=0            # 1 = --dangerously-skip-permissions instead of allowlist
FORCE_ISSUE=""      # non-empty = work exactly this issue number, then stop
SMOKE=0             # 1 = plumbing test: worker writes smoke-ok and exits
# All config vars above are used by parse_args (Task 2) and main loop (Tasks 3+);
# mark as used for shellcheck since this task defines them but doesn't use them.
: "${LOOP_HOME} ${MAX_ISSUES} ${MAX_WAIT_HOURS} ${TIMEOUT_HOURS} ${INCLUDE_EPICS} ${BYPASS} ${FORCE_ISSUE} ${SMOKE}"

# ---- small utilities ----

# log MSG — timestamped line to stdout (the driver's own log is its stdout;
# the entry skill redirects it to $LOOP_HOME/run.log when launching).
log() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

# die MSG — unrecoverable driver-level error (bad args, missing tools, lock).
die() {
  log "FATAL: $*"
  exit 3
}

# notify MSG — best-effort desktop notification; never fails the loop.
notify() {
  osascript -e "display notification \"$1\" with title \"techdebt-loop\"" \
    2>/dev/null || true
}

# require_cmds CMD... — fail fast if a tool the loop depends on is missing.
require_cmds() {
  local c
  for c in "$@"; do
    command -v "$c" >/dev/null 2>&1 || die "required command not found: $c"
  done
}

# ---- outcome classification (pure functions, unit-tested) ----

# parse_reset_epoch TRANSCRIPT — extract the unix epoch from the CLI's
# "Claude AI usage limit reached|<epoch>" error, if present. Prints nothing
# when absent; caller falls back to a fixed wait.
parse_reset_epoch() {
  sed -n 's/.*usage limit reached|\([0-9][0-9]*\).*/\1/p' "$1" 2>/dev/null | head -1
}

# is_rate_limited TRANSCRIPT — did this session die because the subscription
# usage window is exhausted? Matches both known CLI phrasings defensively.
is_rate_limited() {
  grep -qiE "usage limit reached\||hit your (session|usage|5-hour|weekly) limit" \
    "$1" 2>/dev/null
}

# classify_result OUTCOME_FILE TRANSCRIPT — the driver's one judgment call.
# Precedence: a worker that finished writes outcome.json (authoritative).
# A worker that died mid-cycle wrote nothing: a usage-limit pattern in the
# transcript means rate-limited (wait + resume); anything else is a failure
# (crash, timeout kill, tooling break).
classify_result() {
  local outcome_file="$1" transcript="$2" outcome=""
  if [ -s "$outcome_file" ]; then
    outcome="$(jq -r '.outcome // empty' "$outcome_file" 2>/dev/null)"
    if [ -n "$outcome" ]; then
      echo "$outcome"
      return 0
    fi
    # File exists but is not valid outcome JSON: treat as a crash.
    echo "failed"
    return 0
  fi
  if is_rate_limited "$transcript"; then
    echo "rate-limited"
  else
    echo "failed"
  fi
}

# compute_wait_secs EPOCH_OR_EMPTY NOW — how long to sleep before resuming
# after a rate limit. Known future reset: sleep to it + 5 min buffer.
# Unknown or already-past reset: conservative 30 min, then just try again.
compute_wait_secs() {
  local epoch="$1" now="$2"
  if [ -n "$epoch" ] && [ "$epoch" -gt "$now" ] 2>/dev/null; then
    echo $((epoch - now + 300))
  else
    echo 1800
  fi
}

# ---- process control ----

# run_with_timeout SECS CMD... — macOS has no GNU `timeout`, so we roll a
# portable one: run CMD in the background, start a watcher that TERMs (then
# KILLs) it after SECS, and normalize a timeout kill to exit code 124 so the
# caller can tell "took too long" from the command's own failures.
run_with_timeout() {
  local secs="$1"; shift
  local start
  start=$(date +%s)
  # Enable job control so the spawned command gets its own process group
  # (pgid == pid). This lets us kill the entire tree with a negative PID.
  set -m
  "$@" &
  local cmd_pid=$!
  set +m
  (
    sleep "$secs"
    # Kill the process group (-$cmd_pid), not just the direct child.
    # This ensures all descendants are reaped. Also kill the bare PID as a
    # fallback: if job control (set -m) somehow didn't give the child its
    # own process group, the group kill above would miss it entirely.
    kill -TERM -- -"$cmd_pid" 2>/dev/null
    kill -TERM "$cmd_pid" 2>/dev/null
    sleep 15
    kill -KILL -- -"$cmd_pid" 2>/dev/null
    kill -KILL "$cmd_pid" 2>/dev/null
  ) &
  local watch_pid=$!
  local rc=0
  wait "$cmd_pid" 2>/dev/null || rc=$?
  if kill -0 "$watch_pid" 2>/dev/null; then
    # Watcher still sleeping => the command finished on its own. Reap it.
    kill "$watch_pid" 2>/dev/null
    wait "$watch_pid" 2>/dev/null
  else
    rc=124
  fi
  # SIGTERM/SIGKILL exit statuses (143/137) also mean our watcher fired.
  if [ "$rc" = "143" ] || [ "$rc" = "137" ]; then
    rc=124
  fi
  # Elapsed time is the ground truth: if the child caught SIGTERM and
  # exited 0 anyway, elapsed time shows we timed out. Normalize to 124.
  local now
  now=$(date +%s)
  if [ "$rc" != "124" ] && [ $((now - start)) -ge "$secs" ]; then
    rc=124
  fi
  # Unconditionally reap the watcher to prevent zombies.
  wait "$watch_pid" 2>/dev/null || true
  return "$rc"
}

# ---- single-instance lock ----
# Two concurrent loops would race on labels and worktrees. `mkdir` is the
# portable atomic test-and-set; the PID file lets a later run distinguish
# "loop still running" from "loop crashed and left its lock behind".

acquire_lock() {
  local lockdir="$LOOP_HOME/lock"
  if mkdir "$lockdir" 2>/dev/null; then
    echo $$ > "$lockdir/pid"
    return 0
  fi
  local oldpid
  oldpid="$(cat "$lockdir/pid" 2>/dev/null || true)"
  # If the pid file is empty/missing, we hit a TOCTOU race: the lock
  # winner has created the dir but hasn't written its pid yet. Grace period:
  # sleep and re-read. A live winner writes its pid within microseconds;
  # a second is a generous grace.
  if [ -z "$oldpid" ]; then
    sleep 1
    oldpid="$(cat "$lockdir/pid" 2>/dev/null || true)"
  fi
  # Check if the oldpid is a live techdebt-loop process. `kill -0` can't
  # distinguish our loop from an unrelated process that recycled the PID,
  # so check the command line: only a process running techdebt-loop counts.
  if [ -n "$oldpid" ]; then
    local cmd
    cmd="$(ps -p "$oldpid" -o command= 2>/dev/null || true)"
    if [ -n "$cmd" ] && echo "$cmd" | grep -q "techdebt-loop"; then
      return 1  # a live loop holds the lock
    fi
  fi
  # Stale lock from a dead process or unrelated PID: reclaim it.
  rm -rf "$lockdir"
  mkdir "$lockdir" 2>/dev/null && echo $$ > "$lockdir/pid"
}

release_lock() {
  local lockdir="$LOOP_HOME/lock"
  local pidfile="$lockdir/pid"
  # Only remove the lock if we own it: the pid file is missing OR contains $$
  if [ -f "$pidfile" ]; then
    local stored_pid
    stored_pid="$(cat "$pidfile" 2>/dev/null || true)"
    if [ "$stored_pid" = "$$" ]; then
      rm -rf "$lockdir"
    fi
  else
    # Pid file missing: we own the lock (no one else holds it).
    rm -rf "$lockdir"
  fi
}

# ---- label taxonomy (spec §4) ----
# Idempotent: --force updates color/description on an existing label instead
# of failing, so every run may call this safely.
setup_labels() {
  gh label create "loop:ready"       --force --color "0E8A16" \
    --description "techdebt-loop: bounded, autonomously fixable, deps met"
  gh label create "loop:blocked"     --force --color "D93F0B" \
    --description "techdebt-loop: blocked; comment records what unblocks it"
  gh label create "loop:needs-human" --force --color "5319E7" \
    --description "techdebt-loop: needs human judgment"
  gh label create "loop:epic"        --force --color "1D76DB" \
    --description "techdebt-loop: multi-PR slice (only with --include-epics)"
  gh label create "loop:in-progress" --force --color "FBCA04" \
    --description "techdebt-loop: a worker session owns this issue"
  gh label create "loop:retry"       --force --color "F9D0C4" \
    --description "techdebt-loop: first cycle failed; one retry allowed"
  gh label create "loop:failed"      --force --color "B60205" \
    --description "techdebt-loop: parked after second failure; human triage"
}

# ---- argument parsing ----
parse_args() {
  SETUP_LABELS_ONLY=0
  while [ $# -gt 0 ]; do
    case "$1" in
      --max-issues)    MAX_ISSUES="$2"; shift 2 ;;
      --max-wait)      MAX_WAIT_HOURS="$2"; shift 2 ;;
      --timeout)       TIMEOUT_HOURS="$2"; shift 2 ;;
      --include-epics) INCLUDE_EPICS=1; shift ;;
      --bypass)        BYPASS=1; shift ;;
      --issue)         FORCE_ISSUE="$2"; shift 2 ;;
      --smoke)         SMOKE=1; shift ;;
      --setup-labels)  SETUP_LABELS_ONLY=1; shift ;;
      *) die "unknown flag: $1" ;;
    esac
  done
}

# spawn_worker ITER — run one fresh worker session. The env vars are the
# entire driver->worker contract; the transcript captures everything the
# session printed (the forensic record when no outcome.json appears).
spawn_worker() {
  local iter="$1"
  OUTCOME_FILE="$RUN_DIR/outcome-$iter.json"
  TRANSCRIPT="$RUN_DIR/iter-$iter.log"
  rm -f "$OUTCOME_FILE"
  local perm_args="--permission-mode acceptEdits"
  if [ "$BYPASS" = "1" ]; then
    perm_args="--dangerously-skip-permissions"
  fi
  # shellcheck disable=SC2086  # perm_args is deliberately word-split
  run_with_timeout "$((TIMEOUT_HOURS * 3600))" \
    env TECHDEBT_OUTCOME_FILE="$OUTCOME_FILE" \
        TECHDEBT_INCLUDE_EPICS="$INCLUDE_EPICS" \
        TECHDEBT_FORCE_ISSUE="$FORCE_ISSUE" \
        TECHDEBT_SMOKE="$SMOKE" \
    claude -p "/techdebt-next" $perm_args \
    > "$TRANSCRIPT" 2>&1
}

# summarize — one place that prints the run's ledger; called on every exit.
summarize() {
  log "run summary — merged: $N_MERGED, skipped: $N_SKIPPED, failed: $N_FAILED, iterations: $ITER"
  notify "techdebt-loop done: $N_MERGED merged, $N_FAILED failed"
}

main() {
  parse_args "$@"
  require_cmds claude gh jq git
  # Always operate from the repo root, wherever the script was called from.
  cd "$(cd "$(dirname "$0")/.." && pwd)" || die "cannot cd to repo root"
  mkdir -p "$LOOP_HOME"
  if [ "$SETUP_LABELS_ONLY" = "1" ]; then
    setup_labels
    exit 0
  fi
  acquire_lock || die "another techdebt-loop is already running"
  trap release_lock EXIT
  # TERM/INT must actually terminate the loop; `exit` here fires the EXIT
  # trap, which releases the lock exactly once. Without these, bash runs the
  # handler and RESUMES the loop — `kill` would release the lock while the
  # driver keeps spawning workers (double-launch hazard).
  trap 'exit 130' INT
  trap 'exit 143' TERM
  RUN_DIR="$LOOP_HOME/run-$(date '+%Y%m%d-%H%M%S')"
  mkdir -p "$RUN_DIR"
  log "run dir: $RUN_DIR"

  ITER=0; N_MERGED=0; N_FAILED=0; N_SKIPPED=0
  CONSEC_FAIL=0
  WAITED_TOTAL=0
  local max_wait_secs=$((MAX_WAIT_HOURS * 3600))

  while :; do
    # --max-issues counts completed issue attempts (merged+failed+skipped).
    if [ "$MAX_ISSUES" -gt 0 ] && \
       [ $((N_MERGED + N_FAILED + N_SKIPPED)) -ge "$MAX_ISSUES" ]; then
      log "--max-issues $MAX_ISSUES reached"
      summarize; exit 0
    fi
    ITER=$((ITER + 1))
    log "iteration $ITER: spawning worker"
    spawn_worker "$ITER" || true   # exit code is informational; outcome rules
    OUTCOME="$(classify_result "$OUTCOME_FILE" "$TRANSCRIPT")"
    log "iteration $ITER: outcome=$OUTCOME"
    case "$OUTCOME" in
      merged)
        N_MERGED=$((N_MERGED + 1)); CONSEC_FAIL=0 ;;
      skipped)
        N_SKIPPED=$((N_SKIPPED + 1)); CONSEC_FAIL=0 ;;
      smoke-ok)
        log "smoke test passed: skill invocation + outcome plumbing verified"
        summarize; exit 0 ;;
      dry)
        log "backlog dry: no loop:ready issues remain"
        summarize; exit 0 ;;
      failed-permission)
        log "PERMISSION DENIED — extend the allowlist and re-run:"
        jq -r '.detail // "unknown command"' "$OUTCOME_FILE" 2>/dev/null | \
          while IFS= read -r line; do log "  $line"; done
        summarize; exit 2 ;;
      rate-limited)
        local epoch wait_secs
        epoch="$(parse_reset_epoch "$TRANSCRIPT")"
        wait_secs="$(compute_wait_secs "$epoch" "$(date +%s)")"
        WAITED_TOTAL=$((WAITED_TOTAL + wait_secs))
        if [ "$max_wait_secs" -gt 0 ] && [ "$WAITED_TOTAL" -gt "$max_wait_secs" ]; then
          log "cumulative wait would exceed --max-wait ${MAX_WAIT_HOURS}h; stopping"
          summarize; exit 4
        fi
        log "usage limit hit; sleeping ${wait_secs}s (issue unpenalized)"
        sleep "$wait_secs"
        continue ;;   # not an issue attempt: counters untouched
      failed|*)
        N_FAILED=$((N_FAILED + 1)); CONSEC_FAIL=$((CONSEC_FAIL + 1))
        log "cycle failed (consecutive: $CONSEC_FAIL) — see $TRANSCRIPT"
        if [ "$CONSEC_FAIL" -ge 2 ]; then
          log "two consecutive failures: something systemic is wrong; stopping"
          summarize; exit 1
        fi ;;
    esac
    if [ -n "$FORCE_ISSUE" ] || [ "$SMOKE" = "1" ]; then
      # Single-shot modes: one real attempt, then stop. A scripting caller
      # can only tell "issue fixed" from "issue failed" apart via the exit
      # code, so it must reflect this attempt's outcome, not a blanket 0.
      # (smoke-ok/dry/failed-permission already exited in their own arms
      # above; rate-limited already `continue`d, so single-shot mode still
      # waits-and-retries on a rate limit instead of landing here.)
      if [ "$OUTCOME" = "failed" ]; then
        summarize; exit 1
      fi
      summarize; exit 0
    fi
  done
}

# ---- test guard: when sourced by the test harness, stop here ----
if [ "${TECHDEBT_TEST:-0}" = "1" ]; then
  # shellcheck disable=SC2317 # reached when script is sourced in test mode
  return 0 2>/dev/null || true
fi

main "$@"
