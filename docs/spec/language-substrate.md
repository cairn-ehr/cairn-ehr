# 9. Language & Substrate Selection Principle

The spec deliberately does **not** fix implementation languages per component. It fixes the *rule* by which they are selected, so the choice is auditable and survives changing tooling.

## 9.1 Selection rule — by defect blast radius
**The cost of a defect dictates how much the language/substrate must prevent defects at compile time or by construction.**

- **Safety-critical bucket** (a defect can silently corrupt the record, mis-merge patients, leak data, or crash an unattended node): implement in **Rust or in-database (SQL / PL-pgSQL / constraints)**. These make whole error classes unrepresentable — memory safety, exhaustive sum-type matching, no runtime metaprogramming, or database-enforced invariants that no buggy caller can bypass. Members: the sync/merge engine, the identity event algebra and projections ([§5.7](identity.md#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable)), HLC ordering, coherence checks, audit-log integrity, access-control enforcement.
- **Fit-for-purpose bucket** (a defect is caught immediately, is advisory, or is cosmetic): optimize for iteration speed and ecosystem. Members: probabilistic matcher / record linkage (advisory — proposes candidates, humans/policy decide; Python's ML ecosystem is decisive here), FHIR façade and integration glue, tooling, UI backends.

In-database is a **first-class member of the safety bucket, not a footnote**: for some merge/projection logic, a constraint or PL/pgSQL routine next to the data is safer and more auditable than any application-layer code in any language, because the invariant is enforced unconditionally and cannot be bypassed.

## 9.2 Primary quality metric — reviewer-legibility
With AI-assisted development, the binding constraint shifts from *authorship fluency* to *specification + review*: comparable results are achievable with far smaller competent teams, and individual per-language coding skill matters much less between design spec and final review. Therefore:
- The artifacts that gate quality are the **specification** and the **review**, not the typing.
- Safety-critical layers are optimized for **auditability / reviewer-legibility**, even over authorship speed. Rust ("the types document the invariants") and in-database ("the logic sits next to the data it governs") both score high on this axis.
- Concentrating safety-critical logic in a small, well-bounded set of restrictive-language components **shrinks the audited surface** — the part needing the most rigorous review is also the smallest. This directly serves the small-team reality.

## 9.3 Integration boundary
Polyglot is expected ("horses for courses"). To avoid fragile coupling, **the language boundary is the database boundary**: each component talks to its node's PostgreSQL; Postgres is the integration substrate. (E.g. the Python matcher writes link-candidate events; the Rust/in-database core consumes them — loose coupling, no FFI.)

## 9.4 Merge / projection boundary — fat Postgres, thin Rust daemon
> Resolves former open question §11.11 — see [ADR-0001](decisions/0001-fat-postgres-thin-daemon.md).

The [§9.1](#91-selection-rule-by-defect-blast-radius) safety bucket is divided concretely. Because every autonomous node runs full PostgreSQL ([§2](topology.md)), in-database logic runs *everywhere* — so safety-critical merge/projection logic is placed where it is **unbypassable and reviewer-legible**: next to the data.

**In Postgres** (constraints / PL-pgSQL / triggers — unbypassable, on every node incl. Pi):
- **Append-only enforcement:** the application role is granted INSERT/SELECT only on event tables; a trigger raises on any UPDATE/DELETE as defense-in-depth.
- **Idempotent apply:** `INSERT … ON CONFLICT (uuid) DO NOTHING` makes sync a clean set-union; a PK collision whose content hash differs is routed to the repair/quarantine queue, never silently merged ([data-model §3.2](data-model.md#32-identity-time) backstop).
- **Structural invariants:** HLC monotonicity / causal-ordering checks, the closed `event_type` enum, FK integrity.
- **The identity event algebra** (deterministic application of link / unlink / reattribute / identify / repudiate / dispute, [§5.7](identity.md#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable)) and **all projections**: the chart projection (union of member-UUID event streams), the [§3.3](data-model.md#33-mutable-non-demographic-state) mutable-list unions, the [§4.2](demographics.md#42-per-field-projection-policy) demographic projection, the golden-identity connected-component ([§5.1](identity.md#51-linkage-layer-never-merge-always-link)), the coherence check ([§5.2](identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split), demotes to *under-review* on a [§4.2](demographics.md#42-per-field-projection-policy) conflict), the chart trust states ([§5.7](identity.md#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable)), and the reattribution overlay ([§5.5](identity.md#55-reattribution-one-primitive-tiered-workflows)).
- **Projections are trigger-maintained incremental tables** (`AFTER INSERT` only — the INSERT-only log means no update/delete maintenance path), **not** periodic `REFRESH MATERIALIZED VIEW`; this keeps per-write cost low on Pi-class hardware.

**In Rust** (the thin sync daemon, [§6.1](sync.md#61-mechanism) — *ships and applies, never decides*): logical-decoding consumer, scope-predicate evaluation, the [§6.1](sync.md#61-mechanism) priority queue, resumable mTLS/HTTPS transport and store-and-forward, idempotent apply. **No merge logic.**

**In Python** (advisory only, [§5.2](identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split) / [§10](technology.md)): the probabilistic matcher *proposes* link candidates by writing candidate events; it never decides. Authoritative application and projection are in-DB. The Python↔in-DB seam is exactly the [§9.3](#93-integration-boundary) database boundary.

> [!WARNING]
> **The load-bearing bet:** that in-DB projections stay cheap enough on Pi-class hardware to keep
> chart reads local and fast (the [§1.2](vision.md#12-the-paper-parity-test-normative) paper-parity
> floor). A Pi serves only a handful of workstations with little concurrency, so the risk is
> single-operation latency on a weak CPU + SD/USB storage, not throughput. The first implementation
> spike is a Pi benchmark that validates or falsifies it.

**Escape hatch — an in-database escalation ladder, never leaving Postgres** (see [ADR-0002](decisions/0002-in-database-rust-pgrx-escape-hatch.md)):

1. **PL/pgSQL** — the default; most legible for set-oriented projection logic; no build step.
2. **Rust via pgrx (in-database)** — when a function is hot or algorithmically complex (the identity connected-component is the prime candidate). Compiled-Rust speed and type-safety while the function **stays a Postgres function** — next to the data, unbypassable, invoked by the same triggers, inside the [§9.3](#93-integration-boundary) database boundary. So "Rust" and "in-database" are one bucket, not two.
3. **External Rust** — only if logic genuinely cannot be a database function (not expected for projections).

The decision is per-projection and criteria-gated (Pi single-op latency, reviewer-legibility, bypassability), not an upfront blanket split. The thin sync daemon still carries no merge logic.
