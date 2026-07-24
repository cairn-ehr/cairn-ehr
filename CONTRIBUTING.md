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
| `clippy + cargo test (cairn_pgx floor)` | `rust.yml` · `test` | The **in-DB safety floor**: builds `cairn_pgx` into a real PostgreSQL 18, then `cargo clippy -D warnings` + `cargo test --workspace` **and** the matcher's DB-gated suite, all with `CAIRN_TEST_PG` set so the gated tests actually run (they self-skip when it is unset). |

Two things that have bitten us, so they are worth stating outright:

- **The floor check's name is deliberately PostgreSQL-version-independent** (`… (cairn_pgx floor)`, not
  `… (PG18 …)`). Encoding the PG major renames the check on every version bump, which *orphans* the
  required name in branch protection — the required check then never reports and every PR is silently
  blocked (this is exactly what a `PG16 → PG18` rename did once). Do not put the PG major in the job name.
- **Renaming any required job means updating branch protection in lockstep.** Because required checks are
  matched by exact name, changing a job's `name:` without updating `main`'s required-checks list orphans
  the old name. If you must rename, coordinate the branch-protection change with a maintainer.

See [GOVERNANCE.md](docs/principles/GOVERNANCE.md) for the rest — how decisions are made, the
defect-blast-radius rule for code, stewardship of the name, the code of conduct, and responsible
disclosure.

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
