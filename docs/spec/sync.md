# 6. Synchronisation Layer

## 6.1 Mechanism
- Transport-agnostic, resumable, delta-based protocol over HTTPS; optional store-and-forward via removable media ("sneakernet sync") for fully disconnected sites.
- **Build, don't adopt.** Built on **PostgreSQL logical decoding** as the change-capture primitive, with a **thin custom Rust sync service** implementing scoping, filtering, priority, and idempotent apply. The service **ships and applies events; it does not merge or resolve conflicts** — because the clinical log is append-only and immutable, syncing the source of truth is INSERT-only, idempotent (UUIDv7 PK), scoped **set-union**; there are no row-level clinical conflicts to resolve. The DB guarantees that "apply" is safe ([language-substrate §9.4](language-substrate.md#94-merge-projection-boundary-fat-postgres-thin-rust-daemon)); the daemon stays thin.

> [!NOTE]
> **pgactive / SymmetricDS are references, not dependencies.** They exist to resolve *row-level*
> conflicts (last-writer-wins and similar) — a problem Cairn designed away — and their default
> policies can *violate* invariants (LWW on a demographic = silent data loss, forbidden by
> [§4](demographics.md)). Borrow their patterns (logical-decoding plumbing, store-and-forward /
> sneakernet) without a hard runtime dependency, honoring the vendor-independence mission. See
> [ADR-0001](decisions/0001-fat-postgres-thin-daemon.md).

- Per [§9](language-substrate.md), this safety-critical service is Rust and/or in-database, never a dynamic language.
- **Sync scopes** are declarative subscription predicates that govern *automatic* replication — an administrative **prefetch default**, not an access boundary. They are versioned, auditable, and normally evaluated at the parent; but a node may acquire any record it has **legitimate need** for, even out of scope and even with the parent unreachable (audited). Scope governs what a node pulls *by default*, never what it is *permitted* to hold. See [§6.4](#64-scope-is-a-prefetch-hint-not-an-authority).
- Bandwidth discipline: compression, binary diffs, attachments synced lazily by reference with priority queues.
- **Upstream priority order:** new clinical events and audit events first; identity events (link/repudiate/reattribute) high priority; attachments last.

## 6.2 Consistency model
- Eventual consistency with causal ordering (HLC) within a patient record.
- Every projection displays a **freshness indicator** ("last synced with parent 4 h ago") — a first-class UI requirement.
- **Honest assembly state.** The chart is always a best-effort assembly of currently-available parts, and must say so as a first-class clinical fact. Beyond freshness it surfaces **known-missing** parts when it can detect them (the parent advertised 5 episodes, only 3 arrived; a sibling is reachable but unsynced) and, when fully partitioned, signals that parts may exist beyond the island. Making absence *visible* is a safety gain with no paper equivalent — on paper the other ward's notes are simply, invisibly absent. See [§6.4](#64-scope-is-a-prefetch-hint-not-an-authority) and [ADR-0004](decisions/0004-dynamic-sync-scope-prefetch-not-authority.md).

## 6.3 Failure modes (designed-for)
| Failure | Behaviour |
|---|---|
| Internet down | Facility operates on facility server; queues outbound |
| Intranet down | Department server is local master for its scope |
| Department server down | Workstations operate standalone on mirrored scope |
| Node destroyed | Re-provision from parent; only unsync'd local events are at risk → aggressive upward sync priority |

## 6.4 Scope is a prefetch hint, not an authority
> Resolves former open question §11.3 — see [ADR-0004](decisions/0004-dynamic-sync-scope-prefetch-not-authority.md).

A patient moves ED→ICU *mid-partition*: who reassigns the sync scope while the parent is unreachable? The question dissolves once scope is understood as administration, not ownership. **Nobody owns the record** — it is the sum of autonomous, signed parts written by different professionals at different places and times, *assembled* from those parts when it can be. A "transfer" reassigns nothing; it merely gives the receiving node *reason to assemble the patient*, so that node **acquires the parts**:

- from a **sibling on the same LAN** (the common "internet-down, intranet-up" case);
- **carried with the patient** on the device that travels with them (store-and-forward / sneakernet, [§6.1](#61-mechanism)) in a total partition — the digital transfer hand-off, paper-parity-exact;
- or from the **parent on reconnect**.

Because acquisition is INSERT-only, idempotent set-union ([data-model §3.1](data-model.md#31-append-only-clinical-event-log-source-of-truth)) it is always safe, and the parent — when reachable — **ratifies and audits** rather than gates. Two asymmetries make this work:

- **Granting scope is urgent and edge-authorized; revoking is lazy and parent-mediated.** Lacking a needed chart is a safety and paper-parity failure; holding an extra copy slightly longer is harmless. So a node never *moves* a scope (a dangerous mutation needing an authority) — it only ever *adds* an interest, and surplus copies are garbage-collected later by the parent.
- **Access follows legitimate need + audit, not pre-granted permission.** On paper, the chart travels with the patient and nobody phones records for permission; the receiving clinician must read and write *now*. The digital equivalent is break-the-glass acquisition that is **recorded** — strictly better than paper, which leaves no trace.

The surviving requirement is **honest assembly-state disclosure** ([§6.2](#62-consistency-model)). Interactions deferred elsewhere: garbage collection of surplus copies touches retention/erasure ([§11.5](open-questions.md)). Legitimate-need acquisition of *sensitive* episodes is **resolved** ([§11.8](open-questions.md), [ADR-0006](decisions/0006-visibility-scope-replication-and-the-safety-projection.md)): replication is **never** the confidentiality boundary — a safety-relevant sensitive episode replicates **unconditionally** (the chart must be able to warn about, and break glass on, content that arrived), and confidentiality is enforced downstream at key-custody and visibility, never by withholding the row. Abstracting a sensitive episode's semantic scope key to an opaque token makes this automatic: the prefetch predicate can no longer select on it, so it falls back to replicate-everything-for-this-patient ([identity §5.9](identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)).
