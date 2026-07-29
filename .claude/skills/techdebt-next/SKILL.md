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

Read `TECHDEBT_*` variables with the allowlisted idiom
`jq -n 'env.TECHDEBT_OUTCOME_FILE'` (or `echo "${TECHDEBT_SMOKE:-}"`) —
`env` and `printenv` are not on the permission allowlist.

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
   lowest-numbered one instead of picking fresh — unless `TECHDEBT_FORCE_ISSUE`
   is set and names a DIFFERENT issue, in which case skip adoption entirely
   and proceed to Step 1 with the forced issue (the in-progress one stays
   claimed for the next non-forced run): reconstruct its position
   from GitHub state and resume from there:

   Reconstruct its position with these commands (the crashed session chose
   the slug, so discover rather than guess):
   - Branch: `git ls-remote --heads origin "loop/<n>-*"`; local worktree at
     `~/.cairn-loop/wt/issue-<n>` may also exist.
   - PR: `gh pr list --state all --limit 50 --json number,state,headRefName,mergedAt`
     and filter for `headRefName` beginning `loop/<n>-` (e.g. with jq).
   - Second-review state: `gh pr view <pr> --comments` — the full PR review
     posted in Step 7 is visible as review comments on the PR.

   - PR exists and is MERGED → resume at Step 9 (cleanup).
   - PR exists, checks failing → resume at Step 8's red-CI arm.
   - PR exists, checks green/pending, second review not yet posted (no PR
     review from this loop visible) → resume at Step 7.
   - PR exists, checks green/pending, second review ALREADY posted → resume
     at Step 8 (docs check + merge). Do not re-run the second review.
   - Branch `loop/<n>-*` exists but no PR → delete the remote branch if
     pushed, remove any local worktree at `~/.cairn-loop/wt/issue-<n>`,
     and restart the cycle at Step 2 (the plan comment already posted
     still stands; do not duplicate it if present).
   - No branch → restart at Step 1's labeling (already done) then Step 2.

## Step 1 — pick

- If `TECHDEBT_FORCE_ISSUE` is set and non-empty, that is your issue
  (verify it is open; if not, write outcome `dry` and stop). A forced issue
  is operator-chosen: it need not carry an eligibility label. When claiming
  it, remove its `loop:ready`/`loop:retry`/`loop:epic` label if present and
  proceed even if it has none — this is the one sanctioned exception to the
  "never touch issues without a `loop:*` label" hard rule.
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
local gate after fixes. A finding genuinely out of scope for this issue
gets a follow-up GitHub issue instead (house rule 5) — never drop it
silently.

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
   required checks).
3. Poll every 2 minutes (max 40 min per CI run — the budget restarts when
   you push a fix and re-enable auto-merge):
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

1. Post the failure comment FIRST, with this exact machine-checkable prefix:
   `techdebt-loop: cycle failed (step <step>): <error detail, what you tried>`
   — adopted sessions rely on that prefix to count prior failures.
2. Labels: remove `loop:in-progress`. Then decide retry vs park:
   - This was the issue's SECOND failed cycle if EITHER you claimed it by
     removing `loop:retry` in Step 1, OR its comments already contain an
     earlier `techdebt-loop: cycle failed` comment → add `loop:failed`.
   - Otherwise (first failure) → add `loop:retry`.
3. If a PR is open: `gh pr ready <pr> --undo` (convert to draft).
4. Remove the worktree (as in Step 9).
5. Write outcome `failed` (step = where it died, detail = one-line error).
   End your turn.

## Hard rules

- ONE issue per session. Ending early with an honest `failed` outcome beats
  a heroic multi-issue session.
- Never `git push --force`, never rewrite main, never merge a red PR.
- Never touch issues without a `loop:*` label (sole exception: a
  `TECHDEBT_FORCE_ISSUE` issue — operator-chosen).
- The outcome file write (when `TECHDEBT_OUTCOME_FILE` is set) is always your last file action.
