# Design — The demographic legibility twin (demographics §4, gap C)

**Date:** 2026-06-27 · **Status:** design approved, pre-implementation · **Scope:**
demographics §4 — bind every demographic assertion to the principle-11 legibility
twin (§3.13 / ADR-0012), generalizing the ad-hoc address `display` / identifier
`value` facets into one uniform, profile-independent rule. Prose-only spec change +
new ADR-0034. No new founding principle, no new envelope field.

## Problem

§3.13 ([ADR-0012](../../spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md))
mandates a signed, mechanically-derived plaintext **legibility twin on every event**, so a
node generations behind — or lacking a profile — can still *read* the event as a clinician
reads a progress note ([principle 11](../../spec/index.md)). Demographic assertions
([§4.1](../../spec/demographics.md#41-demographic-assertions)) **are** events, so at the
envelope level they already inherit the twin. But §4 never says so, and the two
representation gaps closed this session each invented a *field-level* facet that overlaps
the *event-level* twin without reconciling them:

- §4.3 calls the address **`display`** facet "the principle-11 legibility twin"
  (materialised at authoring, profile-independent).
- §4.4 calls the identifier **`value`** facet "the principle-11 legibility analogue."

This leaves three real gaps:

1. **Unreconciled levels.** A field-level facet (`display`/`value`) and an event-level twin
   (§3.13) are never tied together — a reader cannot tell whether they are the same twin or
   two things that can drift.
2. **Most fields have no stated twin.** Names, DOB, sex/gender, phone, deceased, photo
   (§4.2) carry no legibility-twin statement at all.
3. **No forward guarantee.** Nothing forces a *future* jurisdiction-defined demographic field
   shape (a culture-specific name structure, a novel profiled field) to be legible without its
   profile. That gap is exactly how principle 11 silently regresses for demographics: someone
   adds a profile-dependent field, a profile-less node renders it as opaque structured noise,
   and no rule was violated.

Gap C closes all three with **one unifying rule**. The deliverable is a *generalization/
unification* of an existing mechanism (§3.13), not a new mechanism — but it is a can't-forget
day-one discipline (like additive-only evolution), so it is recorded as an ADR in the
[0032](../../spec/decisions/0032-culture-neutral-address-representation.md)/[0033](../../spec/decisions/0033-patient-identifier-representation.md)
cadence.

## Requirements (agreed)

For every demographic assertion, regardless of field, issuing system, jurisdiction, or which
nodes hold which profiles/schemas:

1. **The demographic fact is human-readable on any node** — a node lacking the field's profile
   (or generations of schema behind) can still read *what was asserted about whom*
   (principle 11).
2. **Uniform, no exceptions** — the twin is mandatory on *every* demographic assertion; a
   self-legible scalar (a name string, a DOB) is its own twin (the value's plaintext
   rendering). Uniformity is what prevents the silent regression on a future field shape.
3. **Profile-independent and materialised at authoring** — the twin never requires the field's
   profile/schema to render; it is carried in the signed event, so it survives honest
   degradation.
4. **The floor stays culture-neutral** (principle 12) — it enforces only the structural
   invariant (a non-empty twin is present), never validates twin *content*, never holds a
   profile.
5. **Legibility is not matching** — the twin is for reading only; it is never a matching
   shortcut. Matching keys (`normalized`, `geo`, structured `parts`) stay separate and
   continue to degrade to human review per ADR-0032/0033.

Explicitly **out of scope:** any change to matching behaviour; any new envelope field; the
generic descriptor-driven renderer (§3.13 Rung 1, deferred there); the provider-number
person×org model (gap B remainder, still open).

## The rule (new demographics §4.5 — "The demographic legibility twin")

A demographic assertion is a §4.1 event, so it **already carries the mandatory §3.13 signed,
mechanically-derived plaintext legibility twin.** §4.5 binds demographics to that invariant
and adds the two demographic-specific requirements the address case discovered:

- **The twin renders *this demographic fact* as profile-independent plaintext** — field +
  human-readable value + `use`/provenance context. Examples:
  - *"Address (residential), document-verified: 12 Smith St, Darwin NT 0800, Australia"*
  - *"NHS number, document-verified: 943 476 5919"*
  - *"Name (legal): 田中 太郎"*
  - *"Date of birth (patient-stated): about 1980 (year only)"*
- **It is materialised at authoring and profile-independent** — never requires the field's
  profile/schema to render — so a node lacking that profile, or far behind on schema, still
  reads the fact. This generalizes §4.3's "materialise `display` into the signed event at
  authoring" from one field to all of §4.

## Reconciling the existing facets (the §4.3/§4.4 prose fix)

`display` (address) and `value` (identifier) are **re-described as the value-core the §3.13
twin wraps for those fields** — named *instances* of the one rule, not separate twins:

- §4.3: `display` stops calling itself "the legibility twin" and becomes "the **value-core**
  realizing the §4.5 demographic legibility twin for an address" (still mandatory, still
  materialised — no behavioural change).
- §4.4: `value` stops calling itself "the principle-11 legibility analogue" and becomes "the
  **value-core** realizing the §4.5 twin for an identifier."
- For self-legible scalars (name, DOB, phone, deceased status) the value-core is the value's
  own plaintext rendering — the twin is mechanically that, no extra facet needed.

This removes the "is this the same twin or two twins?" ambiguity and aligns with §3.15's
"one twin, cannot diverge" framing.

## Forward guarantee (the point of the ADR)

Any **future** jurisdiction-defined demographic field shape inherits §4.5 by construction: it
cannot be introduced in a form a profile-less node renders as opaque structured noise, because
the twin is mandatory and profile-independent on every assertion. This is principle 11 made
un-forgettable for demographics, the demographic analogue of ADR-0012's additive-only schema
evolution.

## Floor & verification (mirrors §4.3/§4.4 exactly)

- **Floor (culture-neutral, principle 12):** enforces only the structural invariant — *every
  demographic assertion carries a non-empty plaintext twin*. It never validates twin *content*,
  never holds a profile, never runs a formatter/normalizer.
- **Advisory cross-facet check:** a profile-holding node may re-derive the twin from the
  structured value and flag drift (`twin == render(value / parts)`), advisory only, never a
  floor gate — the same treatment §4.3 gives `display == formatter(parts)` and §4.4 gives
  `normalized == normalizer(value)`.

## Boundary (legibility ≠ matching)

Stated as an explicit non-goal so nobody routes a veto or auto-link through twin text: the twin
is for **reading**. A profile-less node reads the twin but **still** degrades matching to human
review per ADR-0032/0033. The matching keys (`normalized`, `geo`, structured `parts`) are
separate from the twin and unchanged by this ADR. The twin is never evidence for or against a
link.

## Spec changes

- **New §4.5 "The demographic legibility twin"** in `demographics.md`: the rule, the
  reconciliation of `display`/`value` as value-cores, the forward guarantee, floor +
  advisory verification, the legibility≠matching boundary.
- **Light prose fix in §4.3** — `display` re-described as the value-core realizing the §4.5
  twin (no behavioural change).
- **Light prose fix in §4.4** — `value` re-described as the value-core realizing the §4.5
  twin (no behavioural change).
- **§3.13 cross-ref** — note demographic assertions as a twin-bearing event class (so the
  binding is discoverable from the canonical twin home, not only from §4).
- **New ADR-0034** — the demographic legibility twin: unifying rule + reconciliation +
  forward guarantee. **Refines ADR-0012**; adjacent to ADR-0032/0033.
- **Bump spec 0.34 → 0.35** in `index.md`.
- **HANDOVER/ROADMAP** currency: gap C closed; gap B provider-number person×org model remains
  the named demographics follow-on.

## Why no new founding principle

This is an application of existing principles (11 legibility-across-time, 12 culture-neutral
floor) reusing ADR-0012's twin mechanism. No new envelope field (the §3.13 twin already
exists). The contribution is *unification + a forward guarantee*, not a new architectural
axis. No new founding principle.

## Testability

Prose change, but each rule is stated as a checkable invariant for when the floor + matcher
are implemented:

- **Floor:** rejects a demographic assertion with an empty/absent twin; accepts any non-empty
  twin regardless of content; never rejects on twin-vs-value mismatch (that is advisory).
- **Legibility:** a node lacking a field's profile still renders the materialised twin for an
  address, an identifier, and a (hypothetical) future profiled field — none degrade to opaque
  structured noise.
- **Reconciliation:** an address assertion's twin is built around `display`; an identifier's
  around `value`; a name/DOB assertion's twin is the value's own rendering — one twin per
  assertion, no second twin.
- **Boundary:** a profile-less node that can read the twin still routes matching for that field
  to human review (twin readability never upgrades a match decision).
