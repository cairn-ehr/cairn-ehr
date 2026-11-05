# Repository map

What every top-level directory is, whether it is **load-bearing** (active product/spec) or
**reference** (frozen/historical), and where to look first. For the *why* behind the structure, read
[Architecture for developers](architecture-for-developers.md) first.

```
cairn-ehr/
├── crates/            root Rust workspace — wire core, sync, node daemon, view models [load-bearing]
├── extensions/        cairn_pgx — in-database Rust extension (pgrx), own cargo tree   [load-bearing]
├── cairn-gui/         the Tauri 2 reference window, own cargo workspace               [load-bearing]
├── db/                numbered SQL migrations + SQL test mirrors — the in-DB floor    [load-bearing]
├── matcher/           cairn-matcher — advisory patient-matcher (Python)               [load-bearing]
├── scripts/           the local test gate, PG-cluster resolver, measurement + CI tools [supporting]
├── docs/              spec, ADRs, principles, spikes, and this manual                 [load-bearing]
├── web/               the public landing page (separate from the docs site)           [load-bearing]
├── packaging/         name-reservation stubs for crates.io / npm / PyPI               [supporting]
├── poc/               frozen proof-of-concept spikes                                  [reference only]
├── scratch/           working sketches / notes                                        [reference only]
├── assets/            shared image/brand assets                                       [supporting]
├── .github/           CI workflows + CodeQL config
├── Cargo.toml         root workspace manifest (excludes extensions/ and cairn-gui/)
├── rust-toolchain.toml  the one pinned Rust toolchain for all three cargo trees
├── deny.toml          cargo-deny policy: the AGPL-compatible licence allow-list + advisories
├── CLAUDE.md          repo guidance (also useful orientation for humans)
├── CONTRIBUTING.md    contribution essentials, the local test gate, the required CI checks
├── README.md          mission + the founding principles
└── mkdocs.yml         docs-site config (nav, theme, strict-build settings)
```

---

## `crates/` — the root Rust workspace

Declared in the root `Cargo.toml` (edition 2021, AGPL-3.0-only, `rust-version` in step with
`rust-toolchain.toml`). **`extensions/cairn_pgx` and `cairn-gui/` are deliberately excluded** — they
are separate cargo trees with their own commands ([Getting started](getting-started.md)).

| Crate | Role |
|---|---|
| **`cairn-event`** | **Wire core (layer 1), safety-critical.** Canonical CBOR, COSE_Sign1/Ed25519 sign + verify, multihash addressing, the `EventSigner` seam, sealing, and the **pure event builders + twins** for every stream (`demographics`, `registration`, `identity`, `john_doe`, `evidence`, `medication/`, `note`, `note_discard`, `sensitivity`, `safety`, `attachment`). Also `schema_generation.rs`. |
| **`cairn-wire`** | The clinical-plane sync protocol: wire types, framing, paging, and the **transport seam** (`tcp.rs` for a peer, `medium.rs` for a backup medium). |
| **`cairn-medium`** | The backup-medium container format: segments, chunks, the hash chain, attestation, health. |
| **`cairn-keystore`** | The at-rest key-file layer (`seal.rs`: Argon2id + AEAD) shared by `cairn-node` and `cairn-sync`. |
| **`cairn-sync`** | The clinical-plane sync daemon: set-union pull/apply via the remote door, paged pulls, `requeue` of penned events, the lazy blob tier. |
| **`cairn-node`** | **The node daemon and library.** CLI (`src/main.rs`), keystore + pairing + mTLS (`pairing.rs`, `transport.rs`), node-plane sync (`sync.rs`), backup/restore (`backup*`, `restore*`, `localstate*`), the key vault (`vault.rs`, `vault_admin.rs`, `enroll.rs`, `actor_enrolment.rs`), the window's connection posture (`conn_posture.rs`, `window_access.rs`), and the clinical orchestration: `patient/` (search, registration, compare), `chart_link*` (link/unlink), `duplicate_*`, `john_doe.rs`, `identify.rs`, `medication/`, `note/` (draft, who's-here, sign, quarantine commit, discard, read), `sensitivity/`, `safety.rs`, `shred.rs`. |
| **`cairn-medication-view`** | Pure medication read model + the one definition of what a sign-off attests (`sign_off_targets`), shared by node and window. `--features fixtures` builds test fixtures. |
| **`cairn-patient-search`** | Pure patient-search read model: what a candidate *is*, the ranking, what a registration attests. |
| **`cairn-db-keepalive`** | Dead-peer TCP settings for a Postgres connection, shared by `cairn-node` and `cairn-sync`. |

`crates/cairn-node/tests/` is the best catalogue of what the node can do — each file is an end-to-end
scenario, named for the behaviour it pins (`note_sign.rs`, `vault_floor.rs`,
`medication_withhold.rs`, `restore_one_event_id_one_body.rs`, …). Most are database-gated
([Getting started](getting-started.md#database-gated-tests)). A few are **repository guards** rather
than behaviour tests — `paper_parity_plan_section.rs`, `crypto_sink_names_are_genuine.rs`,
`cargo_lockfiles_tracked.rs`, `db_gate_actually_ran.rs` — and fail when a house rule is broken.

`crates/cairn-node/results/` holds measured results and the DR-restore timing `RUNBOOK.md`.

---

## `extensions/cairn_pgx` — in-database Rust

The small Rust surface that runs **inside** PostgreSQL via [pgrx](https://github.com/pgcentralfoundation/pgrx):
`cairn_verify` (the signature gate the floor calls), blob verification, the DEK-wrap/unseal primitives
behind born-sealed bodies, `cairn_vault_unseal`, and a few helpers. Default feature targets **PG 18**;
a `pg16` feature is retained for a legacy local instance. Built and tested with `cargo pgrx install`
/ `cargo pgrx test`, **not** plain `cargo`. Its version is bumped whenever a migration starts to need
a new function from it.

---

## `cairn-gui/` — the reference window

A **separate cargo workspace**, so a webview stack never enters `cairn-node`'s dependency tree. The
dependency runs one way: `cairn-gui/*` may depend on `crates/*`, never the reverse.

| Crate | Role |
|---|---|
| **`cairn-gui-tauri`** | **The window binary** (Tauri 2). `src/` holds the Rust commands and every sentence the clinician reads; `src-ui/` the hand-written, unbundled JavaScript + HTML/CSS (no npm). Its `README.md` is required reading before touching the frontend, and `results/RUNBOOK.md` holds the human paper-parity and accessibility passes, one numbered section per slice. |
| **`cairn-gui-funnel`** | The §5.3/§5.8 search-before-create funnel's rules, as pure functions. |
| **`cairn-gui-live`** | The funnel's ports backed by a real node's database (DB-gated tests). |
| **`cairn-gui-data`** | The data-port traits the window reads through, and the `--mock` fixture implementation. |
| **`cairn-gui-tabs/*`** | Per-tab view models; `cairn-gui-tab-medications` owns every medication display decision and its accessibility contract. |
| **`cairn-gui-tab`, `cairn-gui-shell`, `cairn-gui-manifest`** | The framework-agnostic tab/pane/routing state and site-manifest model that survived the iced retirement ([Spike 0004](../spikes/0004-iced-reference-ui-viability.md)); tested, partly not yet wired into the window. |

---

## `db/` — the in-DB enforcement floor (the heart)

Numbered SQL migrations, loaded in order by `cairn-node` on connect (the `SCHEMA` array in
`crates/cairn-node/src/db.rs`). This is **layer 2**, the unbypassable safety floor. Roughly, by range:

| File(s) | What it establishes |
|---|---|
| `001`–`006` | the signed-event envelope (`event_log`), projection scaffolding, blob references, the actor registry, **`submit_event`** (the strict door), recall |
| `007`–`009` | the node plane: `node_event`, admission, runtime role, supersede + restore doors (`008` is a spike-only surrogate-key experiment the product loader skips) |
| `010`–`017` | demographics (identifiers, fields, names, sex/gender, address), the globalised twin, the **match hard-veto**, the advisory match-proposal worklist |
| `018`–`030` | the identity algebra (link, dispute, identify, repudiate, evidence), **`apply_remote_event`** (the remote door), the sync quarantine pens, blob verification, HLC collision log, John Doe |
| `031`–`043` | `clinical.medication` (assert, dose, reconciliation, attestation, coding), **born-sealed bodies** (`037`), the schema stamp (`038`), the projection registry, clock-confidence grade, deferred re-adjudication |
| `044`–`060` | gesture timing, **registration + search** and the registration-first rule, the sensitivity stream and safety projection, restore doors, the substitution guard, person charts (linked sets), match pending, withhold + cessation reasons |
| `061`–`069` | the **human key vault**, the window's own role, enrollers, the `clinical.note` stream, note drafts and their parts, the quarantine commit, the audited discard |
| `tests/*.sql` | SQL-level mirrors of the floor's rules — run **only** via `scripts/run-db-sql-tests.sh` (they are destructive) |
| `bench/` | micro-benchmarks |

> [!NOTE]
> The migration *numbers* are stable; the ranges above are orientation, not a contract. The
> authoritative list is the `SCHEMA` array, and the current generation is `SCHEMA_GENERATION`.
> **Every migration replays whenever the schema changes**, so a later file cannot safely redefine
> something an earlier file creates — the earlier file would undo it on the next replay. That is why
> some slices edit an older migration *in place*; [Durable rules](durable-rules.md) records each such
> case and its pin. Read the block for any file you are about to touch.

---

## `matcher/` — the advisory patient-matcher (`cairn-matcher`, Python)

A `uv` project; the *fit-for-purpose* tier. **Pure scoring core with zero runtime dependencies**,
plus an optional DB-bearing `pipeline` extra.

- `src/cairn_matcher/` — the pure core: `agreement.py`, `comparators.py`, `records.py`,
  `scoring.py` (the Fellegi–Sunter combiner), `orchestrator.py`; `cli.py` (the `cairn-matcher`
  entry point); `eval/` (evaluation tooling).
- `src/cairn_matcher/pipeline/` — the IO-bearing pipeline: `adapter.py`, `alias.py`, `banding.py`,
  `blocking.py`, `blocking_sql.py` (all pure), `db.py` (the one module that imports `psycopg` at
  import time), `runner.py`, `sweep.py`, and the commit-time worker — `targeted.py`, `judged.py`,
  `queue_db.py`, `worker.py`, `worker_plan.py`, `watch.py` (run as `cairn-matcher watch`; see
  [Running the duplicate check](running-the-duplicate-check.md)).
- `tests/` — pure tests (`uv run pytest`) + DB-gated integration tests.

The matcher **only scores and proposes**. The hard veto that can actually stop a match is in
`db/016_match_veto.sql` — in the database, where the blast radius demands it.

---

## `scripts/`

| Script | Use |
|---|---|
| `run-db-gated-tests.sh` | **The** local database gate: SQL mirrors + full workspace `cargo test` + `cairn-gui-live`, env baked in. |
| `run-db-sql-tests.sh` | The `db/tests/*.sql` mirrors in a throwaway database. |
| `pg-target.sh` | Finds a running PG ≥ 18 cluster for the two scripts above, and refuses one too old. |
| `codeql-alerts.sh` | Read the open CodeQL alerts (read-only). |
| `check_closing_keywords.py`, `collect_pr_text.sh` | The closing-keyword guard CI runs on every PR. |
| `measure_*.py`, `bench-reproject.sh` | Measurement rigs (DR restore, patient search, prompt truncation, reprojection). |
| `techdebt-loop.sh` | The tech-debt elimination driver (one fresh session per issue). |
| `mutations/` | Throwaway per-PR mutation-testing harnesses, kept so a plan's mutation ledger is reproducible. |
| `tests/` | Tests for the scripts themselves (CI runs them). |

---

## `docs/` — spec, decisions, principles, and this manual

| Subtree | What it is | Authority |
|---|---|---|
| `docs/principles/` | mission & governance (`GOVERNANCE.md`, `STEWARDSHIP-OF-THE-NAME.md`) | **highest** |
| `docs/spec/` | the canonical architecture spec, one file per aspect; entry point `index.md` | **canonical** (the *what*) |
| `docs/spec/decisions/` | the **ADR log** — numbered, dated, **immutable**; a reversal is a new ADR | **canonical** (the *why*) |
| `docs/spikes/` | build-prep records: what we tried, on what, what we learned | reference |
| `docs/case-studies/` | clinical case-mining: real failure modes tested against the primitives | reference |
| `docs/ecosystem/` | dependency, framework and design-lesson evaluations | reference |
| `docs/code_reviews/` | whole-project review reports | reference |
| `docs/essays/` | long-form narrative pieces (published on the site) | narrative |
| `docs/developers/` | **this manual**, including [Durable rules](durable-rules.md) | practical guide |
| `docs/ROADMAP.md` | the foundation build order + progress | disposable scaffolding |
| `docs/HANDOVER.md` | the current-session snapshot (**read first each session**) | disposable scaffolding |
| `docs/superpowers/` | per-slice `specs/` (designs) + `plans/` (TDD task lists) | working artifacts |

> [!IMPORTANT]
> When sources disagree, authority wins in this order: **principles → spec → ADRs (for the *why*) →
> everything else.** `HANDOVER.md`, `ROADMAP.md`, and `superpowers/` are disposable working
> scaffolding — never a source of truth. `HANDOVER.md`, `superpowers/`, and `requirements.txt` are
> **excluded from the built docs site**, so do not link to them with Markdown links (it breaks the
> strict build) — refer to them by path in prose instead.

---

## `poc/`, `scratch/`, `web/`, `packaging/`, `assets/`

- **`poc/`** — **frozen** proof-of-concept spikes: `walking-skeleton` (the original event + WAN-sync
  skeleton), `pg-android-kit` (Postgres-on-Android), `replication-failover`. Read-only historical
  reference — *do not* build product code here; the proven primitives have graduated into `crates/`
  and `db/`.
- **`scratch/`** — working sketches and notes (e.g. UI sketches). Reference only.
- **`web/`** — the public landing page, deployed separately (Cloudflare Pages) from the docs site.
- **`packaging/`** — placeholder packages that reserve the project's names on crates.io, npm and PyPI.
- **`assets/`** — shared brand/image assets.

---

## Next

**[Codebase tour →](codebase-tour.md)** — a guided reading path through the real source.
