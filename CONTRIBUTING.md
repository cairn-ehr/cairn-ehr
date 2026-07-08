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
