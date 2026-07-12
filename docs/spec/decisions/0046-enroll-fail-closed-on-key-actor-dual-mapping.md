# ADR-0046 — Enroll fails closed on key→actor dual mapping (the B-direction whole-history guard)

- **Status:** Accepted (refines [ADR-0044](0044-enroll-fail-closed-on-actor-id-collision.md), [ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md))
- **Date:** 2026-07-12

## Context

[ADR-0044](0044-enroll-fail-closed-on-actor-id-collision.md) made `enroll_actor` (db/004) fail
closed in the **A-direction**: one `actor_id` claimed by two *different* signing keys, refused via
the whole-history predicate `cairn_actor_id_key_conflict(actor_id, key)`. The **B-direction** — one
signing *key* enrolled under two *different* `actor_id`s — was left unguarded (issue #166).

`submit_event` (db/005) resolves the author of every event to an actor purely by `signing_key_id`
(`SELECT array_agg(DISTINCT actor_id) … WHERE signing_key_id = …`; a single result stamps
`event_log.actor_id`, anything else stamps `NULL`). So a key mapping to **two** `actor_id`s silently
sets `actor_id = NULL` for **every** event that key authors node-wide — a graceful but real,
irreversible attribution loss (principle 4: "attribution honestly unknown", not misattribution). The
old floor allowed that mapping to be created even **serially** (each enroll computes a *different*
`actor_id`, and the A-direction check only inspects rows already under that `actor_id`); a Rust
orchestrator (`enroll_human_actor`) carried a read-then-write guard for it, but per-caller and racy,
so every future enrollment door (matcher agent, device, the ADR-0044 §3 actor-sync apply door) would
re-derive the same hole.

The per-epoch matcher already assigns a **fresh key per epoch** (`matcher_actor.rs`), so a
one-key→one-actor invariant does not constrain any legitimate flow.

## Decision

`enroll_actor` fails closed in the B-direction too: a fresh `enroll` binding a signing key that
already binds a **different** `actor_id` **anywhere in `actor_event` history** is refused. The
invariant is now bidirectional: **≤ 1 distinct signing key per `actor_id`, and ≤ 1 distinct
`actor_id` per signing key.**

1. A pure `STABLE` predicate `cairn_key_actor_id_conflict(p_key, p_actor_id)` — TRUE iff an
   `enroll`/`supersede` row carries `p_key` under an `actor_id` other than `p_actor_id`.
   Independently testable, reusable at the future actor-sync apply door / rotate-key door.
2. `enroll_actor` acquires a txn-scoped advisory lock on the **key first**, then on the `actor_id`
   (one global order → deadlock-free), then runs both the A- and B-direction checks. Under READ
   COMMITTED the loser of a concurrent same-key enroll re-reads the committed row and is refused,
   closing the TOCTOU race the Rust orchestrator could not.

**Whole-history, not live-only** (the deliberate scope choice): a key that ever bound a *different*
actor can never enroll a new one, even after that actor is revoked/superseded. This mirrors the
A-direction's anti-resurrection posture and the mission's identity conservation — a signing key is a
lifelong cryptographic identity of one entity; reusing it for a second actor is the silent identity
blur the floor exists to refuse. Both directions prevent the db/005 NULL harm; whole-history
additionally bans key-reuse across actors.

**Not a unique constraint:** idempotent re-enroll deliberately inserts a *second* `enroll` row with
the same key and same `actor_id` (re-runnable provisioning), which a partial unique index over
`signing_key_id` would reject. The predicate + advisory-lock shape is the ADR-0044 mechanism reused.

**Single door, mirror obligation:** the check lives on the only implemented write door
(`enroll_actor`). A future `rotate-key`/`supersede` door and the ADR-0044 §3 actor-event sync apply
door **must mirror both checks** — the analogue of the [issue #154](https://github.com/cairn-ehr/cairn-ehr/issues/154) apply-door caveat.

## Consequences

- **Safer:** the silent node-wide attribution loss is unreachable via the enroll door; the invariant
  is enforced at the floor, not re-derived per caller. The Rust `enroll_human_actor` Guard 1 becomes
  advisory legibility + the idempotent shortcut.
- **Harder / narrower:** a genuinely different entity must present its own key (already true for the
  matcher and for humans). A retired key cannot be recycled onto a new actor.
- **Unchanged:** db/005's NULL-on-ambiguity degradation and `events_by_actor_epoch`'s NULL-attribution
  fallback (db/006) stay — the guard narrows *how* the ambiguous state can arise (no longer via the
  enroll door), not the projection's duty to cope with it if it arises by another path (historical
  rows, a not-yet-guarded future sync door). The contamination-cascade recall tests stage that state
  via raw `actor_event` INSERT accordingly.

Floor-authorization only: no wire / event-format / SCHEMA / projection change.
