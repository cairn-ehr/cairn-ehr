# 10. Technology Candidates (all AGPL-3.0-compatible)

Selection governed by [§9](language-substrate.md). "Substrate" reflects the [§9.1](language-substrate.md#91-selection-rule-by-defect-blast-radius) bucket; specific frameworks are illustrative, not fixed.

| Role | Candidate / reference | Substrate bucket | License | Note |
|---|---|---|---|---|
| Database | PostgreSQL ≥ 18 | — (foundation) | PostgreSQL (permissive) | uuidv7(), async I/O, logical replication |
| Change capture | Logical decoding (`pgoutput` / wal2json) | safety / in-database | PostgreSQL / BSD | Core primitive |
| Sync daemon (transport/scope/apply) | (custom) | **safety → Rust** | — | Thin; ships & applies, no merge logic ([§6.1](sync.md#61-mechanism), [§9.4](language-substrate.md#94-merge-projection-boundary-fat-postgres-thin-rust-daemon)) |
| Identity algebra & projections | (custom) | **safety → in-database** | — | [§5.7](identity.md#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable), [§9.4](language-substrate.md#94-merge-projection-boundary-fat-postgres-thin-rust-daemon); trigger-maintained projections; PL/pgSQL default, pgrx escape hatch |
| In-database Rust (escape hatch) | pgrx | **safety → in-database** | MIT / Apache-2.0 | Rust Postgres functions for hot/complex projections; stays in-DB, unbypassable ([§9.4](language-substrate.md#94-merge-projection-boundary-fat-postgres-thin-rust-daemon), [ADR-0002](decisions/0002-in-database-rust-pgrx-escape-hatch.md)) |
| Multi-master reference | pgactive | reference only | Apache-2.0 | Borrow decoding patterns; **not** a dependency ([§6.1](sync.md#61-mechanism)) |
| Heterogeneous sync reference | SymmetricDS | reference only | GPL-3.0 | Borrow store-and-forward/sneakernet patterns; not a dependency ([§6.1](sync.md#61-mechanism)) |
| Thin-client store (optional) | PGlite | — | Apache-2.0 | Postgres-in-WASM for tablet/web *thin clients* only — transient buffering, **not** an autonomous edge node ([§2](topology.md)) |
| Read-path sync reference | ElectricSQL | — | Apache-2.0 | Shape-based partial replication patterns |
| Record linkage / matcher | Splink + custom | **fit-for-purpose → Python** | MIT | Advisory; Fellegi–Sunter; ML ecosystem |
| FHIR façade / interop | HAPI FHIR / fhir.resources | fit-for-purpose | Apache-2.0 / BSD | Interface, not merge core |
| Integration glue / tooling / UI backends | (various) | fit-for-purpose | permissive | Iteration speed prioritized |
