---
name: finalcheck
description: final pre-merge pass on the current PR — review, fix everything, then drive GitHub CI to green
disable-model-invocation: false
allowed-tools: Bash(git *), Bash(gh pr *), Bash(gh run *), Bash(gh issue *), Bash(cargo *), Bash(uv *), Bash(bash *), Bash(python3 *), Bash(psql *)
---
Run these steps in order on the PR for the current branch. Work can live in the main checkout or in a worktree
under `.claude/worktrees/`, so first run `pwd && git branch --show-current && git worktree list`, and `cd` into
the checkout holding the PR's branch. Then find the PR with `gh pr view --json number,url,headRefOid,isDraft`. If
there is none, stop and tell me. **`gh api` is deny-listed repo-wide and must stay that way**: use `gh pr` /
`gh run` / `gh issue` only.

1. **Review.** Invoke the `pr-review-toolkit:review-pr` skill via the Skill tool and let it finish.
2. **Fix.** Invoke the `fixall` skill via the Skill tool on that review's findings: fix what is fixable now,
   file a GitHub issue for the rest (house rule 5), and bring `docs/HANDOVER.md` / `docs/ROADMAP.md` up to date
   only where the fixes change them. Merge `origin/main` first if the branch is behind. Before every commit run
   `python3 scripts/check_closing_keywords.py <commit-message-file>` (it must say "closes no issue"; the slip
   lives in the commit message and fixing it needs a force-push). Commit and push into the PR, with
   `git -c credential.helper= -c 'credential.helper=!gh auth git-credential' push` (plain HTTPS push has no
   credentials in this shell). Then confirm the push landed: `git fetch origin && git status -sb` must show no
   `ahead`, and the PR's `headRefOid` must equal `git rev-parse HEAD`.
3. **Drive CI to green.** Loop:
   - Watch the checks for the pushed head commit. Interactively, `gh pr checks --watch` dies on
     `connection reset by peer`: use the Monitor tool with a poll loop over `gh pr checks <n> --json
     name,bucket` (30 s+ apart) that emits each finished check and exits when none is pending. Checks can take
     a minute to register. `docs.yml` is path-gated (deploys on push to `main` only) and posts no PR check;
     that is normal.
   - **All green** → done. No status check is REQUIRED on `main` yet (#444, admin-only), so "all green" means
     every check posted on the head commit passed: `rustfmt`, `cargo doc (API surface)`, `cargo-deny`,
     `clippy + cargo test (cairn-gui)`, `clippy + cargo test (cairn_pgx floor)`, `ruff + pytest`, `build` (the
     docs build), `closing-keyword guard`, `CodeQL (rust|python|javascript-typescript|actions)` plus the
     `CodeQL` aggregate, and `Cloudflare Pages`. Report the PR URL and the final check list.
   - **A check failed** → read the cause: `gh run list --branch "$(git branch --show-current)" --limit 5`, then
     `gh run view <run-id> --log-failed`. For a failed **CodeQL** check, the new alerts are NOT visible through
     `scripts/codeql-alerts.sh` (it reads `main` only): WebFetch the failing CodeQL check-run URL to get rule /
     file / line / message, and remember CodeQL picks crypto sinks by binding NAME (CLAUDE.md house rule 6).
     Reproduce locally with the command the failing job runs (`.github/workflows/`), using a scratch target dir
     (`export CARGO_TARGET_DIR=/tmp/finalcheck-target`; a running IDE holds `target/`):
     - `rustfmt` → `cargo fmt --all --check`, `cargo fmt --check --manifest-path extensions/cairn_pgx/Cargo.toml`,
       `cargo fmt --all --check --manifest-path cairn-gui/Cargo.toml`
     - `cargo doc (API surface)` → `RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --workspace` (an
       intra-doc link to a private item fails here and nowhere else)
     - `cargo-deny` → `cargo deny check`, `cargo deny --manifest-path extensions/cairn_pgx/Cargo.toml check
       --config deny.toml`, `cargo deny --manifest-path cairn-gui/Cargo.toml check`
     - `clippy + cargo test (cairn-gui)` → `cargo clippy --locked --manifest-path cairn-gui/Cargo.toml
       --workspace --tests -- -D warnings`, `CAIRN_ALLOW_DB_SKIP=1 cargo test --locked --manifest-path
       cairn-gui/Cargo.toml --workspace`, `RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
       --manifest-path cairn-gui/Cargo.toml --workspace`. A stale `cairn-gui/Cargo.lock` or
       `extensions/cairn_pgx/Cargo.lock` after a root-workspace crate change fails `--locked` here only.
     - `clippy + cargo test (cairn_pgx floor)` → `cargo clippy --locked --workspace --tests -- -D warnings`,
       then `bash scripts/run-db-gated-tests.sh` (SQL mirrors + the full workspace with `CAIRN_TEST_PG*` +
       `cairn-gui-live`; with `PGHOST=127.0.0.1 PGPORT=5532` on the Mac). The full run takes ~2 h: run it
       DETACHED (`nohup … & disown`, in a worktree pinned to the PR head, with a done-marker file), never as a
       harness background job, which can be SIGTERMed. For one failing suite, run just
       `cargo test --locked -p <crate> --test <suite> -- --nocapture` with the three `CAIRN_TEST_PG*` strings,
       and check there is no `skipped:` line. Also `cargo test --locked -p cairn-medication-view --features
       fixtures` and the job's script self-tests (`bash scripts/tests/pg_target_test.sh`,
       `python3 scripts/tests/measure_dr_restore_test.py`, `python3 scripts/tests/measure_patient_search_test.py`,
       `python3 scripts/measure_prompt_truncation.py --self-test`).
     - `ruff + pytest` → `cd matcher && uv run ruff check . && uv run pytest -q` (uv only, never pip/venv; never
       beside `run-db-gated-tests.sh`, as both replay DDL on `cairn_test` and deadlock)
     - `build` (docs) → `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict` (the pinned
       requirements only; never ad-hoc `--with`)
     - `Cloudflare Pages` → the landing page under `web/` (not a GitHub Actions run; read its details link
       from `gh pr checks`). A PR that does not touch `web/` failing here is almost always infra.
     - `closing-keyword guard` → `python3 scripts/tests/check_closing_keywords_test.py`,
       `bash scripts/tests/collect_pr_text_test.sh`, then `python3 scripts/check_closing_keywords.py` on the PR
       title + body and every commit message. A slip in a pushed commit message needs a force-push: stop and ask
       me first.

     Judge each command by its **exit code**, never by filtered output: `cmd | tail` reports `tail`'s status
     (in zsh read `${pipestatus[1]}`), and a killed test binary exits 101 with no `test result: FAILED` line.
     A gate is evidence only for the tree it ran against: re-run after the final edit. Fix the root cause,
     test-first where it is a code bug. Confirm the fix passes locally, commit with a message naming the CI
     failure, push, and loop again.
   - **Linux-only failures** (CI is ubuntu-24.04 and macOS passes locally): reproduce in Docker (`ubuntu:24.04`
     with the toolchain `rust-toolchain.toml` pins; start the daemon with `open -a Docker`). Don't guess.
   - **A job CANCELLED with no log** means the runner was killed, not that a test was slow. Treat it as a code
     bug and find what killed the runner. Do not rerun it as a flake.
   - If the failure is not caused by the code (runner/infra outage, network flake, missing secret), rerun once
     with `gh run rerun <run-id> --failed`. If it fails the same way again, stop and report rather than
     changing code to work around it. The Actions API returns empty or stale data during an outage, so re-query
     before concluding a run never happened.
   - Never "fix" CI by skipping, `#[ignore]`-ing or deleting tests, or by weakening lint/CI config, source
     guards, `deny.toml`, CodeQL model rows, or any allowlist (the license allow-list is the anti-capture
     mission; `crypto_sink_names_are_genuine.rs`'s `ALLOWED` is the inventory of real cryptography), without my
     explicit permission. Bumping a pinned count for a genuinely new member (`twin_registry.rs` +
     `db/tests/034`, both `cairn_projection_apply` counts, `db_errors_stay_legible.rs`) is correct. Lowering one
     to make a test pass is not. If the same failure survives three fix attempts, stop and report what you
     tried.
4. **Not covered by CI.** CI never runs `cairn-gui-tauri`'s DB-gated tests (#905). If the PR touches
   `cairn-gui/`, run them locally: `cargo test --locked --manifest-path cairn-gui/Cargo.toml -p
   cairn-gui-tauri -- --nocapture` with the three `CAIRN_TEST_PG*` strings, and confirm no `skipped:` line.
   Also run the ROOT source guards that read `cairn-gui/*/src`, DB-free:
   `grep -ln production_rust_files crates/*/tests/*.rs` gives the suites; run them with an array of
   `--test` flags (zsh never word-splits an unquoted `$args`). Include both results in the report. A
   human-owed RUNBOOK measurement the PR adds stays owed. Say so in the report; never mark it done.
