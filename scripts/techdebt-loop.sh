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
  "$@" &
  local cmd_pid=$!
  (
    sleep "$secs"
    kill -TERM "$cmd_pid" 2>/dev/null
    sleep 15
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
  if [ -n "$oldpid" ] && kill -0 "$oldpid" 2>/dev/null; then
    return 1  # a live loop holds the lock
  fi
  # Stale lock from a dead process: reclaim it.
  rm -rf "$lockdir"
  mkdir "$lockdir" 2>/dev/null && echo $$ > "$lockdir/pid"
}

release_lock() {
  rm -rf "$LOOP_HOME/lock"
}

# ---- test guard: when sourced by the test harness, stop here ----
if [ "${TECHDEBT_TEST:-0}" = "1" ]; then
  # shellcheck disable=SC2317 # reached when script is sourced in test mode
  return 0 2>/dev/null || true
fi

main "$@"
