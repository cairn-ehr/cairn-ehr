# ADR-0031 — Canonical identifiers and node-local surrogate keys: the dual-identifier discipline

- **Status:** Accepted
- **Date:** 2026-06-22
- **Refines:** [ADR-0001](0001-fat-postgres-thin-daemon.md), [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md), [ADR-0015](0015-event-serialization-signatures-and-content-addressing.md)

## Context

Cairn's federation/fractal nature requires **globally-unique, offline-mintable** identity: any node, partitioned and alone, must be able to create events and entities that will never collide with another node's, with no coordination. We satisfy that with UUIDv7 (`event_id`, `patient_id` — [§3.2](../data-model.md#32-identity-time)) and content-addresses (`content_address`, `actor_id`, `blob_address` — multihash `BYTEA`, [ADR-0015](0015-event-serialization-signatures-and-content-addressing.md)). These are the right identifiers for *identity*. They are the **wrong** identifiers for *physical join keys*, and in an EHR that distinction is a **safety** issue, not a convenience one: a record that is slow to retrieve fails paper-parity ([principle 3](../index.md#founding-principles-the-lens-for-every-decision)).

The cost is concrete and lands in three distinct places in PostgreSQL (which is heap-organized, not index-clustered — so the penalty sits in the indexes and WAL, not in heap reorganization):

1. **Random insertion order** → B-tree page splits, low fill-factor, buffer-cache churn, WAL full-page-image bloat. UUIDv7 already fixes this half (it is k-sortable → right-hand-side appends). **Content-addresses cannot be helped this way — they are hashes, uniformly random by design.**
2. **Key width propagated into every referencing index.** A 16-byte `patient_id` (or a 34-byte multihash) repeated across every projection row and every foreign-key index. UUIDv7 does nothing for this; it is the dominant cost as the FK graph fans out.
3. **Wide *random* `BYTEA` keys** (the content-addresses) are *both* random *and* wide — the worst of both worlds for any index that references them.

The conflation to defuse is *"the fractal nature demands UUIDs (so they must be our keys)."* Federation demands globally-unique **identity**; it does not demand that identity be the physical join key. Cairn already separates the two layers that own these two jobs — the signed, synced **event core** versus the per-node, rebuildable **projection** ([§3.1](../data-model.md#31-append-only-clinical-event-log-source-of-truth): *"Current state … is a projection materialized per node — rebuildable, cacheable, never synced itself"*). The performance fix is therefore an instance of an existing architectural seam, not a new concept. This is also a **can't-retrofit** decision: once projections and their foreign keys are built around one identifier choice, changing it is a migration across the whole physical schema — so it is fixed now, before further implementation.

## Decision

**Two identifier planes, one discipline.**

1. **Canonical plane (global, signed, synced) is unchanged.** `event_id`/`patient_id` stay UUIDv7; content-addresses/actor-IDs/blob-digests stay self-describing multihashes ([ADR-0015](0015-event-serialization-signatures-and-content-addressing.md)). These are the only identifiers that ever appear in a signed event body, on the inter-node wire, as a content-addressing input, or as a stable API identity. They are immortal ([principle 2](../index.md#founding-principles-the-lens-for-every-decision)).

2. **Projection plane may intern canonical IDs to node-local `bigint` surrogates.** A node maintains a private interning dictionary mapping each canonical ID to a dense `bigint` (`bigserial`), used as the physical foreign-key and join key inside projections. **`bigint`, never `int`** — a busy facility over decades can pass 2.1 × 10⁹ events; int8 headroom (9.2 × 10¹⁸) is free.

3. **The hard rule (leakage is silent cross-node corruption).** A surrogate must **never** appear in a signed body, on the wire, as a content-addressing input, or as a stable API identity. If a surrogate leaked into signed content, two nodes would assign different integers to the same entity → divergent digests/signatures → set-union sync silently breaks. Leakage is made *hard*, not merely forbidden:
   - **Distinct domain types** — a `local_ref` domain (`bigint`) distinct from the `uuid`/`bytea` canonical types, so a function that accepts a surrogate where a global ID belongs does not typecheck.
   - **Mapping is confined to the floor functions.** `submit_event` ([ADR-0022](0022-validated-submit-surface-the-write-path.md)) resolves global→local on ingress; the projection-read and sync-emit paths rehydrate local→global on egress. No other code path interns or de-interns.
   - **API egress is always the global ID** — never a surrogate (a client bookmark must use the UUID; see rule 6).

4. **Bind the pair once per entity, at its home/anchor row; references carry only the surrogate.** The "carry both the UUID and the internal id as two fields" instinct is correct **at the anchor row and on the already-signed `event_log`** (where the canonical UUIDs are mandatory and present anyway), and **self-defeating on every referencing row**. Carrying a 16-byte UUID *and* an 8-byte ref on every referencing row makes references *larger* than the pure-UUID design while still needing an index — it re-imports exactly cost #2. So: the UUID↔ref binding lives **once per entity** (the dictionary / anchor row, the small extra storage — 8 bytes per *entity*, not per *reference*); downstream references carry **only the `ref`**; the UUID is recovered by a join to the anchor, **and only at egress** (one join per distinct entity, not per row — rendering a chart, the patient UUID is known once). Where profiling later shows a specific hot read path that the egress join hurts, denormalizing the UUID *there* is a permitted targeted covering optimization — the exception, not the default shape.

5. **Scope by where the cost actually is, confirmed by measurement.**
   - **Strongest case — wide random `BYTEA` references** (`content_address`, `actor_id`, `blob_address` wherever used as a join/FK key): keep the canonical digest in exactly one column of one row; everything that *references* it uses an `int8` surrogate. (One unavoidable random unique index on `content_address` remains, for sync dedup — but it is no longer propagated across the FK graph.)
   - **Next — `patient_id`** in the projection plane: the highest-fan-out FK in the system; the single biggest measurable win.
   - **Leave `event_id` PKs as UUIDv7** — written once, must byte-match the signed body anyway, a friendly k-sortable index, and not a high-fan-out join target; interning it buys little and burdens the hottest write path.
   - Final magnitude/scope is **measured on Bet B** of [Spike 0001](../../spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md) (the Pi board), exactly as [ADR-0001](0001-fat-postgres-thin-daemon.md)'s compute bet is — a "no measurable win" result narrows the scope rather than failing the discipline.

6. **Surrogates are not durable identity, even locally.** They are not stable across a projection rebuild (replay can reassign them in a different order) and never portable across nodes (only the global-ID event stream syncs). This is *fine* — projections are rebuildable and surrogates never persist off-node — **but** anything durable that must outlive a rebuild (a saved query, an analytic snapshot, a local bookmark, any persisted access-control artifact) must reference the **canonical ID**, never the surrogate.

**Canonical home:** [data-model §3.18](../data-model.md#318-canonical-identifiers-and-node-local-surrogate-keys-the-dual-identifier-discipline).

## Consequences

- **Easier:** foreign-key indexes shrink from 16/34 bytes to 8 (≈3× on the patient/content-address fan-out), denser and more cache-resident — directly the paper-parity retrieval floor on Pi-class hardware ([principle 3](../index.md#founding-principles-the-lens-for-every-decision)). Joins compare 8-byte integers, not 16/34-byte values. The seam is the one Cairn already has — projections are local and rebuildable — so this adds no new architectural concept.
- **Harder:** the interning dictionary is a write-hot serialization point on ingress (needs a concurrency-safe `INSERT … ON CONFLICT … RETURNING`, with the unique index as backstop) — *N* fat random FK indexes traded for *one* fat random dictionary index, a net win that Bet B must confirm. Indirection (a join to rehydrate the UUID at egress) is added to read paths. The leakage hazard is real and silent; the `local_ref` domain type plus the floor-function chokepoint are the standing guards.
- **The bet:** the dual-identifier discipline holds — surrogates never escape the projection plane, and the interning cost on ingress stays below its budget on weak hardware. We would know the bet fails if Bet B shows no material FK-index/join win (→ narrow or drop interning, keep UUIDv7-only), or if a surrogate is ever found in a signed body / on the wire (→ a `local_ref`-domain or floor-function discipline breach, fixed in-DB).
- **No new founding principle.** This is [principle 3](../index.md#founding-principles-the-lens-for-every-decision) (*paper-parity — retrieval speed is a safety floor*) and the [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md) layering (*the local physical schema sits below the wire core*) applied to identifier representation. The canonical, immortal identity ([principle 2](../index.md#founding-principles-the-lens-for-every-decision)) is untouched — the surrogate is merely the canonical ID's local handle.
