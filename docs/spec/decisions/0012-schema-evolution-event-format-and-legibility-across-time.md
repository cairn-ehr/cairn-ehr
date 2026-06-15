# ADR-0012 — Schema evolution, event-format migration across the offline fleet, and legibility across time

- **Status:** Accepted
- **Date:** 2026-06-15

## Context

Former open question §11.4 — *schema migrations across a fleet of offline nodes; the version-skew
tolerance window; forward-compatible event formats* — is the last of the original §11 clusters that is
**load-bearing before implementation begins**. Attachments (§11.6) and locale-pluggable matcher comparators
(§11.7) are self-contained subsystems addable later; schema evolution is not, because it **constrains the
event envelope itself**, and the envelope is the most foundational and least-reversible thing in Cairn. Just
as `t_effective` ([§3.6](../data-model.md#36-bitemporal-event-time-recording-time-vs-effective-time)) and the
encryption-capable body slot ([§3.5](../data-model.md#35-event-storage-model-hybrid-envelope),
[ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md)) had to be *reserved from day one* because they
cannot be retrofitted onto an append-only log, the event-format-evolution contract must exist before the first
production clinical event is written. One event written without a version discriminator and a forward-compat
rule is already an un-migratable corpus.

The forces, sharpened by the offline-first / fractal / append-only commitments:

- **The append-only log forbids the classic migration.** A historical event signed under schema vN must remain
  **byte-identical forever** — any rewrite breaks its signature and would be resurrected by set-union sync from
  a sibling, backup, or WORM archive. *"Migrating data"* in the `ALTER TABLE … UPDATE` sense is structurally
  unavailable for the log ([§3.1](../data-model.md#31-append-only-clinical-event-log-source-of-truth),
  [§3.8](../data-model.md#38-erasure-and-key-custody)).
- **Version skew is permanent and unbounded.** A node offline for three years, or a resource-constrained,
  bandwidth-poor site that may *never* upgrade, is a designed-for case ([vision §1.4](../vision.md)). At any
  moment a node must read events authored across a *range* of schema versions — older *and* newer than its own
  code. A node running v1 will receive a v9 event and must store it, forward it, and display it safely without
  ever having seen the v9 format.
- **"Database migration" hides two different problems.** (a) **Local Postgres DDL / projection migration** on a
  single node — largely *already solved* by Cairn's own architecture, because projections are rebuildable and
  never synced ([§3.1](../data-model.md#31-append-only-clinical-event-log-source-of-truth)), so a bad projection
  schema is recovered by drop-and-rebuild-from-log. (b) **Event-format evolution across time and the fleet** —
  the hard, distinctive, safety-critical problem.
- **Much functionality runs as Postgres extensions, and the binary must travel with the migration.** A DDL change
  and the Rust/pgrx extension that implements logic over that schema
  ([ADR-0002](0002-in-database-rust-pgrx-escape-hatch.md)) are one versioned unit. But an extension is *native,
  architecture-specific* code (ARM for a Pi, x86-64 for a server, pinned to a Postgres major) that runs inside
  the database — it cannot be shipped like data over the clinical mesh without opening a remote-code-execution
  channel into every node (a direct violation of [principle 8](../index.md#founding-principles-the-lens-for-every-decision)).
- **A stuck-old node must keep a newer event *legible*, not merely stored.** The deeper requirement the case
  surfaced: a node generations behind must still be able to *read* a v9 event as a clinician reads a progress
  note, forward it intact, and preserve it for a future proper import — the property paper always had (a note
  from decades ago needs no "version" to be read) and that digital records routinely lose.

## Decision

Schema evolution is **the founding principles applied to the schema itself**. Resolved across two deliberately
separate planes plus a new founding principle. Canonical homes: event-format invariants
[data-model §3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin); the two planes
and lossless passthrough [sync §6.5](../sync.md#65-schema-evolution-two-planes-and-lossless-forwarding); the
distribution plane [security §7.6](../security.md#76-the-software-distribution-plane-signed-releases-and-extension-load);
the legibility ladder's unification with the safety projection
[identity §5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope).

1. **Two planes that run at different speeds.** The **sync plane** carries signed, immutable clinical events —
   set-union, AP, tolerant of unbounded skew, and **never executable code**. The **distribution plane** carries
   code/DDL/extensions — per-node, per-architecture, with a different trust model (reproducible builds, releases
   signed against a steward key, verified before install) and offline/sneakernet delivery. The decoupling is the
   whole game: **the extension/schema version at node X only ever has to match node X's own schema, never the
   version of events arriving from elsewhere.** Event forward-compatibility is precisely what lets the two planes
   move independently, which dissolves any "lockstep fleet upgrade" requirement.

2. **Four day-one, can't-retrofit event-format essentials** (the sync plane):
   - **`schema_version` on every event** — the body-format version within its `event_type` family. It is
     deliberately also the future join key into a schema-descriptor registry (see consequence on Rung 1), so
     richer generic rendering can be added later with no envelope change.
   - **A mandatory, signed, mechanically-derived plaintext legibility twin on every event** (see principle 11
     and §3.13). Derived from the body at write-time by code that understands the format, carrying a `rendered-by`
     stamp (schema + renderer version). It is not merely a fallback: it is the version-independent substrate for
     human audit, full-text search, and compact RAG context, and its storage cost is repaid by those.
   - **Lossless passthrough.** A node stores / re-propagates / exports the **original signed bytes untouched** —
     never reject, never drop, never down-convert, never re-serialize. This requires the **signature to cover a
     canonical byte representation stored as such**, not one re-derived from JSONB (JSONB does not preserve key
     order, whitespace, or duplicate keys, so re-serialization would break both signature validity and the
     round-tripping of fields a node does not understand).
   - **Additive-only evolution** — *never erase, always overlay*
     ([principle 1/2](../index.md#founding-principles-the-lens-for-every-decision)) applied to the schema: never
     remove or repurpose a field; never delete or renumber a closed-enum value (`event_type`, the role enum, the
     identity/actor algebras) — only add, and deprecate by overlay. A new constraint may only be one all historical
     events already satisfy, or is scoped going-forward (binds events recorded under schema ≥ X).

3. **The effective rendering is one projection bounded on two axes:**
   `min(what this node can parse, what this node is cleared to see)`. **Version-skew degradation and
   confidentiality degradation are the same mechanism** — a node that cannot parse a v9 *format* is in the same
   position as a node that cannot decrypt a sealed *body*, and both degrade down one ladder: rich structured →
   generic descriptor-driven (Rung 1, deferred) → carried flat plaintext twin (Rung 0, the baseline) → the
   [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope) safety
   projection (for a sealed body) → the partition-honest floor (*"an event of type X, authored by Y, N fields,
   not interpretable on this node"*). **Coarseness varies; existence never disappears** — the §5.9 safety-floor
   invariant, generalized. The **version-skew tolerance window is therefore infinite for custody and best-effort
   for understanding**: there is no point at which a node may refuse or discard an event it does not understand.

4. **The distribution plane: extensions travel *with* migrations, as a signed atomic bundle.** A migration unit
   is `{ DDL (architecture-independent text) + extension binary per architecture × Postgres-major +
   projection-rebuild recipe }`, signed against the steward key, verified before load, installable from offline
   media via an audited ceremony (the same shape as node provisioning, [§3.2](../data-model.md#32-identity-time)).
   Upgrades are **fail-safe (availability beats upgrade,
   [principle 5](../index.md#founding-principles-the-lens-for-every-decision))**: the prior extension is retained
   until the new one is verified healthy (blue-green at the extension level); additive DDL means rollback loses
   nothing; and any writes during a half-applied upgrade are just more append-only events, re-projected once it
   settles. An unattended Pi must never brick. **The difficulty is proportional to native-code surface** —
   PL/pgSQL/SQL migrations are architecture-independent text that ride a trivial channel, and only pgrx forces the
   per-architecture binary plane, so [ADR-0001](0001-fat-postgres-thin-daemon.md)/[ADR-0002](0002-in-database-rust-pgrx-escape-hatch.md)'s
   "keep the native surface small" discipline earns a second payoff: it minimizes migration blast radius.

5. **A new founding principle — 11. Legibility across time.** Every clinical event must remain human-readable for
   as long as it exists, independent of how far the schema or software has since moved. This is paper-parity
   ([principle 3](../index.md#founding-principles-the-lens-for-every-decision)) extended along the *time/version*
   axis — ink on paper from decades past needs no version to be read, and a Cairn event must match that. *Schema
   is versioned data, not privileged structure.* The mechanism is the mandatory mechanically-derived signed
   plaintext twin (essential 2) plus additive-only evolution (essential 4), so the original is never lost and the
   rendering can always be regenerated richer after an upgrade.

6. **Blast radius ([§9](../language-substrate.md)).** **Safety-critical** (in-database/Rust): the
   serialization/signature-canonicalization contract, the lossless-passthrough guarantee, additive-only
   enforcement, and the distribution-plane signature-verification and extension load. **Fit-for-purpose:** all
   renderers (the write-time twin derivation, the generic descriptor renderer), locally-regenerated twins, and
   search/RAG. The one safety-critical **seam** is the write-time body→twin derivation path — *the same seam* as
   the [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope) seal-time
   projection, since legibility and confidentiality are now one rendering mechanism.

## Consequences

- **Easier:** version skew stops being a coordination problem and becomes a property of the data — a node upgrades
  on its own schedule and never blocks on the fleet; a stuck-forever node remains a safe, legible, forwarding,
  preserving participant; the plaintext twin gives full-text search and RAG context for free off an artifact that
  had to exist anyway; the legibility and confidentiality ladders collapse into one mechanism (and one seam) to
  build and review; and the §3.8 "never DDL-migrate the log to delete" rule means local schema mistakes are
  recoverable by projection rebuild.
- **Harder / new surface:** every event now carries a mechanically-derived plaintext twin (storage cost — judged
  cheap, and net-positive given search/RAG/audit value, and compressed at rest); the signature must bind a
  canonical byte form, not re-serialized JSONB (a real discipline on the write path); and the **distribution plane**
  is genuinely new operational surface — reproducible builds, per-architecture signed release artifacts, an
  offline install ceremony, and fail-safe extension swapping on unattended hardware.
- **The bet:** that forward-compatible event format + a universal legibility twin let an arbitrarily version-skewed
  fleet interoperate safely *forever* with no coordinated upgrade, and that keeping the native (pgrx) surface small
  keeps the per-architecture distribution plane manageable. We would know it is wrong if the generic-rendering
  deferral proves false (some early format cannot be safely rendered by a stuck-old node even with the carried
  twin), if the twin and the structured body drift in practice despite mechanical derivation (poisoning search/RAG),
  or if fail-safe extension swapping proves impractical on Pi-class hardware (an upgrade that can brick a node would
  violate availability).
- **Policy-neutral ([principle 9](../index.md#founding-principles-the-lens-for-every-decision)):** Cairn ships the
  format contract, the twin, the two-plane separation, and the signed-release/extension-load mechanism; *which*
  releases a deployment installs, on *what* schedule, through *which* offline channel, and *who* may sign or
  authorize an upgrade, are policy.
- **Generic descriptor-driven rendering (Rung 1) is deliberately deferred and is itself an asserted property:**
  because every event already carries `schema_version` (the registry join key) and a flat twin floor, Rung 1 —
  a syncable, signed, version-pinned schema-descriptor registry plus one generic renderer — can be added later as
  **pure read-side machinery with no envelope change and no migration**. The deferral is safe by construction.
