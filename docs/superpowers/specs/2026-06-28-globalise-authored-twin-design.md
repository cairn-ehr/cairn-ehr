# Design — Globalise the authored legibility twin

**Date:** 2026-06-28 · **Status:** approved, ready for plan · **Spec target:** v0.39 → v0.40

## Problem

The §3.13/§4.5 plaintext legibility twin is the principle-11 guarantee: every event stays
human-readable for as long as it exists, no matter how far the schema has moved. Principle 11
requires the twin be materialised by the *author* (who understands the schema) and signed into
the body, so a reader generations behind carries it forward and never has to re-derive it from a
schema it may not understand.

Today only the two demographic event types carry an author-materialised twin. Every other type
falls through the validated door's `cairn_event_twin` hook ([db/005](../../../db/005_submit.sql),
last replaced in [db/011](../../../db/011_demographics_fields.sql)) to `cairn_twin_skeleton` — a
*receiver-derived* rendering (`[type] schema for patient <id>`, no payload). That is a
legibility-across-time hole and the open [db/005:29](../../../db/005_submit.sql#L29) TODO. A
second derive-path exists in `cairn-sync` (`apply_signed`, spike-grade WAN sync that bypasses the
floor) which re-derives the twin on apply.

The deferral: *globalise the authored twin to every event type; retire the skeleton fallback as
the default.*

## Decisions (from brainstorming)

1. **Honest degradation, not hard reject.** The authored twin is mandatory at *authoring* (every
   conformant builder materialises it) and *preferred* at the floor; but a twin-less event (an
   older / non-conformant peer) is still **stored**, with a derived skeleton twin **flagged** as
   non-author-faithful. This preserves set-union convergence (principle 1 + availability over
   consistency + ADR-0012's heterogeneous, no-lockstep fleet). The skeleton is repositioned to the
   honest-degradation path, never the default. Mirrors the recurring Cairn pattern (floor open /
   projection gated; degrade to human review).
   - **Exception, by design:** the two demographic types keep their *hard* authored-twin
     requirement (ADR-0034 — demographics is a day-one twin-native surface; no conformant path
     omits the twin, and an older node that doesn't know the demographic types rejects them at the
     classification step anyway, so a twin-less demographic event is a same-version bug → reject).

2. **Scope: validated door + cairn-sync mirror fix.** Core is `cairn-event` renderers + the
   `db/005` hook. Plus a small consistency fix to `cairn-sync`'s apply/authoring paths so the two
   write-paths agree. *Out of scope:* routing `cairn-sync` through `submit_event` (a separate
   Phase-1 roadmap thread).

3. **One generic renderer, reused.** The 6 non-demographic types
   (`note.added`, `patient.created`, `patient.amended`, `advisory.added`, `salience.downgrade`,
   `visibility.suppress`) are placeholder spike types, authored only in tests. Reposition the
   existing `cairn_event::plaintext_twin()` (already renders type/schema/patient/HLC/effective/
   payload) as THE canonical generic authoring renderer. Per-type prose is deferred until each
   type gets a real clinical spec (demographics already has its tailored renderers).

## Key mechanism — authored/derived is derivable, not stored

The signed body is immutable and already persisted (`event_log.signed_bytes`). Because
`EventBody.plaintext_twin` is `#[serde(skip_serializing_if = "Option::is_none")]`,
`cairn_body(signed_bytes) ->> 'plaintext_twin'` is an authoritative, immutable answer to "did the
author materialise a twin?" (verified: [cairn_pgx cairn_body](../../../extensions/cairn_pgx/src/lib.rs#L25)
serializes the full `EventBody`).

Therefore: **no new column, no `submit_event` re-declaration** (avoids the copy-paste drift the
demographics work deliberately avoided — only the `cairn_event_twin` hook changes). The
authored-vs-derived flag is a read-time *projection* of the signed body, exposed for the future
duplicate-sweep / audit / re-authoring worklist.

## Components

### A. `cairn-event` (Rust) — the authoring renderer
- Reposition `plaintext_twin(&body) -> String` as the canonical generic authoring renderer;
  document the dual role (author materialises + signs it in; same shape the fallback derives).
- Add pure `resolve_twin(&EventBody) -> String` = prefer a trimmed-non-empty `body.plaintext_twin`,
  else `plaintext_twin(&body)`. The single rule both the cairn-sync apply path and (mirrored in
  SQL) the floor follow.
- Add `materialise_generic_twin(EventBody) -> EventBody` — sets `plaintext_twin: Some(...)` if
  `None` (idempotent; won't clobber an existing authored twin) — so a conformant author globalises
  in one call before `sign()`.

### B. `db/015_globalise_twin.sql` (new migration; register in `db.rs` SCHEMA, 13 → 14)
- Improve `cairn_twin_skeleton(p_type, b)` to render the payload too (closes the db/005 TODO).
- `CREATE OR REPLACE cairn_event_twin`: demographic branches **unchanged** (structural check +
  hard authored-twin requirement, per ADR-0034); **every other type** → prefer `b->>'plaintext_twin'`
  if non-empty, else `cairn_twin_skeleton` (derived).
- Add read-time `cairn_twin_is_authored(signed bytea) -> boolean` and an `event_twin_provenance`
  view `(event_id, twin_authored)` over `event_log.signed_bytes`; `GRANT SELECT` to `cairn_agent`.
- No `submit_event` change.

### C. `cairn-sync` (Rust) — mirror fix
- Apply path (`apply_signed`, [main.rs:173](../../../crates/cairn-sync/src/main.rs#L173)): replace
  `plaintext_twin(&body)` with `resolve_twin(&body)`.
- Authoring path ([main.rs:342](../../../crates/cairn-sync/src/main.rs#L342), the emit/ingest
  command minting a new event with `plaintext_twin: None`): materialise via
  `materialise_generic_twin` before signing.

### D. Why the rule lives in two places
The floor is SQL (the in-DB door); cairn-sync is Rust on a different, spike-grade write path.
Rather than add a pgrx function (forcing an extension rebuild), the trivial rule ("non-empty
authored else derive") lives in both, each unit-tested, with a comment cross-linking them.
Accepted, documented parallel.

> **Future option (noted, not now):** unify the rule into a single pgrx function
> (`cairn_resolve_twin`) called by both the SQL floor and cairn-sync, eliminating the parallel
> at the cost of an extension rebuild on the migration. Revisit if/when more logic migrates
> back into the pgrx extension.

## Testing (TDD — failing test first)

- **`cairn-event` unit:** `resolve_twin` prefers a non-empty authored twin, derives when
  absent/whitespace; `materialise_generic_twin` yields a non-empty twin and is idempotent; a
  generic authored twin round-trips `sign`→`verify` and survives CBOR.
- **`db` integration (`cairn-node/tests/twin_globalise.rs`, PG18 + cairn_pgx):**
  - authored twin on a `note.added` passes through verbatim and `cairn_twin_is_authored` = true;
  - twin-less `note.added` → derived skeleton twin (incl. payload), `cairn_twin_is_authored` = false;
  - demographic assertion still hard-rejects a twin-less body (ADR-0034 floor regression);
  - legacy regression: existing types still submit green.
- **`cairn-sync` unit:** apply/authoring twin selection via the shared helpers (no DB).

## ADR + spec

- **New ADR-0039** (immutable): the authored twin is global; the floor honestly degrades a
  twin-less event to a flagged, payload-rendering derived skeleton; authored-vs-derived is a
  derivable projection of the signed body, not stored. Refines ADR-0012 (legibility across time)
  and ADR-0034 (demographic twin generalised to all events).
- Spec touch: §3.13 (global authored twin + honest-degradation floor) + §4.5 cross-ref;
  bump spec v0.39 → v0.40.

## Build order

ADR/spec → `cairn-event` (renderer/helpers + tests) → `db/015` + `db.rs` registration +
integration tests → `cairn-sync` mirror + tests → full workspace suite + clippy →
HANDOVER/ROADMAP refresh → commit/push/PR.

Closes the "globalise the authored twin" deferral and the db/005:29 TODO.
