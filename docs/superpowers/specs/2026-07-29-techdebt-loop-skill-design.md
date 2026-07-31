# Tech-debt elimination loop — skill design

**Date:** 2026-07-29
**Status:** Approved design, pre-implementation
**Owner:** HH

## 1. Goal

Systematically drain the GitHub issue backlog of bounded tech-debt fixes, one issue at a
time, each through a full quality cycle (plan → TDD → review → PR → second full PR review →
docs → merge), with a **fresh context per issue** so issue #40-in-a-row gets the same
attention as issue #1. Runs unattended until the ready backlog is dry.

## 2. Decisions made during brainstorming

| Question | Decision |
|---|---|
| Merge policy | **Auto-merge everything** once CI is green and both reviews are clean. Branch protection (5 required checks, `enforce_admins`) is the hard backstop — nothing red can merge. |
| Issue eligibility | **Self-triage with labels** at the start of every run; blocked issues re-examined each round (deferral to later rounds falls out naturally). |
| Run scope | **Until dry, with safety valves** (2 consecutive cycle-failures stop the run; per-issue retry-once-then-park). |
| Issue size | **Bounded single-PR fixes only**; multi-PR feature slices labeled `loop:epic`, eligible only with `--include-epics`. |
| Architecture | **Outer bash driver + one fresh headless `claude -p` session per issue** (Approach A). Worker skill also usable standalone/attended. |
| Permissions | **Allowlist-first, graduate to bypass**: project-settings allowlist + `acceptEdits`, driver flag `--bypass` for later. Permission denials stop the run without penalizing the issue. |

## 3. Architecture

Three committed artifacts plus a label taxonomy:

| Artifact | Role |
|---|---|
| `.claude/skills/techdebt-next/SKILL.md` | **Worker**: does exactly one issue through the full cycle. Standalone-invocable (`/techdebt-next`) for attended runs. |
| `.claude/skills/techdebt-loop/SKILL.md` | **Entry point**: preflight, triage pass, then launches the driver. |
| `scripts/techdebt-loop.sh` | **Driver**: dumb bash loop; spawns worker sessions, counts failures, times out, logs, summarizes. |

**Load-bearing principle: GitHub is the only state store.** Labels, issue comments, PR
state, and CI status fully determine where the loop is. No session remembers anything;
restarting after any crash is always just re-running the driver. This is what makes
"clear context per issue" safe.

Run logs live in `~/.cairn-loop/run-<timestamp>/`: one transcript per iteration plus the
worker's `outcome.json` handoff files. Outside the repo.

## 4. Label taxonomy

| Label | Meaning | Set by |
|---|---|---|
| `loop:ready` | Bounded, autonomously fixable, dependencies met | Triage |
| `loop:blocked` | Blocked by unmerged work / upstream / hardware; comment records what unblocks it. Re-examined every run. | Triage |
| `loop:needs-human` | Needs HH's judgment (design sessions, decisions) | Triage |
| `loop:epic` | Multi-PR feature slice; eligible only with `--include-epics` | Triage |
| `loop:in-progress` | A worker session owns this issue right now (or crashed owning it) | Worker |
| `loop:retry` | First cycle failed (comment has detail); one more attempt in a later round | Worker |
| `loop:failed` | Second cycle failed; permanently parked for human triage | Worker |

Labels are created idempotently on first run. Issues already carrying a `loop:*` label are
not re-triaged, except `loop:blocked` (unblocking check) — so triage cost shrinks each run.

## 5. Triage pass

Runs at the start of every `/techdebt-loop` invocation, in the launching (interactive)
session, fanning out subagents over unlabeled issues. Classification per §4. Every
non-`ready` classification gets a one-comment justification on the issue (audit trail;
also the input for the next run's re-check of `loop:blocked`).

`--dry-run` stops after triage and prints the classification table — recommended for the
first run so HH can sanity-check labels before any code is touched.

**Author gate.** The repo is public and an issue body is untrusted input to an unattended,
merge-capable worker (branch protection requires green checks, not human approval). Triage
therefore never gives a non-operator-authored issue an eligibility label (`loop:ready` OR
`loop:epic`) — it parks as `loop:needs-human` with a comment — and the worker re-verifies
authorship before claiming. The gate covers authorship only: comments on operator-authored
issues remain untrusted, so both triage subagents and the worker treat issue text as data
(never instructions), and the worker counts machine-checkable failure markers only from
operator-authored comments.

## 6. Driver (`scripts/techdebt-loop.sh`)

Deliberately dumb — all intelligence is in the worker.

- **Lockfile** (`~/.cairn-loop/lock`) prevents two concurrent loops.
- Loop body: spawn `claude -p "/techdebt-next" --permission-mode acceptEdits`
  (or `--dangerously-skip-permissions` with `--bypass`) with a **per-iteration timeout**
  (default 3 h), transcript teed to the run dir.
- After each iteration, read the worker's `outcome.json`:
  - `merged` — reset consecutive-failure counter, continue.
  - `skipped` — worker relabeled the issue (e.g. discovered it is an epic at plan time)
    without attempting it: continue, counter untouched.
  - `dry` — no `loop:ready` issues remain: summarize, notify, exit 0.
  - `failed` — increment counter; **2 consecutive failures aborts the run** (systemic
    problem: CI infra, DB substrate, allowlist rot).
  - `failed-permission` — **abort immediately without penalizing the issue** (its labels
    are untouched); print the denied command so HH extends the allowlist and re-runs.
  - `merge-pending` — the worker's honest "work done, every check green, auto-merge
    armed, merge not yet fired when its bounded foreground wait ran out": routed
    through the same adoption machinery as a crash (LANDED scan, then adopt the CI
    watch), but **scoped to the issue the outcome file names** — the one arm where the
    driver knows exactly which issue the cycle owned. Under `--issue N` the operator's
    scope beats the worker's declaration; an issue-less `merge-pending` **fails
    closed** (no scope → no scan — a wide scan could adopt foreign wreckage and run
    destructive cleanup on an issue the worker never owned). Adopted → `merged`; watch
    expires or the PR closes unmerged → `failed`. Smoke-mode guard applies as for
    `crashed`.
- **Missing `outcome.json`** (worker died before writing it): the driver classifies from
  the exit code and transcript:
  - transcript matches a usage-limit pattern (`Claude AI usage limit reached|<epoch>` or
    "You've hit your session limit") → **`rate-limited`**: parse the reset epoch when
    present and sleep until reset + 5 min; if no epoch is parseable, sleep 30 min and
    retry. Does **not** touch the consecutive-failure counter and does not penalize the
    issue — a mid-cycle kill is just a crash, absorbed by §7 step 0 adoption. Total
    waiting is capped by `--max-wait` (default 6 h; `0` = wait indefinitely, for
    weekly-cap survival) — beyond the cap, summarize and exit with a clear message.
    (Known bound: a session that dies without `outcome.json` while its final output
    happens to quote a limit phrase is misclassified as rate-limited; the damage is
    capped waiting under `--max-wait`, and the §11 statusLine-hook hardening would
    remove transcript matching entirely.)
  - timeout kill or anything else → **`crashed`**, which is not yet a failure: a
    headless session terminates the instant its turn ends, so a worker that yielded
    with a background wait pending may have died AFTER its merge landed (or with
    auto-merge armed and CI minutes from green) but before its outcome write
    (issue #320, observed 2026-08-01: both cycles of a run died this way while the
    work itself merged, and the false `failed`s tripped the systemic halt). The
    driver runs a GitHub post-mortem before deciding: a `loop:in-progress` issue
    (closed — or still open, when the merged PR lacked a closing keyword) whose
    `loop/<n>-*` PR merged after the iteration started → count `merged` and
    finish the dead worker's cleanup (close-if-unclosed, stale label, worktree,
    branches); an open claimed issue with auto-merge armed on its open PR → adopt
    the dead worker's CI watch (poll up to 30 min) and count by what the PR does;
    anything else → `failed`. Guards: the post-mortem is skipped entirely in smoke
    mode (a smoke worker creates nothing adoptable, and stale wreckage must not
    green a broken plumbing check), scoped to the forced issue under `--issue N`,
    adopts any given issue at most once per run, and closes an adopted issue whose
    merged PR lacked a closing keyword before stripping its claim label. The skill
    side of the same fix forbids workers from backgrounding anything in the first
    place.
- Flags: `--max-issues N`, `--include-epics`, `--issue N` (force one
  specific issue), `--bypass`, `--max-wait H`, `--timeout H`, `--smoke`, `--setup-labels`.
- On any exit: run summary (merged / skipped / failed / iterations) + notification.

The orphaned `loop:in-progress` label left by any kill (timeout, rate limit, crash) is
the next worker's crash-recovery signal (§7 step 0).

## 7. Worker cycle (`/techdebt-next`)

One fresh session, one issue, full cycle. All steps leave GitHub evidence.

0. **Preflight & crash recovery**: fetch main. If a stale `loop:in-progress` issue
   exists, adopt it: reconstruct position from GitHub (branch exists? PR open? CI state?
   review posted?) and resume its cycle from there instead of picking fresh.
1. **Pick**: lowest-numbered `loop:ready` (honoring `--issue N`); label
   `loop:in-progress`; comment "cycle started" with session identifier.
2. **Worktree**: branch `loop/<issue>-<slug>` off fresh `main`. Serial execution +
   always-branch-from-merged-main = no conflict stacking.
3. **Plan**: brief written plan posted as an issue comment (auditable design record).
   Includes the `## Paper-parity benchmark (§1.2)` section per house rule 7 — most tech
   debt takes the one-line not-clinical-surface escape, but the gate test
   (`paper_parity_plan_section.rs`) applies to plan files, and clinical-surface issues
   must genuinely address it.
4. **TDD**: failing test first, then the fix (house rule 2).
5. **Local gate**: full-workspace `cargo test` (never `-p`, never piped through `tail`),
   `cargo fmt --check`, `cargo clippy`, SQL-mirror tests (`scripts/run-db-sql-tests.sh`)
   when `db/` is touched, cairn_test DB recreation when `event_log` columns change.
6. **Pre-PR review**: code-reviewer subagent on the working diff; fix findings.
7. **PR**: open with `Fixes #N` + clear description.
8. **Second, full PR review**: a *fresh-context* multi-agent review of the complete PR
   (whole diff + tests + description — not the incremental working view; empirically it
   still finds real issues). Findings: fix in place, or file a follow-up issue if out of
   scope (house rule 5) — never silently dropped. (`/code-review ultra` is user-triggered
   and billed; the loop cannot launch it and does not try.)
9. **Docs**: HANDOVER/ROADMAP updated only if state materially changed; bundled in the
   same PR.
10. **Merge**: `gh pr merge --auto --merge` (merge-commit convention; `--auto` waits for
    the required checks; requires the repo's "Allow auto-merge" setting — if it is off,
    the worker stops the whole run via `failed-permission` with the fix in the detail,
    keeping `loop:in-progress` so the next run resumes at this step; the entry skill
    also checks the setting in preflight). CI red → one diagnosis-and-fix attempt, then
    treat as cycle failure.
11. **Cleanup**: remove worktree + branch (local and remote — the repo does not
    auto-delete merged heads), verify the issue auto-closed, remove
    `loop:in-progress`, write `outcome.json`, exit.

**Failure handling**: on irrecoverable failure at any step — label `loop:retry` (first
time) or `loop:failed` (second), post a detailed comment (step reached, error, what was
tried), convert any open PR to draft, clean the worktree, write `outcome.json` with
`failed` + reason, exit. A permission denial instead writes `failed-permission` + the
denied command and leaves the issue's labels untouched.

## 8. Permissions

Start with a curated allowlist in project settings (git, gh, cargo, uv, psql,
`scripts/*`; seeded via `/fewer-permission-prompts` from transcript history) and
`--permission-mode acceptEdits`. The `failed-permission` outcome (§6) makes allowlist
gaps cheap: the run stops, HH adds the pattern, re-runs; no issue is penalized. Once the
allowlist has matured through a few supervised runs, graduate to `--bypass`
(`--dangerously-skip-permissions`) for maximum unattended robustness. Branch protection
remains the merge gate in both modes.

Be honest about what the deny list is: prefix patterns are convenience brakes, not a
security boundary — variant spellings (`git push origin -f`, a `+refspec` force push)
evade prefix matching. The enforced boundary is server-side branch protection
(required checks + `enforce_admins`). Note also that project-settings denies apply to
EVERY Claude session in this repo, interactive ones included (e.g. `gh api` reads).

## 9. Non-goals

- No parallel issue execution (serial by design — conflict-free, reviewable, simple).
- No autonomous work on `loop:needs-human` / `loop:blocked` / unlabeled issues.
- No `/code-review ultra` invocation.
- No cross-repo operation (cairn-ehr only, though nothing prevents later reuse).
- The loop does not create architecture: an issue whose fix would require a new ADR or
  spec change is `loop:needs-human` by triage definition.

## 10. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Worker merges something subtly wrong | Two independent reviews + TDD + full CI + HH's async audit of merged PRs; pre-clinical posture bounds blast radius |
| Runaway loop burns tokens on a hopeless issue | Retry-once-then-park; 2-consecutive-failure abort; per-iteration timeout; `--max-issues` |
| Allowlist gap mid-run | `failed-permission` outcome: stop, no issue penalized, exact command reported |
| Crashed session leaves an issue claimed | `loop:in-progress` + GitHub-state reconstruction in the next session's preflight |
| Two loops collide | Driver lockfile |
| Usage-window exhaustion mid-run | `rate-limited` classification: sleep until the parsed reset epoch (+5 min) and resume; issue unpenalized, cycle resumed via §7 step 0 adoption; `--max-wait` caps the zombie-driver risk |
| Shared-file churn (HANDOVER/ROADMAP) across serial PRs | Each cycle branches from freshly-merged main; docs touched only when material |
| Triage misclassifies an epic as ready | Worker re-checks scope at plan time; if the plan exceeds single-PR size, it relabels `loop:epic`, comments, and moves on (counts as no attempt, not a failure) |
| Hostile issue or comment from an untrusted author steers an unattended worker (public repo; no human merge gate) | Author gate (§5): only operator-authored issues get eligibility labels, everything else parks `loop:needs-human`; the worker re-verifies authorship before claiming; issue text is treated as data, never instructions; failure markers counted only from operator-authored comments; branch protection + the two reviews remain downstream nets |

## 11. Deferred to the implementation plan

- Exact allowlist seed list and settings file placement.
- Verification that `claude -p "/techdebt-next"` invokes the skill in the installed CLI
  version, and exact flag set (output format, model).
- Notification mechanism at run end (terminal summary at minimum).
- `outcome.json` schema (issue, outcome, step reached, PR/commit refs, denied command).
- Whether the second review reuses `code-review:code-review` or a purpose-built
  multi-agent review prompt.
- Exact usage-limit transcript patterns to match (verify against the installed CLI
  version at implementation time; both known phrasings above are matched defensively).
  Optional later hardening: a statusLine hook that writes the reset epoch to a flag file
  the driver reads, instead of transcript parsing (community "Smart Resume" pattern).
