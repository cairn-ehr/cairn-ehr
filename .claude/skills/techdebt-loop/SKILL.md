---
name: techdebt-loop
description: Triage all open GitHub issues into loop:* labels, then launch the tech-debt elimination driver that works loop:ready issues one fresh session at a time until the backlog is dry. Args are passed to the driver (--dry-run stops after triage; --max-issues N, --include-epics, --issue N, --bypass, --max-wait H, --timeout H).
---

# techdebt-loop — triage the backlog, then launch the driver

Design doc: `docs/superpowers/specs/2026-07-29-techdebt-loop-skill-design.md`.
Arguments you receive are driver flags, except `--dry-run` which you handle
yourself (triage only, no launch).

## 1. Preflight

- `git fetch origin main` and confirm `gh auth status` succeeds. If either
  fails, stop and report it — triage and the driver both depend on them.
- `scripts/techdebt-loop.sh --setup-labels` (idempotent).
- If `~/.cairn-loop/lock/pid` names a live process whose command line
  contains `techdebt-loop` (`ps -p <pid> -o command=`), a loop is already
  running — tell the user and stop. A dead or unrelated PID is a stale
  lock; the driver reclaims it itself, so proceed.

## 2. Triage (spec §5)

Collect issues to classify: every OPEN issue with NO `loop:*` label, plus
every `loop:blocked` issue (re-check whether its blocker has cleared).

Classify in parallel: dispatch Explore subagents in batches of ~8 issues.
Each subagent's prompt MUST state: "Read-only triage: do not run any
state-changing command (no `gh issue edit`, no `gh issue comment`, no
label changes) — return classifications and justifications as text only."
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

(same pattern for needs-human / epic. A re-checked `loop:blocked` issue
may transition to ANY other classification: swap `loop:blocked` for
`loop:ready` if the blocker cleared, or for `loop:needs-human` /
`loop:epic` if the re-check reveals it was misfiled — always with a
comment saying why.) Already-correct labels are left untouched.

Apply only the four defined classifications. If a subagent's result is
off-taxonomy, missing a justification, or otherwise malformed, re-dispatch
that batch once; if still malformed, classify those issues yourself before
writing any label or comment.

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
DRIVER_PID=$!
disown
sleep 3
kill -0 "$DRIVER_PID" 2>/dev/null || { tail -5 ~/.cairn-loop/run.log; }
```

If the driver died within those 3 seconds, report the log tail as a failed
launch instead of handing the user monitoring instructions. Otherwise tell
the user:
- how to watch: `tail -f ~/.cairn-loop/run.log` (driver log) and the
  per-iteration transcripts under `~/.cairn-loop/run-<timestamp>/`;
- how to stop: `kill <pid>` (report the PID) — the driver exits promptly
  and its EXIT trap releases the lock. The in-flight worker session keeps
  running to its own completion or timeout; its issue stays claimed via
  `loop:in-progress` and the next run's crash recovery adopts it. State is
  safe in GitHub either way.
- that re-running `/techdebt-loop` after ANY interruption is always safe
  (crash recovery adopts in-flight issues).

For a FIRST-EVER run, recommend the user instead do: `--dry-run` first,
then `--max-issues 1` watched via the log, before an unbounded run.
