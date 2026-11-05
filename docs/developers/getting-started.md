# Getting started

This page takes you from a fresh clone to a green build of every component. Cairn is a polyglot
repository — Rust, in-database SQL/PL-pgSQL, a Rust-in-Postgres extension, a Tauri 2 desktop window,
a Python package, and a MkDocs site — but you do **not** need all of them working to contribute to
one. Set up only the surface you are touching; each section below is independent.

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
| **rustup** | the Rust toolchain for all three cargo trees | Install via [rustup](https://rustup.rs). You do **not** pick a version: `rust-toolchain.toml` at the repo root pins one exact toolchain (with `rustfmt` + `clippy`) and rustup installs it on first use. The workspace's `rust-version` is kept in step with that pin. |
| **PostgreSQL ≥ 18** | the integration substrate + the in-DB floor | The floor is stated in `db/001_envelope.sql`; below it the schema does not load (it uses aggregates older servers lack). A legacy PG 16 build path exists only for the extension (see §4). |
| **cargo-pgrx** | builds/installs the `cairn_pgx` in-database extension | `cargo install cargo-pgrx --version 0.18.1 --locked` — it **must** match the `pgrx = "=…"` pin in `extensions/cairn_pgx/Cargo.toml`. |
| **uv** | the Python matcher **and** the docs build | [Astral's `uv`](https://docs.astral.sh/uv/). **Use `uv`, never `venv`/`pip` directly** — it is how the matcher and the site are built in CI. |
| **WebKit/GTK dev libraries** (Linux only) | the Tauri 2 reference window (`cairn-gui/`) | Debian/Ubuntu: `libwebkit2gtk-4.1-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev librsvg2-dev` (the CI `gui` job's list in `.github/workflows/rust.yml` is authoritative). macOS needs only the Xcode command-line tools. **No Node.js/npm** — the window has no JS build step. |

You can do meaningful work with a subset:

- **Spec / docs / ADR work** → only `uv` (for the site build).
- **Pure Rust work** (`cairn-event`, `cairn-wire`, `cairn-medium`, the view-model crates) → only rustup.
- **In-DB floor or `cairn-node` integration work** → rustup + PostgreSQL ≥ 18 + `cargo-pgrx`.
- **Reference-window work** → rustup + the WebKit libraries; add PostgreSQL to run it against a real node.
- **Matcher work** → only `uv` (pure tests); add PostgreSQL for the integration tests.

---

## 2. Clone

```bash
git clone https://github.com/cairn-ehr/cairn-ehr.git
cd cairn-ehr
```

Top-level layout (full detail in the [Repository map](repository-map.md)):

```
crates/        the root Rust workspace: wire core, sync, the node daemon, shared view models
extensions/    cairn_pgx — the in-database Rust extension (pgrx); its OWN cargo tree
cairn-gui/     the Tauri 2 reference window; its OWN cargo workspace
db/            numbered SQL migrations + SQL test mirrors — the in-DB enforcement floor
matcher/       cairn-matcher — the advisory patient-matcher (Python, uv project)
scripts/       the local test gate, the PG-cluster resolver, measurement + CI helpers
docs/          the spec, ADRs, principles, and this manual (MkDocs site)
poc/           frozen historical proof-of-concept spikes — read-only reference
web/           the public landing page (separate from the docs site)
```

There are **three separate cargo trees** — the root workspace, `extensions/cairn_pgx`, and
`cairn-gui/`. The root `Cargo.toml` `exclude`s the other two, so **`cargo test --workspace` at the
root never touches them**. Each has its own commands below.

---

## 3. The root Rust workspace (`crates/`)

```bash
# Build and run the tests. Without a database, you must SAY so (see below).
CAIRN_ALLOW_DB_SKIP=1 cargo test --workspace

# Lint exactly as CI does — warnings are errors.
cargo clippy --workspace --tests -- -D warnings

# Formatting, across all three trees.
cargo fmt --all
cargo fmt --manifest-path extensions/cairn_pgx/Cargo.toml
cargo fmt --all --manifest-path cairn-gui/Cargo.toml
```

### Database-gated tests

Most of the safety-relevant tests (anything touching the in-DB floor — the `demographics*`,
`medication*`, `note*`, `vault*`, `restore*`, `federation` suites in `crates/cairn-node/tests/`, and
the clinical-plane suites in `cairn-sync`) need a **live PostgreSQL ≥ 18 with `cairn_pgx`
installed** (see §4). They find it through **three** connection strings, one database each:

| Variable | Database | Needed by |
|---|---|---|
| `CAIRN_TEST_PG` | `cairn_test` | every floor / projection / identity / clinical-stream suite |
| `CAIRN_TEST_PG2` | `cairn_test2` | multi-node convergence and schema-subset suites |
| `CAIRN_TEST_PG3` | `cairn_test3` | the three-node federation acceptance test |

**The one command for the database half of the local gate is:**

```bash
scripts/run-db-gated-tests.sh
```

It discovers a running PostgreSQL ≥ 18 cluster (`scripts/pg-target.sh` — set `PGPORT` to name one,
and it refuses to guess if several qualify), exports all three strings, runs the `db/tests/*.sql`
mirrors, then the full root-workspace `cargo test`, then the one `cairn-gui` crate that talks to a
real node (`cairn-gui-live`). It takes no arguments by design.

Three things that will otherwise surprise you:

- **A skipped test is not a passing test.** Each DB-gated test self-skips when its variable is unset
  — and a skip prints `ok`. So `db_gate_actually_ran.rs` **fails** a run that is missing the
  variables unless you declare the opt-out with `CAIRN_ALLOW_DB_SKIP=1`. That is why the bare
  `cargo test --workspace` above needs it. Setting it loses real coverage; that is the point of
  having to say so.
- **The suites share one cluster and serialize themselves** with a Postgres advisory lock
  (`db::test_serial_guard`). Advisory locks are scoped **per database**, so every suite takes the
  guard on `CAIRN_TEST_PG` whatever database its own work then uses.
- **Run the SQL mirrors only through `scripts/run-db-sql-tests.sh`** (which the gate script calls).
  They are destructive fixtures that run in a throwaway `cairn_sqltest` database the script drops and
  recreates, and each refuses any database without the `cairn_scratch_database` marker.

The full rules for the local gate (and what the required CI checks are) live in the root
`CONTRIBUTING.md`, which is kept current with the workflows. Read its *Running the tests locally*
section once.

### How the schema gets into a database

You do not apply `db/*.sql` by hand. `cairn-node` embeds the ordered migration list (the `SCHEMA`
array in `crates/cairn-node/src/db.rs`) and **replays it on connect**: idempotently, under a load
lock, refusing a database stamped by a *newer* build (the downgrade guard), and skipping the replay
entirely when the `node_schema` stamp says nothing changed
([ADR-0080](../spec/decisions/0080-a-connect-replays-the-schema-only-when-it-changed.md)). The
schema's version is `SCHEMA_GENERATION` in `crates/cairn-event/src/schema_generation.rs`; adding a
migration bumps it, and a guard test fails if the two disagree.

---

## 4. The in-database extension (`cairn_pgx`)

`cairn_pgx` is the small Rust surface that runs **inside** PostgreSQL: event-signature verification
(`cairn_verify`, the gate `submit_event` calls), blob verification, the DEK-wrap/unseal primitives
behind born-sealed bodies (`cairn_wrap_dek`, `cairn_unseal_body`), and the key-vault unseal
(`cairn_vault_unseal`). It is built with
[pgrx](https://github.com/pgcentralfoundation/pgrx), not plain `cargo`, which is why it is its own tree.

```bash
cd extensions/cairn_pgx

# One-time: register your PG 18 install with pgrx (or let it manage one for you).
cargo pgrx init --pg18 "$(which pg_config)"   # point at a PG 18 pg_config, not just any on PATH

# Build + install into that PostgreSQL (the default feature is pg18).
cargo pgrx install

# The extension's own tests, inside Postgres.
cargo pgrx test
```

> [!IMPORTANT]
> **Schema migrations can require a newer extension.** When a migration needs a new extension
> function, the extension's version (`extensions/cairn_pgx/Cargo.toml`) is bumped in the same change
> and the migration refuses to load against an older install. If a DB-gated suite suddenly fails at
> schema load after you pull, re-run `cargo pgrx install` first.

> [!NOTE]
> A legacy PG 16 path is retained for an older local instance:
> `cargo pgrx install --no-default-features --features pg16`. pgrx permits exactly one `pgNN` feature
> at a time. The schema itself does not load on PG 16 — this is for the extension only.

---

## 5. The reference window (`cairn-gui/`)

The reference UI is a **Tauri 2** desktop window
([Spike 0004](../spikes/0004-iced-reference-ui-viability.md) retired the earlier iced shell over
accessibility). It is a separate cargo workspace so that the webview stack never enters
`cairn-node`'s dependency tree — `cairn-gui/*` may depend on `crates/*`, never the reverse.

```bash
cd cairn-gui

# Tests (view models, session lock, the JS/Rust drift guard). cairn-gui-live is DB-gated.
CAIRN_ALLOW_DB_SKIP=1 cargo test --workspace
cargo clippy --workspace --tests -- -D warnings

# Fixture mode — no database, nothing written anywhere. The fastest way to see the window.
cargo run -p cairn-gui-tauri -- --mock
```

**Against a real node** the window needs a node database that `cairn-node` has initialised, and its
**own** low-privilege login: it never connects as the owner and never loads the schema
([ADR-0084](../spec/decisions/0084-the-window-never-holds-the-owners-keys.md)). In outline:

1. `cairn-node … init` (creates the node key and loads the schema);
2. `cairn-node … provision-window-role --role <login>` (the window's login; refuses a cluster with
   any `trust` rule in `pg_hba.conf` — use `scram-sha-256`);
3. `cairn-node … vault-bootstrap-superuser` and `vault-set-terminal-passphrase` (the first staff
   member of the key vault, [ADR-0083](../spec/decisions/0083-the-human-key-vault.md));
4. `CAIRN_CONN="<the window's connection string>" cargo run --release -p cairn-gui-tauri -- --key <node key>`.

The exact, tested commands — including a throwaway `scram-sha-256` cluster and the server-logging
settings that must be checked so a passphrase never reaches a log — are in
`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` §0 and §16. Start there rather than improvising; the
window refuses several plausible-looking shortcuts on purpose (a remote host, an over-privileged
login, a database prepared by a different build), and each refusal tells you which.

Read `cairn-gui/cairn-gui-tauri/README.md` before changing the frontend: it is hand-written,
unbundled JavaScript (`src-ui/`), kept honest by Rust tests that check every field the JS reads is
one the backend sends.

---

## 6. The Python matcher (`cairn-matcher`)

The advisory patient-matcher is the project's first **Python** component (the *fit-for-purpose* tier
— see the [language rule](architecture-for-developers.md#6-choosing-a-language-defect-blast-radius)).
It is a `uv` project with a **pure, zero-runtime-dependency core** and an optional DB-bearing
`pipeline` extra.

```bash
cd matcher

uv run ruff check .        # lint (CI runs this)
uv run pytest              # pure scoring-core tests — no database

# Integration tests (the pipeline that reads patient_* projections and writes proposals):
# needs PostgreSQL >= 18 + cairn_pgx, and the optional `pipeline` extra (psycopg).
CAIRN_TEST_PG="host=127.0.0.1 port=<port> user=<your-pg-user> dbname=cairn_test" \
  uv run --extra pipeline pytest
```

The integration tests follow the same rule as the Rust ones: without `CAIRN_TEST_PG` they fail
unless `CAIRN_ALLOW_DB_SKIP=1` is declared. Running the commit-time duplicate worker for real is
its own page: [Running the duplicate check](running-the-duplicate-check.md).

---

## 7. The documentation site

The spec, the ADRs, the principles, and this manual are a [MkDocs](https://www.mkdocs.org/)
(Material theme) site. **Source is Markdown; HTML is generated and never hand-edited or committed**
(the built `site/` is gitignored).

```bash
# Live preview at http://127.0.0.1:8765 (the port is set in mkdocs.yml).
uv run --with-requirements docs/requirements.txt -- mkdocs serve

# One-off strict build — exactly what CI runs.
uv run --with-requirements docs/requirements.txt -- mkdocs build --strict
```

Always use the **pinned** `docs/requirements.txt`, never ad-hoc `--with` installs — unpinned docs
tooling executes unaudited code at build time.

> [!IMPORTANT]
> The CI check (`.github/workflows/docs-check.yml`) runs `mkdocs build --strict` on **every** PR — a
> dead internal link, a bad nav entry, a page missing from the nav, or a link to an excluded file
> (`HANDOVER.md`, `superpowers/`, `requirements.txt` are excluded from the build) turns into an
> **error** and blocks the merge. Run the strict build locally before pushing doc changes, and refer
> to excluded files by path in backticks, not with a Markdown link.

Author callouts in GitHub/Obsidian syntax (`> [!NOTE]`) so the same source renders correctly both on
GitHub and as Material admonitions in the built site.

---

## 8. What CI runs

Every pull request runs the workflows in `.github/workflows/`. In short:

- **`rust.yml`** — `rustfmt` over all three trees; `cargo-deny` (AGPL-compatible licence allow-list,
  RUSTSEC advisories) over all three; the **floor job**, which builds `cairn_pgx` into a real
  PostgreSQL 18 and runs `clippy -D warnings` + `cargo test --workspace` with all three
  `CAIRN_TEST_PG*` set, the SQL mirrors, the matcher's DB-gated suite and the blocking `cargo doc`
  gate; and a separate job for the `cairn-gui` workspace.
- **`matcher.yml`** — `ruff check` + the pure pytest suite.
- **`docs-check.yml`** — the strict docs build (and `docs.yml` deploys it on merge to `main`).
- **`codeql.yml`** — CodeQL. Read open alerts with `scripts/codeql-alerts.sh`; house rule 6 (in
  [Contributing workflow](contributing-workflow.md#the-coding-house-rules)) explains the recurring
  crypto-naming trap.
- **`closing-keywords.yml`** — refuses a PR title/body/commit that would make GitHub close an issue
  the text says is *not* closed.

**Which of these are required checks, and which only advise**, is tabled in the root
`CONTRIBUTING.md` (*Continuous integration — the required checks*). Treat a red advisory job as a
blocker by hand. CI does not replace the local gate: run `scripts/run-db-gated-tests.sh` before you
push anything that touches the database.

---

## Next

You have a build. Now learn the shape of what you just built:
**[Architecture for developers →](architecture-for-developers.md)**
