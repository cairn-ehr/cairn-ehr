# ADR-0038 — Demographic address display: per-use recency-first

- **Status:** Accepted
- **Date:** 2026-06-28
- **Refines:** [ADR-0032](0032-culture-neutral-address-representation.md) (representation),
  follows [ADR-0036](0036-demographic-name-display-recency-first.md) (volatile-field logic)

## Context

[ADR-0032](0032-culture-neutral-address-representation.md) fixed the address *representation*
(the three-facet value: mandatory `display`, optional `geo`, optional `structured`) but
deliberately left the *projection* — which assertion is the "current" address — open, calling
the thin "recency wins" treatment a matching statement, not a projection one. The §4.3 summary
table, written the same day, said the per-use current address was *"highest-provenance
most-recent"* — provenance-first, the DOB lock. That predates the names slice
([ADR-0036](0036-demographic-name-display-recency-first.md)), which established that a
**volatile, legitimately-changing field must be recency-first**, or a stale verified value pins
over the current truth (the deadname / stale-married-name failure).

## Decision

The per-`use` current address is **recency-first**: within a `use`, the newest assertion wins
(HLC wall then counter), with `provenance_rank` then `asserted_origin` as deterministic
tiebreaks. Address is the archetypal volatile field — people move — so a fresh patient-stated
"I moved last month" must displace a stale document-verified address; that is *where you would
send an ambulance or a letter*. This is the same reasoning ADR-0036 applied to names, and the
deliberate inverse of DOB's provenance-lock ([ADR-0037](0037-demographic-administrative-sex-and-per-field-winner-policy.md)).

The projection is a **retained set** (`patient_address`, keyed `(patient, use, display)`) plus a
per-use display-winner VIEW (`patient_address_current`, one row per `(patient, use)`) — **one
current address per use**; residential, postal, and work are independently current. There is no
legal-tier preference (unlike names) and no cross-use fallback; the UI surfaces past or other-use
addresses from the retained set. All addresses are retained as evidence regardless of which one
displays; provenance still feeds the later [§5.2](../identity.md) matcher.

## Consequences

- **Easier:** the current address always reflects the latest claim about where the patient is;
  address history is intact (append-only — "moved out" is a new assertion, never an overwrite);
  the projection reuses the names machinery verbatim (no new event type, additive floor branch).
- **The bet / trade:** a recency-first winner trusts the newest assertion even when lower
  provenance, so a mistaken or malicious fresh assertion can transiently displace a good one — but
  it is **overlay, never erasure** (the displaced address is retained, attributable, and re-assertable),
  and the matcher reads the whole set, not just the winner.
- **Explicitly out of scope (deferred):** an explicit address supersession/unlink event (the
  append-only set + recency covers "moved"); the §5.2 comparator using the address `profile`;
  advisory validators (lat/lon bounds, `display == formatter(parts)` drift, profile re-derivation).
