# Glossary

Cairn-specific terms a newcomer meets in the code and docs, in plain language. For the authoritative
treatment of any concept, follow the link into the [spec](../spec/index.md) or the
[ADR log](../spec/decisions/README.md).

---

### Append-only event log
The core store. Clinical content is recorded as **immutable, signed events** that are only ever
*added*, never updated or deleted in place. Corrections and identity repairs are *new* events
referencing the originals. See [principle 1](../spec/index.md) and the [data model](../spec/data-model.md).

### Projection
A queryable "current state" table (e.g. `patient_identifier`, `patient_name`, `patient_demographic`)
that is **derived** from the event log by triggers/functions. Projections are caches — rebuildable
from the log at any time. You never write a projection directly; you submit an event and the
projection updates.

### The floor (in-DB enforcement floor)
The safety-critical validation that lives **inside PostgreSQL** — the validated `submit_event` door,
row-level security, constraints, and projection triggers (`db/*.sql`, layer 2). It is **unbypassable**:
even a client connecting with raw SQL cannot break the record. Proven against a hostile agent in
[Spike 0002](../spikes/0002-advisory-actor-write-contract.md).
See [Architecture for developers](architecture-for-developers.md#3-the-four-layer-model).

### `submit_event`
The single validated write door into the event log (`db/005_submit.sql`). It verifies the signature
(via the in-DB `cairn_verify`), runs the per-type structural floor checks, and appends to `event_log`.
There is no other write path. See [ADR-0022](../spec/decisions/0022-validated-submit-surface-the-write-path.md).

### Fat Postgres, thin daemon
The architecture's central engineering choice: safety-critical logic lives **in the database**, and
the Rust daemon stays thin (crypto, transport, orchestration). The integration boundary *is* the
PostgreSQL boundary. See [ADR-0001](../spec/decisions/0001-fat-postgres-thin-daemon.md).

### Wire core
Layer 1 — the signed event itself (serialization, signature, content-addressing, the identity/actor
algebras). **Nothing above it sits on the inter-node path**, so any node interoperates with any other.
`crates/cairn-event`. See [ADR-0021](../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md).

### HLC (Hybrid Logical Clock)
The ordering mechanism for events. It combines wall-clock time with a logical counter so **causal
order survives clock skew** across offline nodes. Events are ordered by HLC, not by raw wall-clock.

### `t_recorded` vs. `t_effective`
The bitemporal time model. `t_recorded` is the objective record time (HLC-bounded — the ceiling);
`t_effective` is the *asserted*, freely-backdatable clinical time (the displayed claim). Clashes are
**flagged, never auto-resolved.** See [ADR-0003](../spec/decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md)
and [principle 4](../spec/index.md).

### Legibility twin
A mandatory, signed, mechanically-derived **plaintext human-readable rendering** carried inside every
event's body (`EventBody.plaintext_twin`) — informally, "the twin". It keeps an event readable forever — even on a node whose
schema has moved on or never understood the event's type — and is the full-text/RAG substrate. An
**authored** twin (built by the event builder) is preferred; non-demographic types **degrade
honestly** to a flagged derived skeleton when one is absent. Principle 11; see
[ADR-0012](../spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md),
[ADR-0034](../spec/decisions/0034-demographic-legibility-twin.md), and
[ADR-0039](../spec/decisions/0039-globalise-authored-legibility-twin.md).

### Set-union sync
Synchronizing two nodes is a **set union** of their immutable, content-addressed events plus a small,
explicitly enumerated set of merge policies — never a dangerous field-level merge. Applying the same
event twice, or learning it from two peers, converges. See the [sync spec](../spec/sync.md) and
[ADR-0004](../spec/decisions/0004-dynamic-sync-scope-prefetch-not-authority.md).

### Identity is a claim, never a fact / *never merge, always link*
Patient identity is an append-only stream of link/unlink/reattribute/repudiate/identify/dispute
events (the closed **identity event algebra**, [§5.7](../spec/identity.md)). Patient UUIDs are
**immortal**; identity errors are repaired by an auditable event with no data loss. You never merge
two patient records or erase one — you *link* or *overlay*. [Principle 2](../spec/index.md).

### Slice
A vertical, end-to-end increment of the clinical product (e.g. "demographics slice 1 = patient
identifiers"). Each slice goes brainstorm → spec → plan → TDD and reuses the event/floor/projection
spine. The current slice is tracked in `docs/HANDOVER.md`.

### Actor
A registered, version-pinned identity that can author or attest events — a human, a device, or an
**advisory agent**. Signing proves origin/integrity; **attestation** confers responsibility, and the
two are separable. See [ADR-0011](../spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md),
[ADR-0007](../spec/decisions/0007-authorship-and-accountability.md), and (for AI agents)
[ADR-0029](../spec/decisions/0029-skill-epoch-as-pinned-actor-determinant.md) /
[ADR-0030](../spec/decisions/0030-advisory-actor-integration-contract.md).

### Veto (match veto)
The **in-DB, safety-critical** check that can stop a patient match: `cairn_match_veto`
(`db/016_match_veto.sql`) returns the closed hard-veto set (same-system identifier mismatch,
verified-DOB clash, verified-sex-at-birth clash). A veto can only ever **withhold an auto-link or
force human review — never auto-reject.** Distinct from the advisory *scorer*. See
[ADR-0014](../spec/decisions/0014-locale-pluggable-matcher-comparators.md).

### Matcher (advisory) / scoring / banding / proposal
The **advisory** patient-matcher (`matcher/`, Python). The pure **scorer** (Fellegi–Sunter) produces
an explainable `MatchScore`; **banding** turns a score + veto findings into `auto_candidate` /
`review` / nothing; a **proposal** is an advisory row in `match_proposal` (`db/017`) for a human to
act on. The matcher never links or rejects on its own — it only proposes.

### Provenance
Where an assertion came from on a graded ladder (e.g. patient-stated → document-verified →
fact-proven). Several demographic fields resolve their display "winner" by provenance (DOB, sex-at-birth,
administrative-sex), while others resolve by recency (names, gender-identity, address). The choice per
field is a deliberate ADR-backed clinical decision. See [demographics](../spec/demographics.md).

### Erasure = key-custody redistribution (crypto-shred)
In an append-only system, "deletion" is implemented as **crypto-shredding** — redistributing or
destroying key custody for an encryption-capable body slot — exposed as a policy-neutral severity
ladder. Deletion is best-effort and declared, never guaranteed. See
[ADR-0005](../spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md), principle 9.

### Fractal topology
One codebase runs at every tier — workstation → department → facility → region → nation. A node's
role is **configuration, not a different product.** Validated down to the phone tier in
[Spike 0003](../spikes/0003-postgres-on-android-bionic-node.md).

### Node / federation / peering
A **node** is one Cairn instance (its own PostgreSQL + the `cairn-node` daemon). Nodes **federate** by
**peering** — exchanging signed pairing offers, pinning each other in an mTLS trust set, and syncing
`node_event` logs by set-union. See [ADR-0017](../spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md).

### Spike
A time-boxed proof-of-concept that answers a viability question before committing to a build (the
walking skeleton, the advisory-actor write contract, Postgres-on-Android). Recorded under
`docs/spikes/`. Proven primitives graduate into `crates/` and `db/`; the spike code in `poc/` is then
frozen reference.

### ADR (Architecture Decision Record)
A numbered, dated, **immutable** record of a load-bearing decision (context → decision →
consequences), under `docs/spec/decisions/`. A reversal is a *new superseding ADR*, never an edit.
**Read the relevant ADR before reopening a settled question.**

### pgrx / `cairn_pgx`
[pgrx](https://github.com/pgcentralfoundation/pgrx) is the framework for writing PostgreSQL extensions
in Rust. `cairn_pgx` (`extensions/`) is Cairn's in-database Rust surface — currently the
`cairn_verify` signature gate the floor calls. Built with `cargo pgrx`, not plain `cargo`.

### `cairn_pgx` runtime role / `db_floor ENFORCED`
The daemon should connect as an **unprivileged** login role granted the `cairn_node` (NOLOGIN) role,
so the in-DB floor genuinely binds it — a raw `INSERT` is then denied (SQLSTATE 42501) while
`submit_event` still works. `cairn-node status` reports `db_floor ENFORCED` vs. `BYPASSABLE` so you
can tell which path you're on.
