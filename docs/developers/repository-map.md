# Repository map

What every top-level directory is, whether it is **load-bearing** (active product/spec) or
**reference** (frozen/historical), and where to look first. For the *why* behind the structure, read
[Architecture for developers](architecture-for-developers.md) first.

```
cairn-ehr/
├── crates/          Rust workspace — the thin daemon + the wire core      [load-bearing]
├── extensions/      cairn_pgx — in-database Rust extension (pgrx)         [load-bearing]
├── db/              numbered SQL migrations + SQL tests — the in-DB floor [load-bearing]
├── matcher/         cairn-matcher — advisory patient-matcher (Python)     [load-bearing]
├── docs/            spec, ADRs, principles, spikes, and this manual       [load-bearing]
├── web/             the public landing page (separate from the docs site) [load-bearing]
├── packaging/       packaging artifacts                                   [supporting]
├── poc/             frozen proof-of-concept spikes                        [reference only]
├── scratch/         working sketches / notes                             [reference only]
├── assets/          shared image/brand assets                            [supporting]
├── Cargo.toml       Rust workspace manifest (excludes extensions/)
├── CLAUDE.md        repo guidance (also useful orientation for humans)
├── CONTRIBUTING.md  short pointer to docs/principles/GOVERNANCE.md
├── README.md        mission + the twelve founding principles
└── mkdocs.yml       docs-site config (nav, theme, strict-build settings)
```

---

## `crates/` — the Rust workspace (thin daemon + wire core)

The workspace is declared in the root `Cargo.toml` (edition 2021, Rust ≥ 1.74, AGPL-3.0-only).
**`extensions/cairn_pgx` is deliberately excluded** — it builds with `cargo pgrx`, not plain `cargo`.

| Crate | Role | Key source |
|---|---|---|
| **`cairn-event`** | **Wire core (layer 1), safety-critical.** Event serialization (canonical CBOR), COSE_Sign1/Ed25519 sign + verify, multihash content-addressing, and the demographics event builders. Kept small and reviewer-legible on purpose. | `src/lib.rs`, `src/demographics.rs` |
| **`cairn-sync`** | The set-union synchronization engine. | `src/main.rs` |
| **`cairn-node`** | **The thin federation daemon.** Sealed Ed25519 keystore, pairing/mTLS transport, set-union `node_event` sync, backup/restore/supersede durability, and the demographics integration tests. Its CLI is the main developer entry point. | `src/{lib,main,keystore,seal,pairing,transport,sync,backup,restore,localstate,medium,identity,db,fsio}.rs`; `tests/*.rs` |

`cairn-node`'s `tests/` directory is the best catalogue of what the node can do — each file is an
end-to-end scenario (`demographics*.rs`, `match_veto.rs`, `floor_enforced.rs`, `federation.rs`,
`pairing.rs`, `restore.rs`, `backup.rs`, `keystore_seal.rs`, `sync_watermark.rs`, `status.rs`, …).
Many are database-gated (see [Getting started](getting-started.md#database-gated-tests)).

---

## `extensions/cairn_pgx` — in-database Rust

The small Rust surface that runs **inside** PostgreSQL via [pgrx](https://github.com/pgcentralfoundation/pgrx)
— currently the `cairn_verify` event-signature gate the in-DB floor calls. Default feature targets
**PG 18** (pgrx `=0.18.1`); a `pg16` feature is retained for the legacy local instance. Built and
tested with `cargo pgrx install` / `cargo pgrx test`, **not** plain `cargo`.

- `src/lib.rs` — the extension entry point.
- `cairn_pgx.control` — the PG extension control file.

---

## `db/` — the in-DB enforcement floor (the heart)

Numbered SQL migrations that load in order. This is **layer 2**, the unbypassable safety floor, and
where the clinical product is being built slice by slice. Roughly:

| File(s) | What it establishes |
|---|---|
| `001_envelope.sql` | the append-only signed-event envelope (`event_log`) |
| `002_projection.sql` | projection scaffolding |
| `003_blobs.sql` | the content-addressed blob references |
| `004_actors.sql` | the actor registry |
| `005_submit.sql` | **`submit_event`** — the single validated write door |
| `006_recall.sql` | recall / suppression overlay |
| `007_node_federation.sql` | `node_event`, the federation admission gate, the runtime-role floor |
| `008_surrogate_projection.sql` | node-local `bigint` surrogate keys (dual-identifier discipline) |
| `009_node_supersede_and_restore.sql` | node `supersede` + restore door |
| `010`–`015` `…demographics…` / `…twin…` | the demographics slices (identifiers, fields, names, sex/gender, address) + the globalised legibility twin |
| `016_match_veto.sql` | the **in-DB hard-veto floor** for patient matching (safety-critical) |
| `017_match_proposal.sql` | the **advisory** match-proposal worklist (not a safety gate) |
| `tests/*.sql` | SQL-level tests for the floor (`004`–`009`) |
| `bench/` | micro-benchmarks (e.g. the surrogate-key B5 measurement) |

> [!NOTE]
> The migration *numbers* are stable; the *exact* file set and what the latest slice added move with
> the build. The authoritative current state is the SCHEMA array in `cairn-node` and
> `docs/HANDOVER.md`, not this table — treat the table as orientation, not a contract.

---

## `matcher/` — the advisory patient-matcher (`cairn-matcher`, Python)

A `uv` project; the first Python component and the *fit-for-purpose* tier. **Pure scoring core with
zero runtime dependencies**, plus an optional DB-bearing `pipeline` extra.

- `src/cairn_matcher/` — the pure core: `agreement.py`, `comparators.py`, `records.py`,
  `scoring.py` (the Fellegi–Sunter combiner), `orchestrator.py`.
- `src/cairn_matcher/pipeline/` — the IO-bearing pipeline (needs `psycopg`): `adapter.py`,
  `banding.py` (both pure), `db.py`, `runner.py`, `sweep.py`.
- `tests/` — pure tests (`uv run pytest`) + DB-gated integration tests.
- `README.md` — the matcher's own developer notes; `pyproject.toml` — the project + the `pipeline` extra.

The matcher **only scores and proposes**. The hard veto that can actually stop a match is in
`db/016_match_veto.sql` — in the database, where the blast radius demands it.

---

## `docs/` — spec, decisions, principles, and this manual

| Subtree | What it is | Authority |
|---|---|---|
| `docs/spec/` | the canonical architecture spec, one file per aspect; entry point `index.md` | **canonical** (the *what*) |
| `docs/spec/decisions/` | the **ADR log** — numbered, dated, **immutable**; a reversal is a new ADR | **canonical** (the *why*) |
| `docs/principles/` | mission & governance (`GOVERNANCE.md`, `STEWARDSHIP-OF-THE-NAME.md`) | **highest** |
| `docs/spikes/` | build-prep records: what we tried, on what, what we learned | reference |
| `docs/ecosystem/` | plugin/dependency evaluations | reference |
| `docs/essays/` | long-form narrative pieces (published on the site) | narrative |
| `docs/developers/` | **this manual** | practical guide |
| `docs/ROADMAP.md` | the foundation build order + progress | disposable scaffolding |
| `docs/HANDOVER.md` | the current-session snapshot (**read first each session**) | disposable scaffolding |
| `docs/superpowers/` | per-task `specs/` + `plans/` from the SDD workflow | working artifacts |

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
- **`packaging/`** — packaging artifacts.
- **`assets/`** — shared brand/image assets.

---

## Next

**[Codebase tour →](codebase-tour.md)** — a guided reading path through the real source.
