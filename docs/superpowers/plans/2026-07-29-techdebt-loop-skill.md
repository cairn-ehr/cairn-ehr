# Tech-Debt Elimination Loop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the two skills (`/techdebt-loop`, `/techdebt-next`) and the bash driver that drain the GitHub backlog of bounded tech-debt issues one fresh headless session at a time, per the approved spec `docs/superpowers/specs/2026-07-29-techdebt-loop-skill-design.md`.

**Architecture:** A deliberately dumb bash driver (`scripts/techdebt-loop.sh`) spawns one fresh `claude -p "/techdebt-next"` session per issue and classifies how each session ended (`outcome.json` from the worker, or transcript forensics for rate-limit/crash). All loop state lives in GitHub labels/comments/PRs. The worker skill does one issue through plan→TDD→review→PR→second review→docs→merge. The entry skill triages the backlog and launches the driver.

**Tech Stack:** bash 3.2 (macOS `/bin/bash`), `gh`, `jq`, `claude` CLI 2.1.220, shellcheck; skill files are Claude Code project skills (`.claude/skills/*/SKILL.md`).

**Hard dependency check before Task 1:** `command -v jq shellcheck gh claude` — all four must resolve (`brew install jq shellcheck` if missing). The driver `require_cmds`-gates on `claude gh jq git` at runtime.

## Global Constraints

- **bash 3.2 compatible** — no associative arrays, no `${var,,}`, no `mapfile`; shebang `#!/bin/bash`. macOS has no GNU `timeout`; use the plan's `run_with_timeout`.
- **shellcheck-clean** — `shellcheck scripts/techdebt-loop.sh scripts/tests/techdebt-loop-test.sh` must pass with zero warnings before every commit of those files.
- **GitHub is the only state store** (spec §3). No loop state may live in files that survive a session, except run logs/outcomes under `~/.cairn-loop/` which are diagnostics + the driver↔worker handoff, never authority.
- **Worker must stay standalone-invocable**: `/techdebt-next` in an interactive session with no `TECHDEBT_*` env vars set must work (it skips the `outcome.json` write).
- **Merge-commit convention**: `gh pr merge --auto --merge`, never squash/rebase.
- **Driver is sourceable for tests**: it ends with `if [ "${TECHDEBT_TEST:-0}" != "1" ]; then main "$@"; fi`. Tests set `TECHDEBT_TEST=1` and source it.
- **TDD for the driver** (house rule 2): every driver behavior lands test-first. Skill `.md` files are prompt-code — their gate is the smoke test (Task 6) plus review, not unit tests.
- **House rule 3**: every function in the driver carries a junior-legible comment: why it exists, how it fits.
- All commits on the current branch `claude/tech-debt-elimination-skill-132fc7`; commit messages end with the Claude co-author trailer.

## File Structure

| File | Responsibility |
|---|---|
| `scripts/techdebt-loop.sh` | Driver: arg parsing, lock, label setup, per-iteration spawn + timeout, outcome classification, failure/wait accounting, summary. Sourceable; all logic in functions. |
| `scripts/tests/techdebt-loop-test.sh` | Test harness + all driver tests. Stubs `claude`/`gh`/`sleep`/`osascript` via a PATH shim dir. |
| `.claude/skills/techdebt-next/SKILL.md` | Worker skill: one issue, full cycle (spec §7). |
| `.claude/skills/techdebt-loop/SKILL.md` | Entry skill: preflight, triage (spec §5), driver launch. |
| `.claude/settings.json` | Project permission allowlist seed + `additionalDirectories` for `~/.cairn-loop` (spec §8). |
| `docs/HANDOVER.md` | One-line currency note (bundled in this PR, per convention). |

**The driver↔worker contract** (used by Tasks 4–6):

- Driver exports per iteration: `TECHDEBT_OUTCOME_FILE` (absolute path the worker writes), `TECHDEBT_INCLUDE_EPICS` (`0`/`1`), `TECHDEBT_FORCE_ISSUE` (issue number or empty), `TECHDEBT_SMOKE` (`0`/`1`).
- Worker writes `outcome.json` exactly once, as its **last file write** before ending:

```json
{"outcome": "merged", "issue": 227, "pr": 301, "step": "cleanup", "detail": ""}
```

  `outcome` ∈ `merged | skipped | dry | failed | failed-permission | smoke-ok`; `issue`/`pr` are numbers or `null`; `step` is the §7 step name reached; `detail` carries the error text (for `failed-permission`: the exact denied command).
- Driver classification precedence: non-empty `outcome.json` wins; else transcript matching a usage-limit pattern → `rate-limited`; else `failed`.

---

### Task 1: Driver skeleton, test harness, and pure classification functions

**Files:**
- Create: `scripts/techdebt-loop.sh`
- Create: `scripts/tests/techdebt-loop-test.sh`

**Interfaces:**
- Produces: `parse_reset_epoch TRANSCRIPT_FILE` (prints epoch or nothing), `is_rate_limited TRANSCRIPT_FILE` (exit 0/1), `classify_result OUTCOME_FILE TRANSCRIPT_FILE` (prints outcome token), `compute_wait_secs EPOCH_OR_EMPTY NOW` (prints seconds), `log MSG`, `die MSG` (exit 3), `notify MSG` (best-effort macOS notification), `require_cmds CMD...`. Test harness: `t_setup`, `t_assert_eq DESC EXPECTED ACTUAL`, `t_assert_ok DESC CMD...`, `t_assert_fail DESC CMD...`, `t_summary`.

- [ ] **Step 1: Write the failing tests**

Create `scripts/tests/techdebt-loop-test.sh`:

```bash
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
TECHDEBT_TEST=1
# shellcheck source=../techdebt-loop.sh
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

t_summary
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `bash scripts/tests/techdebt-loop-test.sh`
Expected: FAIL — sourcing errors out because `scripts/techdebt-loop.sh` does not exist yet.

- [ ] **Step 3: Write the driver skeleton with the pure functions**

Create `scripts/techdebt-loop.sh`:

```bash
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

# ---- test guard: when sourced by the test harness, stop here ----
if [ "${TECHDEBT_TEST:-0}" = "1" ]; then
  return 0 2>/dev/null || true
fi

main "$@"
```

Note the guard placement: functions added by later tasks go **above** the guard; `main` is defined in Task 4 (until then, direct execution fails with "main: command not found" — acceptable mid-build).

- [ ] **Step 4: Run tests to verify they pass**

Run: `bash scripts/tests/techdebt-loop-test.sh`
Expected: all PASS, `0 failed`.

- [ ] **Step 5: Shellcheck**

Run: `shellcheck scripts/techdebt-loop.sh scripts/tests/techdebt-loop-test.sh`
Expected: no output. Fix any finding before committing (suppress nothing without a comment saying why).

- [ ] **Step 6: Commit**

```bash
git add scripts/techdebt-loop.sh scripts/tests/techdebt-loop-test.sh
git commit -m "feat(techdebt-loop): driver skeleton + outcome classification (TDD)"
```

---

### Task 2: Portable timeout and the lockfile

**Files:**
- Modify: `scripts/techdebt-loop.sh` (add functions above the test guard)
- Modify: `scripts/tests/techdebt-loop-test.sh` (append tests before `t_summary`)

**Interfaces:**
- Consumes: `t_setup`/`t_assert_*` from Task 1.
- Produces: `run_with_timeout SECS CMD...` (returns the command's exit code, or 124 if killed on timeout), `acquire_lock` (exit 0 = lock held; 1 = another live loop), `release_lock`. Lock lives at `$LOOP_HOME/lock/` (atomic `mkdir`) with the holder's PID in `lock/pid`; a lock whose PID is dead is stale and reclaimed.

- [ ] **Step 1: Write the failing tests**

Append to `scripts/tests/techdebt-loop-test.sh` (before `t_summary`):

```bash
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
```

- [ ] **Step 2: Run tests to verify the new ones fail**

Run: `bash scripts/tests/techdebt-loop-test.sh`
Expected: Task 1 tests PASS; new tests FAIL with "run_with_timeout: command not found".

- [ ] **Step 3: Implement**

Add to `scripts/techdebt-loop.sh`, after `compute_wait_secs`, above the test guard:

```bash
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `bash scripts/tests/techdebt-loop-test.sh`
Expected: all PASS. The timeout test adds ~1–2 s runtime; that is fine.

- [ ] **Step 5: Shellcheck, then commit**

```bash
shellcheck scripts/techdebt-loop.sh scripts/tests/techdebt-loop-test.sh
git add scripts/techdebt-loop.sh scripts/tests/techdebt-loop-test.sh
git commit -m "feat(techdebt-loop): portable timeout + stale-safe lockfile (TDD)"
```

---

### Task 3: Label setup subcommand

**Files:**
- Modify: `scripts/techdebt-loop.sh`
- Modify: `scripts/tests/techdebt-loop-test.sh`

**Interfaces:**
- Produces: `setup_labels` — idempotently creates/updates the seven `loop:*` labels via `gh label create --force`. Invoked by the entry skill as `scripts/techdebt-loop.sh --setup-labels` (flag wiring lands in Task 4; the function is testable now).

- [ ] **Step 1: Write the failing test**

Append before `t_summary`. The stub technique used here (a fake `gh` on PATH that records its argv) is reused for `claude` in Task 4:

```bash
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
```

- [ ] **Step 2: Run tests to verify the new ones fail**

Run: `bash scripts/tests/techdebt-loop-test.sh`
Expected: new tests FAIL ("setup_labels: command not found").

- [ ] **Step 3: Implement**

Add above the test guard:

```bash
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
```

- [ ] **Step 4: Run tests, shellcheck, commit**

```bash
bash scripts/tests/techdebt-loop-test.sh
shellcheck scripts/techdebt-loop.sh scripts/tests/techdebt-loop-test.sh
git add scripts/techdebt-loop.sh scripts/tests/techdebt-loop-test.sh
git commit -m "feat(techdebt-loop): idempotent loop:* label setup (TDD)"
```

---

### Task 4: Driver main loop

**Files:**
- Modify: `scripts/techdebt-loop.sh`
- Modify: `scripts/tests/techdebt-loop-test.sh`

**Interfaces:**
- Consumes: everything from Tasks 1–3.
- Produces: `parse_args "$@"`, `spawn_worker ITER` (one iteration: env + claude + timeout + transcript), `main "$@"`. Exit codes: `0` backlog dry / max-issues reached / smoke ok; `1` two consecutive failures; `2` permission denial; `3` driver-level fatal (`die`); `4` max-wait exceeded.
- The `claude` invocation (exact): `claude -p "/techdebt-next" --permission-mode acceptEdits` — or with `--bypass`: `claude -p "/techdebt-next" --dangerously-skip-permissions`.

- [ ] **Step 1: Write the failing scenario tests**

The fake `claude` reads a scenario file: line N tells call N how to behave. Append before `t_summary`:

```bash
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
  export STUB_COUNTER="$COUNTER" STUB_SCENARIO="$SCENARIO" SLEEP_CALLS
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
```

- [ ] **Step 2: Run tests to verify the new ones fail**

Run: `bash scripts/tests/techdebt-loop-test.sh`
Expected: new scenario tests FAIL ("main: command not found" or nonzero MAIN_RC mismatches).

- [ ] **Step 3: Implement `parse_args`, `spawn_worker`, `main`**

Add above the test guard:

```bash
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
  trap release_lock EXIT INT TERM
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
      # Single-shot modes: one real attempt, then stop.
      summarize; exit 0
    fi
  done
}
```

Then move the existing `main "$@"` line to stay as the last line (after the test guard), and delete nothing else. Note `local` inside `main`'s `case` is invalid outside a function only — it is inside `main`, fine; but bash 3.2 disallows `local` in a `case` arm only if outside a function, so this is valid. If shellcheck flags `local` placement, hoist `epoch wait_secs` declarations to the top of `main`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `bash scripts/tests/techdebt-loop-test.sh`
Expected: all PASS. Debug any scenario mismatch by reading `$T_TMP/main-out` (add a temporary `cat`).

- [ ] **Step 5: Shellcheck, then commit**

```bash
shellcheck scripts/techdebt-loop.sh scripts/tests/techdebt-loop-test.sh
git add scripts/techdebt-loop.sh scripts/tests/techdebt-loop-test.sh
git commit -m "feat(techdebt-loop): driver main loop — spawn, classify, valves, rate-limit resume (TDD)"
```

---

### Task 5: Worker skill `/techdebt-next`

**Files:**
- Create: `.claude/skills/techdebt-next/SKILL.md`

**Interfaces:**
- Consumes: env contract from the File Structure section (`TECHDEBT_OUTCOME_FILE`, `TECHDEBT_INCLUDE_EPICS`, `TECHDEBT_FORCE_ISSUE`, `TECHDEBT_SMOKE`); the `loop:*` labels from Task 3.
- Produces: the worker behavior the driver classifies. This file is prompt-code: its correctness gate is Task 6's smoke test + the supervised first run.

- [ ] **Step 1: Write the skill file**

Create `.claude/skills/techdebt-next/SKILL.md` with exactly this content:

````markdown
---
name: techdebt-next
description: Work exactly one loop:ready GitHub issue through the full quality cycle (plan, TDD, review, PR, second review, docs, merge), then stop. Designed to run in a fresh headless session spawned by scripts/techdebt-loop.sh, but safe to invoke standalone in an interactive session.
---

# techdebt-next — one issue, full cycle

You do EXACTLY ONE issue this session, then end your turn. Never start a
second issue. A fresh session handles the next one — that is the design
(fresh context per issue). Design doc:
`docs/superpowers/specs/2026-07-29-techdebt-loop-skill-design.md`.

## Outcome protocol (read first)

If the environment variable `TECHDEBT_OUTCOME_FILE` is set, your LAST file
write before ending the session MUST be that file, containing exactly one
JSON object:

```json
{"outcome": "<token>", "issue": <n|null>, "pr": <n|null>, "step": "<step>", "detail": "<text>"}
```

Outcome tokens: `merged` (cycle completed), `skipped` (relabeled without
attempting), `dry` (no eligible issue), `failed` (cycle failed),
`failed-permission` (a tool call was denied by permissions — put the exact
denied command in `detail`), `smoke-ok` (smoke mode only).
If `TECHDEBT_OUTCOME_FILE` is unset (standalone use), skip all outcome
writes and just report to the user in prose.

**Permission denials:** if any tool call fails because permission was
denied, do NOT improvise around it with different commands. Roll back
nothing, write outcome `failed-permission` with the denied command in
`detail`, and end your turn. The operator extends the allowlist and
re-runs; the issue's labels must be left exactly as you found them.

## Smoke mode

If `TECHDEBT_SMOKE=1`: verify you can run `git status` and `gh auth status`
via Bash, then write outcome `smoke-ok` (issue/pr null, step "smoke") and
end your turn. Touch nothing else — no labels, no issues, no branches.

## Step 0 — preflight and crash recovery

1. `git fetch origin main`.
2. `gh issue list --label "loop:in-progress" --state open --json number`
   — a non-empty result means a previous worker died mid-cycle. ADOPT the
   lowest-numbered one instead of picking fresh: reconstruct its position
   from GitHub state and resume from there:
   - PR exists and is MERGED → resume at Step 9 (cleanup).
   - PR exists, checks failing → resume at Step 8's red-CI arm.
   - PR exists, checks green/pending, second review not yet posted (no PR
     review from this loop visible) → resume at Step 7.
   - Branch `loop/<n>-*` exists but no PR → delete the remote branch if
     pushed, remove any local worktree at `~/.cairn-loop/wt/issue-<n>`,
     and restart the cycle at Step 2 (the plan comment already posted
     still stands; do not duplicate it if present).
   - No branch → restart at Step 1's labeling (already done) then Step 2.

## Step 1 — pick

- If `TECHDEBT_FORCE_ISSUE` is set and non-empty, that is your issue
  (verify it is open; if not, write outcome `dry` and stop).
- Else: lowest-numbered open issue labeled `loop:ready`. If none and
  `TECHDEBT_INCLUDE_EPICS=1`, lowest open `loop:epic`. If none at all —
  check `loop:retry`: lowest open `loop:retry` is eligible ONCE more.
  If still none, write outcome `dry` and stop.
- Claim it: `gh issue edit <n> --add-label "loop:in-progress" --remove-label "loop:ready"`
  (or `--remove-label "loop:retry"` / `"loop:epic"` as appropriate), then
  comment: `techdebt-loop: cycle started (session <today's date>).`

**Scope re-check:** read the issue fully. If the fix genuinely needs
multiple PRs or an architecture decision (new ADR / spec change), do NOT
attempt it: relabel (`--add-label "loop:epic"` or `"loop:needs-human"`,
`--remove-label "loop:in-progress"`), comment one paragraph explaining why,
write outcome `skipped`, stop.

## Step 2 — worktree

```bash
git worktree add ~/.cairn-loop/wt/issue-<n> -b loop/<n>-<short-slug> origin/main
cd ~/.cairn-loop/wt/issue-<n>
```

## Step 3 — plan

Post a brief implementation plan as an issue comment (what you'll change,
which files, how you'll test it). Include the paper-parity line per house
rule 7: for non-clinical-surface work a single line
`Paper-parity: not clinical-surface — <reason>`; if the issue DOES change a
clinical workflow, a real `## Paper-parity benchmark (§1.2)` section.

## Step 4 — TDD implement

Failing test first, then the minimal fix (use superpowers:test-driven-development).
Follow CLAUDE.md house rules: junior-legible comments, pure functions over
cleverness, no hard-coded crypto material in tests.

## Step 5 — local gate (all must pass before a PR)

- `cargo test` — FULL workspace from the worktree root. Never `-p`, never
  piped through `tail` (masks the exit code).
- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets`.
- If anything under `db/` changed: `scripts/run-db-sql-tests.sh`.
- If `db/` added columns to `event_log`: recreate cairn_test/2/3 on :5532
  first (stale positional ROW literals otherwise fail born_sealed tests).
- Python touched: `uv run pytest` in the affected directory.

## Step 6 — pre-PR review

Dispatch the `pr-review-toolkit:code-reviewer` agent on your working diff
(`git diff origin/main...HEAD`). Fix every finding it confirms; re-run the
local gate after fixes.

## Step 7 — PR + second full review

1. Push and open the PR: title `fix(#<n>): <summary>`; body explains the
   change for a reviewer with zero context and contains `Fixes #<n>`.
2. Then run the second, full PR review — fresh eyes on the COMPLETE PR:
   invoke the `code-review:code-review` skill on this PR number. This is
   not redundant with Step 6 (it sees the whole PR: diff + tests +
   description) and empirically still finds real issues.
3. Every finding: fix it and push, or — only if genuinely out of scope —
   file a follow-up GitHub issue capturing it (house rule 5). Never drop a
   finding silently. Re-run the local gate if code changed.

## Step 8 — docs, then merge

1. If this change materially alters build state, update `docs/HANDOVER.md`
   / `ROADMAP.md` in the SAME PR (keep both under 500 lines). Skip when
   nothing material changed — most tech debt.
2. `gh pr merge <pr> --auto --merge` (merge commit; --auto waits for the
   5 required checks).
3. Poll every 2 minutes (max 40 min):
   `gh pr view <pr> --json state,statusCheckRollup`.
   - MERGED → Step 9.
   - Any required check failed → diagnose and fix ONCE (push the fix,
     `gh pr merge --auto --merge` again, resume polling). A second red
     round → treat as cycle failure (below).
   - 40 min without resolution → cycle failure.

## Step 9 — cleanup and report

```bash
cd <repo root>
git worktree remove ~/.cairn-loop/wt/issue-<n> --force
git branch -D loop/<n>-<short-slug> 2>/dev/null || true
git fetch origin main
```

Verify the issue auto-closed (`gh issue view <n> --json state`); if not,
close it with a comment linking the PR. Remove `loop:in-progress`. Write
outcome `merged` (step "cleanup", pr number filled in). End your turn.

## Cycle failure (any step irrecoverable)

1. Comment on the issue: which step, what failed (include the actual error
   text), what you tried.
2. Labels: remove `loop:in-progress`; add `loop:retry` if this was the
   issue's FIRST failed cycle (no prior failure comment from the loop),
   else `loop:failed`.
3. If a PR is open: `gh pr ready <pr> --undo` (convert to draft).
4. Remove the worktree (as in Step 9).
5. Write outcome `failed` (step = where it died, detail = one-line error).
   End your turn.

## Hard rules

- ONE issue per session. Ending early with an honest `failed` outcome beats
  a heroic multi-issue session.
- Never `git push --force`, never rewrite main, never merge a red PR.
- Never touch issues without a `loop:*` label.
- The outcome file write is always your last file action.
````

- [ ] **Step 2: Review the skill against spec §7**

Read the spec's §7 side by side with the file; every numbered spec step must appear. Verify the frontmatter parses (no tabs, valid YAML).

- [ ] **Step 3: Commit**

```bash
git add .claude/skills/techdebt-next/SKILL.md
git commit -m "feat(techdebt-loop): /techdebt-next worker skill — one issue, full cycle"
```

---

### Task 6: Settings allowlist + end-to-end smoke test

**Files:**
- Create: `.claude/settings.json`

**Interfaces:**
- Consumes: driver `--smoke` (Task 4), worker smoke mode (Task 5).
- Produces: the permission seed that headless workers run under; a verified end-to-end plumbing path (`driver → claude -p → skill → outcome.json → driver classification`).

- [ ] **Step 1: Create the allowlist seed**

Create `.claude/settings.json` (no project settings file exists yet — creating, not merging):

```json
{
  "permissions": {
    "allow": [
      "Bash(git:*)",
      "Bash(gh:*)",
      "Bash(cargo:*)",
      "Bash(uv:*)",
      "Bash(jq:*)",
      "Bash(shellcheck:*)",
      "Bash(psql:*)",
      "Bash(scripts/run-db-sql-tests.sh:*)",
      "Bash(scripts/techdebt-loop.sh:*)",
      "Bash(bash scripts/tests/techdebt-loop-test.sh:*)"
    ],
    "additionalDirectories": [
      "~/.cairn-loop"
    ]
  }
}
```

`additionalDirectories` is what lets an `acceptEdits` worker write `outcome.json` under `~/.cairn-loop/` without a prompt. Tilde expansion in settings paths is not guaranteed on every CLI version — if Step 2's smoke test fails on the outcome write, replace `"~/.cairn-loop"` with the absolute `"/Users/hherb/.cairn-loop"` and re-run. The list will grow empirically: every `failed-permission` stop names the missing pattern.

- [ ] **Step 2: Run the end-to-end smoke test**

```bash
scripts/techdebt-loop.sh --smoke
```

Expected: exit 0; log line `smoke test passed`; a `run-*/outcome-1.json` under `~/.cairn-loop/` containing `"outcome": "smoke-ok"`. This proves: skill discovery in `-p` mode, env passthrough, allowlist sufficiency for `git status`/`gh auth status`, the outcome write path, and driver classification — before any issue is ever touched.

If it fails, read `~/.cairn-loop/run-*/iter-1.log`: a permission denial means the allowlist or `additionalDirectories` needs adjusting; "Unknown skill" means skill discovery failed (verify the file path and frontmatter).

- [ ] **Step 3: Commit**

```bash
git add .claude/settings.json
git commit -m "feat(techdebt-loop): permission allowlist seed; end-to-end smoke verified"
```

---

### Task 7: Entry skill `/techdebt-loop`

**Files:**
- Create: `.claude/skills/techdebt-loop/SKILL.md`

**Interfaces:**
- Consumes: `scripts/techdebt-loop.sh` flags (Task 4), labels (Task 3).
- Produces: the user-facing entry point.

- [ ] **Step 1: Write the skill file**

Create `.claude/skills/techdebt-loop/SKILL.md` with exactly this content:

````markdown
---
name: techdebt-loop
description: Triage all open GitHub issues into loop:* labels, then launch the tech-debt elimination driver that works loop:ready issues one fresh session at a time until the backlog is dry. Args are passed to the driver (--dry-run stops after triage; --max-issues N, --include-epics, --issue N, --bypass, --max-wait H, --timeout H).
---

# techdebt-loop — triage the backlog, then launch the driver

Design doc: `docs/superpowers/specs/2026-07-29-techdebt-loop-skill-design.md`.
Arguments you receive are driver flags, except `--dry-run` which you handle
yourself (triage only, no launch).

## 1. Preflight

- `git fetch origin main` and confirm `gh auth status` succeeds.
- `scripts/techdebt-loop.sh --setup-labels` (idempotent).
- If `~/.cairn-loop/lock` exists with a live PID, tell the user a loop is
  already running and stop.

## 2. Triage (spec §5)

Collect issues to classify: every OPEN issue with NO `loop:*` label, plus
every `loop:blocked` issue (re-check whether its blocker has cleared).

Classify in parallel: dispatch Explore subagents in batches of ~8 issues.
Each subagent gets, per issue: number, title, body, comments (via
`gh issue view <n> --comments`), and returns for each a classification +
one-sentence justification:

- **ready** — bounded (one PR), autonomously fixable from the repo alone,
  all dependencies merged, no hardware/external requirement, no new ADR or
  spec change needed.
- **blocked** — depends on an unmerged issue/PR (name it), an upstream
  release (name it), or hardware access (name it).
- **needs-human** — requires a design decision, an ADR, a spec change, or
  the user's clinical judgment; also anything explicitly labeled a design
  session.
- **epic** — legitimate work but multiple PRs / a feature slice.

The MAIN session (you) applies the results — subagents must not write:

```bash
gh issue edit <n> --add-label "loop:ready"          # ready needs no comment
gh issue edit <n> --add-label "loop:blocked"        # + justification comment
gh issue comment <n> --body "techdebt-loop triage: blocked — <justification>. Re-checked each run."
```

(same pattern for needs-human / epic; a re-checked `loop:blocked` issue
whose blocker cleared: swap the label to `loop:ready` and comment that it
unblocked). Already-correct labels are left untouched.

## 3. Report

Print a table: issue number, title (truncated), classification, one-line
reason — grouped ready / blocked / needs-human / epic. State the counts.
If `--dry-run` was passed: STOP HERE and tell the user to re-invoke without
`--dry-run` when the classification looks right.

## 4. Launch

Launch the driver detached so it survives this session ending, passing
through every flag you received except `--dry-run`:

```bash
mkdir -p ~/.cairn-loop
nohup scripts/techdebt-loop.sh <flags> >> ~/.cairn-loop/run.log 2>&1 &
disown
```

Then tell the user:
- how to watch: `tail -f ~/.cairn-loop/run.log` (driver log) and the
  per-iteration transcripts under `~/.cairn-loop/run-<timestamp>/`;
- how to stop: `kill <pid>` (report the PID) — the in-flight worker
  session finishes or times out; state is safe in GitHub either way;
- that re-running `/techdebt-loop` after ANY interruption is always safe
  (crash recovery adopts in-flight issues).

For a FIRST-EVER run, recommend the user instead do: `--dry-run` first,
then `--max-issues 1` watched via the log, before an unbounded run.
````

- [ ] **Step 2: Review against spec §5–6, commit**

Check: triage categories match spec §4; the launch honors every driver flag; `--dry-run` stops before launch.

```bash
git add .claude/skills/techdebt-loop/SKILL.md
git commit -m "feat(techdebt-loop): /techdebt-loop entry skill — triage + detached launch"
```

---

### Task 8: Final verification, docs currency, PR

**Files:**
- Modify: `docs/HANDOVER.md` (one bullet in the current-state section)

- [ ] **Step 1: Full gate**

```bash
bash scripts/tests/techdebt-loop-test.sh
shellcheck scripts/techdebt-loop.sh scripts/tests/techdebt-loop-test.sh
cargo test
```

Expected: driver tests all pass; shellcheck silent; the full workspace suite unaffected (this PR adds no Rust). All three must succeed — do not proceed on any failure.

- [ ] **Step 2: Repeat the smoke test once more from a clean state**

```bash
rm -rf ~/.cairn-loop/lock
scripts/techdebt-loop.sh --smoke
```

Expected: exit 0, `smoke test passed` in output.

- [ ] **Step 3: HANDOVER currency note**

Add one bullet to `docs/HANDOVER.md`'s current-state/tooling section (keep the file under 500 lines):

```markdown
- **Tech-debt loop** (2026-07-29): `/techdebt-loop` triages issues into
  `loop:*` labels and drives `/techdebt-next` one fresh headless session per
  issue until the ready backlog is dry (spec:
  `docs/superpowers/specs/2026-07-29-techdebt-loop-skill-design.md`).
  First run: `--dry-run`, then `--max-issues 1`, then unbounded.
```

- [ ] **Step 4: Commit and open the PR**

```bash
git add docs/HANDOVER.md
git commit -m "docs: HANDOVER note for the tech-debt elimination loop"
git push -u origin claude/tech-debt-elimination-skill-132fc7
gh pr create --title "feat: tech-debt elimination loop (/techdebt-loop + /techdebt-next + driver)" --body "$(cat <<'EOF'
Implements the approved design in docs/superpowers/specs/2026-07-29-techdebt-loop-skill-design.md.

- scripts/techdebt-loop.sh — TDD'd bash driver: fresh `claude -p "/techdebt-next"` session per issue, outcome classification (incl. usage-limit auto-resume with reset-epoch parsing), 2-consecutive-failure abort, stale-safe lock, idempotent loop:* label setup. Tests: scripts/tests/techdebt-loop-test.sh (stubbed claude/gh/sleep).
- .claude/skills/techdebt-next — worker: one issue through plan → TDD → local gate → pre-PR review → PR → second full review → docs → auto-merge → cleanup; crash adoption via loop:in-progress; retry-once-then-park.
- .claude/skills/techdebt-loop — entry: label setup, subagent triage into loop:ready/blocked/needs-human/epic, detached driver launch.
- .claude/settings.json — permission allowlist seed (+ ~/.cairn-loop additionalDirectories) for allowlist-first headless runs.

Paper-parity: not clinical-surface — development tooling only.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 5: Review PR feedback per normal process** (this PR is itself reviewed by the human + any requested review pass before merge — the loop is not used to merge its own implementation).

---

## Self-Review (performed while writing)

- **Spec coverage:** §3 components → Tasks 1–7; §4 labels → Task 3; §5 triage → Task 7; §6 driver incl. rate-limit/`skipped`/`failed-permission`/`--max-wait` → Tasks 1, 4; §7 worker incl. crash adoption, retry-once-then-park, epic re-check → Task 5; §8 permissions → Task 6; smoke verification (spec §11 "verify claude -p invokes the skill") → Task 6. `loop:retry` re-eligibility (spec §7 failure handling "one more attempt in a later round") → worker Step 1 picks `loop:retry` only when ready is empty — matches "later round".
- **Placeholder scan:** none — all code, commands, and skill bodies are complete.
- **Type consistency:** outcome tokens identical across driver (`classify_result`, `main` case arms), stub scenarios, and worker skill; env var names identical in `spawn_worker`, stub, and both skills; exit codes 0/1/2/3/4 consistent between Task 4 interface note and `main`.
