# Contributing to Cairn

The full contribution guide and project governance live in one place:

### → [docs/principles/GOVERNANCE.md](docs/principles/GOVERNANCE.md)

A few essentials up front:

- **Clinical realism is a first-class contribution.** A well-described front-line failure mode — the
  workflow, its paper-era counterpart, exactly where it breaks, and the honest outcome it should have
  — is a genuine contribution, no code required. Open an issue.
- **The architecture spec is complete; the first clinical surface is under construction.** Much
  contribution is still design work on the Markdown spec under `docs/spec/`, but there is now a Rust/Cargo
  workspace (`crates/`, `extensions/`, `db/`) and an advisory Python matcher (`matcher/`), both with tests
  and CI gates (see *Continuous integration* below). Load-bearing decisions are recorded as immutable
  [ADRs](docs/spec/decisions/README.md) — read the relevant one before reopening a settled question.
- **AGPL-3.0, inbound = outbound, DCO not CLA.** Contributions are under the
  [AGPL-3.0](LICENSE); sign off every commit (`git commit -s`) per the
  [Developer Certificate of Origin](https://developercertificate.org/). The project deliberately uses
  **no CLA** — keeping the copyleft strong and the project uncapturable.
- **The mission is the tie-breaker**, and **paper-parity is the governing law**: no clinical workflow
  may be slower, harder, more cognitively demanding, or impossible than its paper equivalent.

## Running the tests locally

`cargo test --workspace` builds and runs everything, but a large part of this repo's test suite is
**DB-gated**: it needs a local PostgreSQL 18 with the `cairn_pgx` extension installed, reached
through three connection strings.

| Variable | Database | Needed by |
|---|---|---|
| `CAIRN_TEST_PG` | `cairn_test` | every in-DB floor / projection / identity / medication suite |
| `CAIRN_TEST_PG2` | `cairn_test2` | the multi-node convergence and schema-subset suites |
| `CAIRN_TEST_PG3` | `cairn_test3` | the three-node federation acceptance test |

`scripts/run-db-gated-tests.sh` bakes all three in and also runs the `db/tests/*.sql` mirrors — it is
the one command for the database slice of the local gate.

**Without a database, declare it: `export CAIRN_ALLOW_DB_SKIP=1`.** Every DB-gated test self-skips
when its connection string is missing, and a skipped test prints `ok` — so a run that proved nothing
is byte-identical to one that proved everything. `db_gate_actually_ran.rs` closes that by failing
when a gate variable the suite reads is unset, and it **fails closed**
([#450](https://github.com/cairn-ehr/cairn-ehr/issues/450)): an absent opt-out is not permission, and
neither is `CAIRN_ALLOW_DB_SKIP=false`. Only `1`/`true`/`yes`/`on` opts out. The same variable and
the same rule cover the matcher's Python DB-gated suite
([#451](https://github.com/cairn-ehr/cairn-ehr/issues/451)).

The guard binds **per-crate runs too**, not just the workspace one
([#481](https://github.com/cairn-ehr/cairn-ehr/issues/481)): a test binary only runs when its own
crate is tested, so until that issue `cargo test -p cairn-sync` reported `101 passed` with no
database in sight. Both crates that hold DB-gated tests now pull in one shared guard module rather
than keeping a copy each, and a test derived from the tree fails by name if a third crate grows
DB-gated tests without binding it.

You lose real coverage by setting it — that is the point of having to say so. The pure Rust crates
and the matcher's pure suite (`cd matcher && uv run pytest`, never venv/pip) still run in full.

## Continuous integration — the required checks

Every pull request must pass this set of **required status checks** before it can merge to `main`.
Each is a job in a workflow under [`.github/workflows/`](.github/workflows/); the check name is the
job's `name:` and GitHub matches required checks by that **exact name**.

| Required check | Workflow · job | What it gates |
|---|---|---|
| `build` | `docs-check.yml` · `build` | The docs site builds clean (`mkdocs build --strict`). |
| `rustfmt` | `rust.yml` · `fmt` | `cargo fmt --check` across **both** cargo trees (workspace + the `cairn_pgx` extension), against the pinned toolchain (`rust-toolchain.toml`). |
| `cargo-deny` | `rust.yml` · `deny` | AGPL-compatible license allow-list + RUSTSEC advisories + wildcard/source bans (`deny.toml`), on both trees. |
| `ruff + pytest` | `matcher.yml` · `lint-test` | The advisory Python matcher: `ruff check` + the **pure** pytest suite (no database). |
| `clippy + cargo test (cairn_pgx floor)` | `rust.yml` · `test` | The **in-DB safety floor**: builds `cairn_pgx` into a real PostgreSQL 18, then `cargo clippy -D warnings` + `cargo test --workspace` **and** the matcher's DB-gated suite, all with `CAIRN_TEST_PG` set so the gated tests actually run (they self-skip when it is unset). The same job also runs the `db/tests/*.sql` **mirrors** via `scripts/run-db-sql-tests.sh`, and — as its last steps — the **blocking half of the `cargo doc` gate** (root workspace, the `fixtures` feature, and `cairn_pgx`). |

### Where the `cargo doc` gate actually blocks

`cargo doc --no-deps` under `RUSTDOCFLAGS=-D warnings` runs over **all three** cargo trees, but not all
of it blocks a merge, so it is worth being precise rather than implying a uniform gate. A broken docs
build is a gap in the ADR-0021 public API surface, and — the reason [#439](https://github.com/cairn-ehr/cairn-ehr/issues/439)
mattered — it hides every subsequent rustdoc error behind the `-A` flags someone adds to get past it.

| Tree | Runs in | Blocks a merge? |
|---|---|---|
| root workspace (`cairn-event`, `cairn-node`, `cairn-sync`, …) | `test` (last steps) **and** `doc` | **Yes**, via `test` |
| `cairn-medication-view` with `--features fixtures` | `test` (last steps) | **Yes**, via `test` |
| `cairn_pgx` extension | `test` (last steps) | **Yes**, via `test` |
| cairn-gui workspace | `gui` | No — see below |

The root-workspace build is deliberately run **twice**: once in the fast standalone `doc` job for
feedback in under a minute, and once inside `test`, which is a required check. Both of #439's actual
defects were in the root workspace, so relying on the advisory job alone would have let the same
regression merge green.

### Jobs that run but do not yet block

Two jobs run on every pull request and are **not** in `main`'s branch-protection set, so today they can
go red without stopping a merge. Only a repository admin can promote them
([#444](https://github.com/cairn-ehr/cairn-ehr/issues/444)); until one does, treat a red run here as a
blocker by hand. **This table records the branch-protection state as of 2026-08-20** — it lives on
GitHub, not in this repo, so no gate can keep it honest. Verify with
`gh api repos/:owner/:repo/branches/main/protection`, and update this section when #444 lands.

| Check | Workflow · job | What it gates |
|---|---|---|
| `closing-keyword guard` | `closing-keywords.yml` · `guard` | That the PR body and its commit messages do not hand GitHub a closing reference the text says is **not** a close — see the section below. Also prints, on every PR, the list of issues the merge will actually close. |
| `clippy + cargo test (cairn-gui)` | `rust.yml` · `gui` | The reference UI's separate cargo workspace — which `cargo test --workspace` does not cover — including the JS/Rust drift guard that is the only compensating control for a webview with no type checking. Also carries the cairn-gui half of the `cargo doc` gate. |
| `cargo doc (API surface)` | `rust.yml` · `doc` | The root workspace's docs build, as a **fast advisory duplicate** of the copy inside `test`. Nothing is gated only here — promoting it buys speed of signal, not coverage. |

Three things that have bitten us, so they are worth stating outright:

- **The floor check's name is deliberately PostgreSQL-version-independent** (`… (cairn_pgx floor)`, not
  `… (PG18 …)`). Encoding the PG major renames the check on every version bump, which *orphans* the
  required name in branch protection — the required check then never reports and every PR is silently
  blocked (this is exactly what a `PG16 → PG18` rename did once). Do not put the PG major in the job name.
- **Run the `db/tests/*.sql` mirrors only through `scripts/run-db-sql-tests.sh`.** They are destructive
  fixtures — several commit, and `017` drops constraints and replays a migration — so since
  [#169](https://github.com/cairn-ehr/cairn-ehr/issues/169) each one refuses any database that does not
  carry the `cairn_scratch_database` marker table, which that script stamps on the throwaway
  `cairn_sqltest` it drops and recreates at the start of every run. Pointing a mirror at the shared `cairn_test` by hand used
  to collide with residue left by a finished `cargo test`; pointing one at a real node would have been
  much worse. Use `-f`, not stdin: psql's `\ir` cannot resolve the shared preamble from a piped script.
- **Renaming any required job means updating branch protection in lockstep.** Because required checks are
  matched by exact name, changing a job's `name:` without updating `main`'s required-checks list orphans
  the old name. If you must rename, coordinate the branch-protection change with a maintainer.

See [GOVERNANCE.md](docs/principles/GOVERNANCE.md) for the rest — how decisions are made, the
defect-blast-radius rule for code, stewardship of the name, the code of conduct, and responsible
disclosure.

## Writing a PR body: never let a closing keyword touch an issue reference by accident

GitHub closes an issue when a merged PR body **or commit message** puts `close`/`closes`/`closed`,
`fix`/`fixes`/`fixed` or `resolve`/`resolves`/`resolved` immediately before a reference to it. The
parser reads **adjacency, never meaning**, so each of these closed an issue while stating the opposite:

| Written | What GitHub did |
|---|---|
| `It does not fix #500` | closed #500 |
| `It does close #101 item 1` | closed #101 — items 2–3 with it |
| `Filed rather than fixed: #534` | closed #534 |

Seven issues were lost this way before anyone noticed — #101, #115, #434, #441, #468, #500, #534 —
including the tracking issue for the disaster-recovery slice then under construction, and one that
stayed wrongly closed for eight weeks. Nothing surfaced it: a wrongly closed issue simply stops
existing as far as triage, a ROADMAP cross-check or a `/techdebt-loop` run can tell.

**The rule: put a word between the keyword and the reference, or use a verb GitHub does not parse.**

- ✗ `does not fix #500` → ✓ `does not address #500`, or `#500 is not fixed by this slice`
- ✗ `Filed rather than fixed: #534` → ✓ `Filed rather than repaired: #534`
- ✗ `closes #101 item 1` → ✓ `closes item 1 of #101 only`

The conventional-commit scope `fix(#500):` is **safe** — the parenthesis breaks the adjacency, proven
by `fix(#288)` and `fix(#530)` sitting on `main` with both issues open. Keep using it.

`scripts/check_closing_keywords.py` enforces this (its own tests are
`scripts/tests/check_closing_keywords_test.py`; run either directly with `python3`). It also prints
what the merge *will* close, so an unintended close is visible before the merge button, not a month
after it.

## Paper-parity benchmark — a required slice-plan section

Paper-parity is the [governing law](docs/spec/vision.md#12-the-paper-parity-test-normative): §1.2
makes it **falsifiable** — *every clinical workflow must name its paper-era equivalent and benchmark
against it in time, steps, and cognitive load; a workflow that loses to paper is a design defect and
is tracked as one.* To keep that from being enforced by taste, **every slice plan for a slice that
adds or changes a clinical workflow — at any layer, the in-DB floor and event core included — must
carry a Paper-parity benchmark section:**

```markdown
## Paper-parity benchmark (§1.2)

- **Paper counterpart:** <named concretely — e.g. "the drug chart: one signature, one form, one act">
- **Steps (paper → Cairn):** paper N human acts → architecture forces M → UI bundling target K.
  <If M > N: "FAILS parity (architecture defect) → tracked as #NNN.">
- **Time + cognitive load:** budget — <e.g. "re-attest a 6-thread list in ≤ 1 gesture, ≤ 2 s">.
  Unmeasured (no runnable surface); measurement owed by <the slice that first exposes one>.
```

Copy the three limb labels (**Paper counterpart**, **Steps**, **Time + cognitive load**) verbatim —
the enforcing guard matches them exactly, so a re-worded label reads as a *missing* section and fails
the check (loudly and safely — a false-fail is never a false-pass, but it is an avoidable surprise).

Three things make this honest rather than ceremonial:

- **Steps are judged on what the architecture *forecloses*, not on rendered gestures.** Bundling N
  events into one human gesture is a UI/policy job ([ADR-0021](docs/spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md));
  the architecture's duty is only to *not foreclose* it (and ideally promote it). So `M` is the human
  acts the design **forces** — the floor no UI can bundle away. **`M > N` is an architecture defect**
  (file an issue, per §1.2 and house rule 5). `M ≤ N` but a UI exposing more than `K` is a **UI**
  defect, tracked against that UI slice.
- **Only the step-count is binding at plan time.** Steps are countable from the design; *time* and
  *cognitive load* need a runnable workflow. So the section states a step-count claim now and a
  time/load *budget* now, with the measurement owed (and named) by the first slice that ships a
  runnable surface. Declaring a budget we cannot yet measure — rather than fabricating a number — is
  acknowledged uncertainty (principle 4) applied to our own process.
- **Below-the-clinical-surface plans take a forced-rationale escape,** not a checkbox. One line:

  ```markdown
  Paper-parity: not clinical-surface — <substantive recorded reason>.
  ```

  A confirmation-style "N/A" is refused; the reason must be substantive (this is §1.2's own permitted
  friction — a forced-rationale gate, never a click-through — applied to the plan document).

**Enforcement.** A no-DB source-guard test
([`crates/cairn-node/tests/paper_parity_plan_section.rs`](crates/cairn-node/tests/paper_parity_plan_section.rs))
runs inside the existing `cargo test` gate and fails any plan dated on/after 2026-07-24 that carries
neither the section nor a substantive escape line. It is **forward-only** — the plans written before
the rule are the historical record and are left untouched (principle 2). The Tauri reference-client
slice is the first plan it binds.
