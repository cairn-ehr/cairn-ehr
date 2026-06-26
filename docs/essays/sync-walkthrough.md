# How Cairn Synchronizes Data

*A step-by-step walkthrough of set-union sync, the Hybrid Logical Clock, and the fast/slow lane split — grounded in the spec, the governing ADRs, and the two working implementations.*
{: .essay-lead }

---

This essay traces, end to end, how data moves between Cairn nodes: from the single idea the whole
design hangs from, through the wire protocol, to the two-lane split that lets a gigabyte of imaging
travel the same mesh as a critical allergy without ever starving it. It is grounded in the canonical
[synchronisation spec](../spec/sync.md), the governing decision records, and the two implementations
that already exist — the Python walking skeleton (`poc/replication-failover/`) and the Rust daemon
(`crates/cairn-sync/src/main.rs`).

## 0. The one idea everything else hangs from

Cairn never *merges*. It *unions*.

Conventional replication ships rows and then resolves conflicts — last-writer-wins, vector-clock
reconciliation, three-way merge. Every one of those is a decision about *whose data survives*, and in
a clinical record a wrong such decision is silent data loss: a demographic overwrite, a mis-merged
patient. Cairn designs the entire conflict-resolution problem away upstream, in the data model, so
the sync layer has nothing left to decide. As [§6.1](../spec/sync.md) puts it, the sync service
"**ships and applies events; it does not merge or resolve conflicts** — because the clinical log is
append-only and immutable, syncing the source of truth is INSERT-only, idempotent (UUIDv7 PK), scoped
**set-union**; there are no row-level clinical conflicts to resolve."

That single property — *sync is set-union over an append-only event log* — is what makes everything
downstream tractable: partition tolerance, sneakernet, multi-source fetch, and the fast/slow lane
split. The rest of this essay is the consequences.

## 1. What a "record" actually is

There is no patient row to overwrite. A patient record is the **sum of autonomous, signed, immutable
events** — written by different professionals at different places and times, and *assembled* from
those parts whenever they can be gathered ([§6.4](../spec/sync.md)). Corrections don't mutate; they
are new events that reference the originals. "Nobody owns the record."

Each event, per [data-model §3.5](../spec/data-model.md), splits into:

- a **typed envelope** the machinery must read — `uuidv7` primary key, `patient_uuid`, the HLC as
  typed fields, contributor set, signature, a closed `event_type` enum, and scope keys; and
- an **opaque body** carrying the clinical content.

Two envelope facts make set-union safe:

1. **A globally-unique id (UUIDv7).** Two nodes writing independently during a partition can never
   collide on a key, so "copy what the other side lacks" is well-defined and idempotent.
2. **A Hybrid Logical Clock**, giving causal order that survives skewed wall-clocks (next section).

The signed artifact is *bytes*, not a parsed structure. Per
[ADR-0015](../spec/decisions/0015-event-serialization-signatures-and-content-addressing.md), Cairn
signs **deterministic CBOR carried in a COSE_Sign1 envelope, with Ed25519**, then verifies by
`hash(stored_bytes)` + signature check — never by re-serializing. The structured view is *parsed out
of* the exact stored bytes and never round-tripped back. This collapses the determinism burden from
"every implementation must canonicalize identically, forever" to "the signer serialized once;
everyone else byte-compares," and it makes lossless forwarding (§5) automatic. Content addresses are
**SHA-256 for events, BLAKE3 for blobs** — the latter's internal Merkle tree is what later enables
chunk-level verification on the slow lane.

## 2. Ordering without a coordinator: the Hybrid Logical Clock

Off-grid nodes have unreliable wall-clocks, and there is no central sequencer to ask. The HLC gives
every event a timestamp `(wall, counter)` that tracks physical time closely enough to be
human-meaningful yet yields a *deterministic total order across independent nodes* even when their
clocks disagree. The canonical cross-node key is the triple **`(wall, counter, node_origin)`**
([data-model §3.2](../spec/data-model.md)).

Two rules drive it, both implemented in `poc/replication-failover/src/cairn_demo/hlc.py`:

- **`tick(now_ms)`** — for a locally-originated event: if physical time advanced, adopt it and reset
  the counter; otherwise keep the (larger) logical time and bump the counter so the new event still
  sorts strictly after the previous one.
- **`merge(remote, now_ms)`** — on *receiving* a remote event: the new wall is
  `max(local, remote, now)`, and the counter is chosen so the merged stamp dominates whichever
  input(s) shared that max.

The payoff: two nodes that wrote independently while partitioned converge on *exactly the same
ordering* after reconnect, with no coordination and no last-write-wins loss.

## 3. The set-union exchange, step by step

The walking skeleton (`poc/replication-failover/src/cairn_demo/sync.py`) reduces the whole protocol
to its essence:

```python
def sync_pair(conn_a, conn_b):
    ids_a, ids_b = event_ids(conn_a), event_ids(conn_b)
    result.a_to_b = _apply_events(conn_b, _fetch_events(conn_a, ids_a - ids_b))
    result.b_to_a = _apply_events(conn_a, _fetch_events(conn_b, ids_b - ids_a))
    result.converged = event_ids(conn_a) == event_ids(conn_b)
```

Step by step:

1. **Diff.** Each side enumerates its event ids; the symmetric difference is what's missing where.
2. **Ship.** Each missing event is copied to the other side with `INSERT ... ON CONFLICT DO NOTHING`
   — so re-running sync is always safe and idempotent.
3. **Advance the clock.** The receiver folds the batch-maximum remote HLC into its own via the
   `merge` rule, so any future local event sorts after everything just absorbed.
4. **Confirm convergence** by re-reading both id sets.

No step anywhere decides whose data wins. There is nothing to decide.

The production daemon does the same thing over a real wire. Its `apply_signed`
(`crates/cairn-sync/src/main.rs`) first calls `verify_self_described(signed_bytes)` — **refusing
anything that doesn't verify** — then derives the content address, inserts with `ON CONFLICT DO
NOTHING`, and advances `hlc_state` with a `GREATEST`-based merge in the same transaction. The daemon
carries **no merge logic**
([ADR-0001](../spec/decisions/0001-fat-postgres-thin-daemon.md), the *fat Postgres / thin Rust
daemon* split): convergence is set-union plus an in-database projection trigger. The daemon only ships
bytes, verifies, and applies.

This is also why **transport is interchangeable**. Set-union doesn't care *how* the missing events
arrive: over a WireGuard link, from a LAN sibling, or hand-carried on a USB stick (store-and-forward
"sneakernet sync", [§6.1](../spec/sync.md)). A total network partition degrades to *latency*, never
to *inconsistency*.

> [!NOTE]
> **A note on scope.** "Sync scopes" (which records a node pulls *by default*) are an administrative
> **prefetch hint, not an access authority**
> ([§6.4](../spec/sync.md), [ADR-0004](../spec/decisions/0004-dynamic-sync-scope-prefetch-not-authority.md)).
> A node may acquire any record it has *legitimate need* for, even out of scope, even with the parent
> unreachable — because acquisition is idempotent set-union, it is always safe, and the parent later
> *ratifies and audits* rather than gates. Granting interest is urgent and edge-authorized; revoking
> is lazy and parent-mediated. Crucially, **replication is never the confidentiality boundary**
> ([ADR-0006](../spec/decisions/0006-visibility-scope-replication-and-the-safety-projection.md)): a
> safety-relevant sensitive episode replicates *unconditionally* so the chart can warn on it;
> confidentiality lives downstream in key-custody and visibility, with a de-identified **safety
> projection** that lets decision-support fire a severity-graded warning *naming nothing*.

## 4. Two planes that run at different speeds

Everything above describes a single channel. But Cairn deliberately separates *two kinds of traffic
that must never share fate*. This is the first and coarsest lane split, from
[ADR-0012](../spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md) /
[§6.5](../spec/sync.md):

- **The sync plane** carries signed, immutable clinical events — set-union, AP, skew-tolerant, and
  **never executable code**. The event *format* evolves here, fleet-wide, forward-compatibly.
- **The distribution plane** carries code/DDL/pgrx extensions — per-node, per-architecture, signed
  against a steward key and verified before install, delivered online or by sneakernet.

Why split them? Because syncing a native extension binary over the clinical mesh would be a
remote-code-execution channel into every node. And because decoupling dissolves the "lockstep fleet
upgrade" problem: **a node's schema version only ever has to match *that node's own* schema, never the
version of events arriving from elsewhere.** A fleet of offline nodes can carry permanent, unbounded
version skew and still interoperate.

The invariant that makes this hold is **lossless forwarding** ([§6.5](../spec/sync.md)): a node
receiving an event authored under a *newer, unseen* schema **stores, re-propagates, and exports it
byte-for-byte** — never down-converting or re-serializing (which would break the signature). Its
human-readable view is a *local projection*; a future upgrade simply re-derives it. "The tolerance
window is infinite for custody, best-effort for understanding." This is only cheap because of the
ADR-0015 sign-the-bytes decision — the node ships exactly what it received.

## 5. The fast lane and the slow lane

Within the sync plane itself there is a second, finer split — and this is the one most people mean by
"fast/slow lane." Clinical events are small and latency-bound; binary attachments (imaging, scans,
waveforms) are orders of magnitude larger and throughput-bound. They must not share a queue, because a
single in-flight gigabyte would head-of-line-block the channel.

The motivating failure is real and named in [§6.6](../spec/sync.md): *a nightly bulk imaging sync that
ground a whole deployment to a halt, so that emergencies could retrieve **no** record at all.* The
lesson: **priority ordering alone is insufficient** — you also need resource isolation.

### Fast lane — the eager event plane

New clinical events and audit events go first; identity events (link / repudiate / reattribute) are
high priority; **attachment bytes go last** ([§6.1](../spec/sync.md) upstream priority order). On the
wire (`crates/cairn-sync/src/main.rs`) this is the `EventsAfter { wall, counter }` request: the puller
asks for every event at or after an HLC watermark; the response ships **verbatim signed bytes**; the
receiver verifies-on-apply and inserts idempotently. The clinical plane is framed as JSON precisely
because it is small and latency-bound, not throughput-bound. Field measurement on the Cape York ↔
Dorrigo satellite link came in at **~494 bytes/event** with **zero signature-verification failures
across 792 events** ([spike 0001](../spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md)).

The key design move
([ADR-0013](../spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md)) is
**reference-eager, byte-lazy**: the attachment *reference* (content digest + descriptor) rides the
eager event plane, so a node knows the attachment *exists* the instant the event arrives — even though
the bytes haven't moved yet. The reference is part of the signed event; the bytes are a separate tier.

### Slow lane — the lazy, resource-isolated byte tier

The bytes are fetched on a separate channel that is **chunked, preemptible, and separately budgeted**,
so clinical events always interleave *between* chunks ([§6.6](../spec/sync.md)). On the wire this is a
second request type, `BlobSlice { addr_hex, offset, len }`, returning a deliberately **binary** frame
— `[found:u8][total_len:u64 BE][slice…]` — *not* JSON, because hex-encoding doubled every transferred
byte and halved measured throughput.

The actual fetch loop is `do_blobd` (`crates/cairn-sync/src/main.rs`). Walking through what makes it
the slow lane:

1. **Slice granularity.** Blobs are addressed in `SLICE_BYTES = 256 KiB` windows, tuned to amortize
   the BLAKE3/bao tree overhead.
2. **Resumability.** Before fetching, it queries which `chunk_index` values are already persisted in
   `blob_chunk`; only the missing indices go into the work queue. Verified slices accumulate across
   passes and across dropped connections — a partition mid-transfer costs you nothing already
   received.
3. **Bounded parallelism.** A worker pool of `window` threads (clamped to `1..=16`, because each
   worker opens its own PG connection and adds link load) pulls indices off a shared queue.
4. **Swarm fetch with zero trust.** Each worker round-robins across all `peers` (offset by
   `worker + index` to spread load), and **verifies every slice against the content address** via
   `verify_slice(slice, &root, offset, len)` *before* persisting. A lying or faulty source is rejected
   and the next source tried. Because a blob self-verifies against the digest in the signed event, it
   can be pulled multi-source, chunked, and resumable from *any* holder — sibling, parent, or the
   device carried with the patient — with no trust in the source. (This is the BLAKE3 Merkle-tree
   payoff from ADR-0015.)
5. **The preemptible budget — the availability floor.** Every worker `sleep`s `budget_ms` between
   requests. This is the mechanism that *guarantees* the slow lane can never starve the fast lane: the
   effective byte-tier load is `budget_ms × window`, deliberately throttled so clinical sync always
   has headroom. The clamp on `window` and the inter-request sleep together enforce ADR-0013's rule:
   **"blob transfer must never reduce clinical-data availability."**
6. **Assemble and flip.** When every index is present, the blob is assembled, **re-verified whole**,
   and flipped to `present`.

### Lazy *and* elective

The slow lane is not just deferred — it is **opt-in and separately scoped** ([§6.6](../spec/sync.md)).
The blob-prefetch predicate is a far narrower thing than the event-scope predicate: **references
replicate everywhere; bytes replicate by election.** A resource-starved node defaults to
references-only, fetch-on-demand (it often *can't* store every PACS blob), pulling bytes from durable
blob-holders upstream only on legitimate need — a clinician opening the viewer promotes that blob to a
foreground fetch that overrides the background "attachments last" priority. And a logical attachment is
a *set* of renditions (raw + lightweight preview + report text), so the small preview can ride along
eagerly while the gigabytes stay on-demand — the chart is legible before the raw study lands.

## 6. Putting it together: the life of a partition-and-reconnect

![Sequence diagram of a Cairn partition-and-reconnect: an ED node writes a note and a CT
reference while offline; after the link returns, the fast lane ships and verifies signed event
bytes (set-union INSERT, HLC merge) so the chart shows the note and "CT exists" before any pixel
arrives, while the slow lane fetches the imaging in verified, budgeted 256 KiB chunks from the
swarm — converging to byte-identical content addresses on both sides.](assets/sync-walkthrough-sequence.svg)

<!-- Source: assets/sync-walkthrough-sequence.mmd — regenerate the SVG with:
     npx -y @mermaid-js/mermaid-cli -i assets/sync-walkthrough-sequence.mmd \
       -o assets/sync-walkthrough-sequence.svg -b transparent -->

> [!NOTE]
> The diagram above is a committed SVG so it renders everywhere — GitHub, a plain Markdown preview,
> and the built site alike. Its editable Mermaid source lives beside it at
> `assets/sync-walkthrough-sequence.mmd`; regenerate the image with the command in the comment above
> after any edit.

In prose:

1. A clinician at an ED node writes a note while the WAN is down. The event is signed (Ed25519/COSE),
   stamped with an HLC `tick`, and committed locally — **availability over consistency**; the write
   never blocks on the network.
2. An attachment (a CT study) generates a *reference* folded into a signed event, plus blob chunks
   staged locally. The reference is fast-lane; the bytes are slow-lane.
3. The patient is moved mid-partition. No scope is "reassigned"; the receiving node simply gains
   *reason to assemble* the patient and acquires the parts — from a LAN sibling, hand-carried, or from
   the parent on reconnect ([§6.4](../spec/sync.md)).
4. On reconnect, the fast lane runs first: `EventsAfter` ships the signed event bytes, each
   verified-on-apply and inserted `ON CONFLICT DO NOTHING`. Both nodes' HLCs merge forward.
   Convergence is byte-identical content addresses on both sides — *same bytes → same address →
   zero-merge convergence*.
5. The slow lane drains in the background: `do_blobd` pulls the CT in 256 KiB verified slices across
   whatever holders are reachable, sleeping `budget_ms` between requests so freshly-arriving clinical
   events keep flowing between chunks.
6. Throughout, every projection shows a **freshness indicator** and surfaces **known-missing** parts
   ([§6.2](../spec/sync.md)) — "honest assembly state." The chart never silently pretends to be
   complete.

## 7. Why this shape, in one paragraph

Cairn earns offline-first, partition-tolerant, multi-source, sneakernet-capable sync by paying a
single upfront price: **the clinical record is an append-only log of signed, immutable, HLC-ordered
events.** That price buys away the entire merge-conflict problem, which lets the sync engine be a thin,
reviewable "ship-verify-apply" daemon
([ADR-0001](../spec/decisions/0001-fat-postgres-thin-daemon.md)) with all the safety logic in the
database. On top of that base sit two orthogonal lane-splits: **sync plane vs. distribution plane**
(clinical events never carry code; schema skew is unbounded and fine), and within the sync plane,
**eager event fast lane vs. lazy, chunked, content-addressed, budget-throttled byte slow lane** (so a
gigabyte of imaging can never starve an emergency lookup). Each split is the same principle applied at
a different grain: *carry what you can verify, defer what you can't afford, and never let the heavy
thing block the urgent thing.*

## Known limits and honest caveats

In keeping with surfacing gaps rather than burying them, three things the current implementation does
*not* yet do:

- **The slow-lane budget is a fixed `budget_ms` sleep, not adaptive.** It guarantees the floor by
  throttling unconditionally — which means on a *fat, idle* link the byte tier under-utilizes
  available bandwidth. The spec language ("preemptible, separately budgeted") would also admit a
  feedback controller that backs off only when the clinical plane is active. A deliberate skeleton
  simplification, not the final design.
- **`chunk_index` is an `i32`**, capping a single blob at ~549 GB at 256 KiB slices — fine for any
  DICOM study, but the dedicated object-store tier (not BYTEA-in-Postgres) is where large blobs
  ultimately belong. The skeleton stores chunks in `blob_chunk`; production blob custody is out of
  scope for it.
- **The two planes share one WireGuard transport in the daemon** (`NoTls` is deliberate — "the link
  is the transport"). The *logical* isolation (separate budgets, separate request types) is real, but
  physical link contention is mitigated by the inter-request sleep, not by separate QoS-tagged
  connections. Adequate for the demonstrated availability-floor bet (Bet A4 passed), but at facility
  scale you would likely want transport-level prioritization too.

---

*Primary sources:* [sync spec §6.1–6.8](../spec/sync.md) · [data-model §3.2/§3.5](../spec/data-model.md)
· ADRs [0001](../spec/decisions/0001-fat-postgres-thin-daemon.md),
[0004](../spec/decisions/0004-dynamic-sync-scope-prefetch-not-authority.md),
[0006](../spec/decisions/0006-visibility-scope-replication-and-the-safety-projection.md),
[0012](../spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md),
[0013](../spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md),
[0015](../spec/decisions/0015-event-serialization-signatures-and-content-addressing.md) ·
implementations `crates/cairn-sync/src/main.rs` and `poc/replication-failover/` · field validation
[spike 0001](../spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md).
