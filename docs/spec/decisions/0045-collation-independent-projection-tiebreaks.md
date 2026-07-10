# ADR-0045 — Collation-independent projection winner tiebreaks

- **Status:** Accepted (refines [principle 1](../index.md#founding-principles-the-lens-for-every-decision); relates to [ADR-0031](0031-canonical-identifiers-and-node-local-surrogate-keys.md), [#115](https://github.com/cairn-ehr/cairn-ehr/issues/115))
- **Date:** 2026-07-10

## Context

The trigger-maintained projections resolve their "current winner" with a tuple comparison whose
tiebreak keys include **TEXT columns compared under the database's default collation** —
`node_origin`/`asserted_origin`, and the final total-order keys `value`/`display`/`use_key`.
Default collation is a *node-local property* (locale/ICU configuration), not a function of the
event set.

So two nodes running **different default collations** can replay the identical append-only event
set and converge on **different display winners** for an exact `(provenance_rank, hlc_wall,
hlc_counter)` tie. This is a silent violation of the set-union convergence guarantee (principle 1)
in the safety-critical projection layer. It is **not** data loss — full history is retained
append-only in `event_log`; the divergence is confined to which retained assertion is *displayed*
as current.

This is real, not merely Byzantine. [#115](https://github.com/cairn-ehr/cairn-ehr/issues/115)
hardened the same layer against a *Byzantine same-origin* HLC-triple collision (a broken signer
reusing its own `(wall, counter, origin)` triple) by appending the collation-free
`content_address` (BYTEA) as the final tiebreak, but explicitly deferred the `origin` TEXT
comparison to this issue. The **cross-origin** `(wall, counter)` tie — two *different* honest
nodes independently stamping the same millisecond wall + same counter — needs **no misbehavior at
all**, just coincidence, and it is decided by the collation-sensitive `origin` compare *before*
`content_address` is ever consulted. So the cross-origin case this ADR closes is strictly *more
likely* than the Byzantine case #115 already fixed.

## Decision

Every projection winner tiebreak comparison over a TEXT key MUST be made under `COLLATE "C"`
(byte order of the identical-on-every-node UTF-8 encoding). This applies to the shared
`cairn_hlc_overlay_wins` predicate (the five standing-state overlays: `patient_chart`,
`patient_link`, `chart_dispute`, `chart_identity_state`, `name_repudiation`) and every demographic
projection trigger and display VIEW (`patient_identifier`, `patient_demographic`, `patient_name`,
`patient_address`).

Every Cairn node stores the same UTF-8 bytes for the same signed event (Postgres
`server_encoding = UTF8`), so a `COLLATE "C"` comparison yields an identical total order on every
node regardless of that node's default/locale collation — exactly the convergence property the
projection layer needs.

### Alternatives rejected

- **`convert_to(x, 'UTF8')::bytea` comparison.** Identical result (`C` *is* byte order of the
  UTF-8 encoding), but costlier per comparison and less idiomatic than a `COLLATE` clause.
- **Carry `content_address` into the demographic projection tables**, mirroring #115's fix for the
  standing-state overlays. Truly canonical, but a schema change to five projection tables plus
  threading `content_address` through every demographic trigger — and `value`/`display` are still
  needed as columns for display regardless. Disproportionate to the defect.

## Consequences

- **Easier:** the display winner is now a deterministic function of the event set alone, not of
  node-local locale configuration — federation-wide convergence holds for the `origin` tiebreak
  step exactly as #115 already made it hold for the Byzantine `content_address` step.
- **Harder / binding on future work:** this invariant binds all future projection slices. A new
  tiebreak over a TEXT key silently reintroduces the divergence unless it is written under
  `COLLATE "C"` — reviewers of new projection triggers/VIEWs must check for this.
- **Scope:** `content_address` (BYTEA) remains the final Byzantine tiebreak for the standing-state
  overlays; this ADR only makes the `origin` step *ahead of it* collation-safe. Projection-read-side
  only — no wire/event-format/floor-gate change, no new event type, no on-wire SCHEMA change.

**Canonical home:** [sync §6.1](../sync.md#61-mechanism).
