# Getting started

This page takes you from a fresh clone to a green build of every component. Cairn is a polyglot
repository — Rust, in-database SQL/PL-pgSQL, a Rust-in-Postgres extension, a Python package, and a
MkDocs site — but you do **not** need all of them working to contribute to one. Set up only the
surface you are touching; each section below is independent.

> [!NOTE]
> **You only ever talk to one integration substrate: PostgreSQL.** This is a deliberate architecture
> choice (*fat Postgres, thin daemon* — [ADR-0001](../spec/decisions/0001-fat-postgres-thin-daemon.md)).
> If you understand that the database is where the safety-critical logic lives, the rest of the layout
> makes sense. See [Architecture for developers](architecture-for-developers.md).

---

## 1. Prerequisites

| Tool | Why | Notes |
|---|---|---|
| **git** | version control | Sign off commits with `git commit -s` (DCO — see [Contributing workflow](contributing-workflow.md)). |
| **Rust** toolchain (stable, ≥ 1.74) | the `cairn-event` / `cairn-sync` / `cairn-node` crates | Install via [rustup](https://rustup.rs). Edition 2021. |
| **PostgreSQL ≥ 18** | the integration substrate + the in-DB floor | The target deployment is PG 18. PG 16 is retained only for a legacy local path (see the extension section). |
| **cargo-pgrx** | builds/installs the `cairn_pgx` in-database extension | `cargo install cargo-pgrx --version 0.18.1` (must match the pin in `extensions/cairn_pgx/Cargo.toml`). |
| **uv** | the Python matcher **and** the docs build | [Astral's `uv`](https://docs.astral.sh/uv/). **Use `uv`, never `venv`/`pip` directly** — it is how the matcher and the site are built in CI. |

You can do meaningful work with a subset:

- **Spec / docs / ADR work** → only `uv` (for the site build) is needed.
- **Pure Rust event-core work** (`cairn-event`, `cairn-sync`) → only the Rust toolchain.
- **In-DB floor or `cairn-node` integration work** → Rust + PostgreSQL ≥ 18 + `cargo-pgrx`.
- **Matcher work** → only `uv` (pure tests); add PostgreSQL for the integration tests.

---

## 2. Clone

```bash
git clone https://github.com/cairn-ehr/cairn-ehr.git
cd cairn-ehr
```

Top-level layout (full detail in the [Repository map](repository-map.md)):

```
crates/        Rust workspace: cairn-event, cairn-sync, cairn-node
extensions/    cairn_pgx — the in-database Rust extension (pgrx); NOT in the workspace
db/            numbered SQL migrations (001…) + SQL tests — the in-DB enforcement floor
matcher/       cairn-matcher — the advisory patient-matcher (Python, uv project)
docs/          the spec, ADRs, principles, and this manual (MkDocs site)
poc/           frozen historical proof-of-concept spikes — read-only reference
web/           the public landing page (separate from the docs site)
```

---

## 3. The Rust workspace (`cairn-event`, `cairn-sync`, `cairn-node`)

The workspace is defined by the root `Cargo.toml`. Note that `extensions/cairn_pgx` is deliberately
**excluded** from the workspace — it is built with `cargo pgrx`, not plain `cargo` (see below).

```bash
# Build and run the unit/integration tests that need no database.
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets    # lint — keep it clean; CI-equivalent gate
```

### Database-gated tests

Many `cairn-node` integration tests (anything touching the in-DB floor — `demographics*.rs`,
`match_veto.rs`, `floor_enforced.rs`, `federation.rs`, …) need a **live PostgreSQL with the
`cairn_pgx` extension installed** (see the next section to install it). They self-serialize
cluster-wide via a Postgres advisory lock (`db::test_serial_guard`), so a plain
`cargo test --workspace` is reliable even though they share one cluster.

They discover the database through an environment variable:

```bash
export CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=<your-pg-user> dbname=cairn_test"
cargo test --workspace
```

A test that needs the database and finds no `CAIRN_TEST_PG` skips cleanly rather than failing, so the
no-database `cargo test --workspace` above stays green on a machine without PostgreSQL.

---

## 4. The in-database extension (`cairn_pgx`)

`cairn_pgx` is the small Rust surface that runs **inside** PostgreSQL (event signature verification —
the in-DB `cairn_verify` gate). It is built and installed with [pgrx](https://github.com/pgcentralfoundation/pgrx),
not plain `cargo`, and lives outside the workspace for that reason.

```bash
cd extensions/cairn_pgx

# One-time pgrx setup (initializes a managed PG for development if you want one).
cargo pgrx init

# Build + install into your local PostgreSQL 18 (the default feature is pg18).
cargo pgrx install

# Run the extension's own tests inside Postgres.
cargo pgrx test
```

> [!NOTE]
> The default feature targets **PG 18** (`default = ["pg18"]`, pgrx `=0.18.1`). A legacy PG 16 path
> is retained for the older local instance — build it with
> `cargo pgrx install --no-default-features --features pg16`. pgrx permits exactly one `pgNN` feature
> at a time.

Once `cairn_pgx` is installed into your cluster, create the test database and load the schema so the
database-gated Rust and matcher tests can run. The numbered SQL migrations in `db/` load in order
(`001_envelope.sql` → … ); the `cairn-node` crate also carries the canonical ordered SCHEMA array it
applies on `init`. The SQL-level tests live in `db/tests/` and are written in plain SQL.

---

## 5. The Python matcher (`cairn-matcher`)

The advisory patient-matcher is the project's first **Python** component (the *fit-for-purpose* tier
— see the [language rule](architecture-for-developers.md#5-choosing-a-language-defect-blast-radius)).
It is a `uv` project with a **pure, zero-runtime-dependency core** and an optional DB-bearing
`pipeline` extra.

```bash
cd matcher

# Pure scoring-core tests — no database, no dependencies to install beyond the dev group.
uv run pytest

# Integration tests (the pipeline that reads patient_* projections and writes proposals):
# needs PostgreSQL >= 18 + cairn_pgx, and the optional `pipeline` extra (psycopg).
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=<your-pg-user> dbname=cairn_test" \
  uv run --extra pipeline pytest
```

The pure suite is dependency-free and should always pass without any database. The integration tests
skip cleanly when `CAIRN_TEST_PG` is unset.

---

## 6. The documentation site

The spec, the ADRs, the principles, and this manual are a [MkDocs](https://www.mkdocs.org/)
(Material theme) site. **Source is Markdown; HTML is generated and never hand-edited or committed**
(the built `site/` is gitignored).

```bash
# Live preview at http://127.0.0.1:8765 (the port is set in mkdocs.yml).
uv run --with-requirements docs/requirements.txt -- mkdocs serve

# One-off strict build — exactly what CI runs.
uv run --with-requirements docs/requirements.txt -- mkdocs build --strict
```

> [!IMPORTANT]
> The CI check (`.github/workflows/docs-check.yml`) runs `mkdocs build --strict` on **every** PR — a
> dead internal link, a bad nav entry, or a link to an excluded file (`HANDOVER.md`,
> `superpowers/`, `requirements.txt` are excluded from the build) turns into an **error** and blocks
> the merge. Always run the strict build locally before pushing doc changes.

Author callouts in GitHub/Obsidian syntax (`> [!NOTE]`) so the same source renders correctly both on
GitHub and as Material admonitions in the built site.

---

## 7. What CI runs today

Be aware of the honest current state so you don't assume a safety net that isn't there yet:

- **Docs:** `mkdocs build --strict` runs on every PR (`docs-check.yml`) and deploys on merge to
  `main` (`docs.yml`).
- **Rust / Python:** there is **no automated CI for the code yet** — `cargo test`, `cargo clippy`,
  and `uv run pytest` are run **locally** and are expected to be green before you open a PR. Treat the
  local gates as mandatory, not optional. (Adding code CI is itself a welcome contribution.)

---

## Next

You have a build. Now learn the shape of what you just built:
**[Architecture for developers →](architecture-for-developers.md)**
