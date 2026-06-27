# Design — Entities, relationships & the provider-number relational model (demographics §4.6, gap B remainder)

**Date:** 2026-06-27 · **Status:** design approved, pre-implementation · **Scope:** the
**provider-number person×location/org relational model** — the piece ADR-0033 / §4.4
explicitly **deferred**. Closes demographics **gap B**. Design/spec work only (ADR + spec
prose); **no code**, consistent with the gap-A/B/C sessions that preceded it.

## Problem

ADR-0033 / §4.4 settled patient-identifier *representation* and drew the
**patient-vs-professional boundary**, but deferred one piece it named explicitly: a
billing/provider number **is not a per-person fact**. In the systems the maintainer has
worked in (AU Medicare the sharpest case), a provider number is issued per
**(practitioner × practice-location)** — one clinician carries a *different* number at each
site they bill from, a third at the hospital. Neither §4.4 (subject = the patient) nor §7.5
(subject = the clinician-as-actor, a per-person identity) has a home for an identifier that
is an attribute of an **edge** between two parties.

Billing is also an unsystematic mess *within* a single country, let alone across them: some
payers scope a number to a physical location, some to a billing organisation, some to a
contract. A model that hard-codes "location" or "org" as the relational anchor would be the
same cultural-capture mistake §4.3/§4.4 avoided for addresses and identifiers.

## The agreed shape (three identifier-bearing subjects)

The maintainer's model, restated: **three independent 0:many relationships**, each carrying
its own identifier set:

| Subject | 0:many identifier set | Example systems |
|---|---|---|
| **Entity** (person) | provider identifiers | NPI, payer-assigned provider IDs |
| **Entity** (location / organisation) | location / org identifiers | Medicare facility/minor ID, private-network site ID, public-health site ID, ABN |
| **Relationship** (person × entity — the *edge*) | billing identifiers | **AU Medicare provider number** |

The decisive realisation: **each identifier set is structurally identical to the §4.4
patient-identifier set** (`value / system / normalized / profile / use·type`, set-union,
append-only, provenance ladder, hard-veto matching). The *only* thing that changes is the
**subject** the set hangs off. So gap B is *"generalise the §4.4 identifier representation
from subject = patient to subject = any entity, plus a reified relationship subject."*

### Decisions taken during brainstorming

1. **Abstract entity, not hard-coded location/org.** The relational anchor is an abstract
   **entity** with an open `kind` (`person`, `organization`, `location`, …). The maintainer
   asked for this explicitly — "an *entity* which may be a person or an org, reusable for
   other contexts too, e.g. suppliers." Naming it **entity** (not "party") because a
   *location* is identity-bearing here but is not a "party" in the classical person|org sense.
2. **Reified relationships.** The provider-at-location edge is **reified** — it has its own
   canonical identity, so identifiers attach to it and it is ended/superseded independently
   (a locum leaving Clinic A ends *that* edge; Clinic B untouched).
3. **Separate entity, one-way reference to the §7.5 actor.** A new entity registry holds
   providers/orgs/locations/suppliers. A person-entity who is also a signing clinician carries
   a **one-way `actor_ref`** to its §7.5 actor-UUID. Signing (actor) and billing (entity) stay
   structurally separate: a billing entity need not be able to sign at all (a supplier never
   signs a clinical event), and the reference confers **no** signing authority.
4. **Home: demographics §4.6** (the §4.4 work this gap belongs to), reusing the §4.4 facets,
   the §4.1 assertion stream, the §5.7/§7.5 append-only algebra *shape*, and §5.2 matching.

## Requirements (agreed)

1. **Any billing/provider-number scoping is expressible** — person×location,
   person×organisation, org×org (supply), without schema migration (kind + relationship-type
   are open data, per Cairn convention).
2. **The relational edge is first-class and independently revocable** — ending one affiliation
   never touches another; the §7.7/§5.5 revocation/contamination cascade scopes to exactly
   the affected edge.
3. **Non-conflation is structural, not conventional** — a billing/entity identifier can never
   act as a patient match key, and never as a signing credential.
4. **The same identifier value may validly recur across partitions** — see the WorkCover case
   below. The invariant is keyed by *position*, not by *value*.
5. **Identifier matching, advisory validation, honest degradation, and the culture-neutral
   floor all reuse §4.4 verbatim** within a subject partition.
6. **Every entity/relationship assertion carries the mandatory §4.5 legibility twin**,
   profile-independent, materialised at authoring.

Out of scope (YAGNI): a closed relationship-type taxonomy (only the billing edge is worked;
the rest is open vocabulary added as data); an organisation-grouping layer *above* location
(addable later as another entity + edge by the same pattern); validity/expiry mechanics
beyond what the assertion stream + supersession already give.

## §1 — The entity (identity-bearing, non-patient, non-signing)

A first-class append-only **entity**, canonical-identified (UUIDv7 + multihash, ADR-0031)
like patients/actors, asserted through the §4.1 stream. It carries:

- **`kind`** — recommended-but-open vocabulary (`person`, `organization`, `location`, …);
  the core never branches on an unknown kind beyond partitioning.
- A 0:many **identifier set** — the §4.4 facets **verbatim** (`value / system / normalized /
  profile / use·type`), set-union, append-only, provenance ladder. Person→provider IDs;
  location→facility/site IDs; organisation→ABN/registration IDs.
- The mandatory **§4.5 legibility twin**; an optional **§4.3 address** facet (esp. for
  location/organisation entities).
- For a person-entity who is also a signing clinician: an optional **one-way `actor_ref`** to
  its §7.5 actor-UUID. Points entity→actor only; **confers no signing authority**; absent for
  suppliers/orgs/locations. A wrong `actor_ref` is repaired by an append-only overlay
  (never-merge-always-link, principle 2), like any other assertion.

## §2 — The reified relationship (where billing numbers live)

A **relationship** is a reified, directional, typed edge between two entities, with its own
canonical identity, asserted append-only:

- **`from` / `to`** (entity-UUIDs) + **`type`** — recommended-but-open (`practices-at`,
  `bills-under`, `employed-by`, `supplies`, …).
- Its **own** 0:many **identifier set** (same §4.4 facets) — **this is the AU Medicare
  provider number**: a `medicare-provider` identifier on the (person `practices-at` location)
  edge. The billing mess is absorbed because the edge terminates at *any* entity:
  person×location for location-scoped payers, person×organisation for org-scoped ones,
  org×org for supply.
- **Validity** is asserted / superseded / ended through the append-only algebra shape borrowed
  from §5.7 / §7.5 (`assert / supersede / revoke / end`), **never overwritten**. Because the
  edge is reified, a locum leaving Clinic A ends *that* relationship; Clinic B is untouched,
  and the §7.7 / §5.5 revocation cascade scopes to exactly that edge.

## §3 — Subject-kind partitioning (the load-bearing safety property)

The ADR-0033 non-conflation invariant becomes **structural, not conventional**:

- Every identifier set is tagged `(subject_kind, subject_uuid)`, with
  `subject_kind ∈ {patient, entity, relationship}` (patient = the existing §4.4 sets).
- **Matching only ever compares identifiers within one `subject_kind` partition.** The §5.2
  patient-match pipeline pulls `subject_kind = patient` and **can never see** a billing
  number. Provider de-duplication pulls `entity, kind = person`. The §4.4 hard veto
  (same `system`, different `normalized`) applies **within** a partition only.
- A billing/entity identifier **never authorizes signing** — only §7.5 actor keys sign;
  `actor_ref` is one-way and non-authorizing.

So a billing number cannot become a patient match key (wrong partition) nor a signing
credential (only actors sign) — **by construction**, not by convention.

### Position, not value — the WorkCover case

The invariant is keyed by **where an identifier sits**, never by its value. The **same**
AHPRA registration string may validly appear in **two** partitions at once:

- as a **licensure / authority credential** on the §7.5 actor (the "who may sign in this
  role" facet), **and**
- as a **billing identifier** in §4.6 (e.g. a WorkCover insurance case that *requires* the
  treating clinician's AHPRA number cited on the claim).

This is **not** conflation. Conflation would be one partition slot serving two roles; the same
value recurring across two partitions for two distinct purposes is exactly what the model must
allow. *Who signs* (actor credential) stays distinct from *who/how billed* (entity ·
relationship identifier), even when the underlying number is identical.

## §4 — Matching, honest degradation, the in-DB floor

- **Identifier matching reuses §4.4 verbatim within a partition** — hard veto = same `system`
  / different `normalized`, **forces a human decision, never auto-link / auto-reject**; honest
  degradation when the normalizer profile is absent (string-equal = positive signal,
  string-inequality = *hold for review*, never a mis-fired veto); `system: unknown` never
  vetoes.
- **§4.5 twin mandatory** on every entity/relationship assertion, profile-independent,
  materialised at authoring. Examples:
  *"Organization: Top End Medical Clinic; ABN 12 345 678 901"*;
  *"Relationship (practices-at): Dr Smith → Top End Medical Clinic; Medicare provider 2426789A, from 2024-03."*
- **In-DB floor (culture-neutral, unbypassable, principle 12):**
  - an entity carries a text `kind` + a non-empty twin;
  - an identifier-set element obeys the §4.4 structural rules (`value` non-empty text;
    `system` present; `normalized` is text when present; `normalized` materialised ⇒ `profile`
    named);
  - a relationship names two **existing** entity-UUIDs + a `type` + a non-empty twin;
  - **the `subject_kind` tag is present and immutable** — so cross-partition comparison is
    structurally impossible.
  - The floor **never** holds a profile, runs a checksum, validates a billing format, branches
    on an unknown `kind`/`type`, or rejects on validation.
- **Cross-facet verification stays advisory** — a profile-holding node may re-derive
  `normalized` / the twin and flag drift, never as a floor gate (same treatment as
  §4.3/§4.4/§4.5).

## Safety classification (defect blast radius, §9)

- **Fit-for-purpose (Python/UI-tier acceptable):** the entity/relationship *data* itself. A
  wrong provider number yields a **rejected claim**, caught immediately — it does not corrupt
  the clinical record, mis-merge a patient, or leak data.
- **Safety-critical (in-DB / Rust trusted base):** the **`subject_kind` partition tag**, the
  **one-way non-authorizing `actor_ref`**, and the **append-only floor invariants** — these
  are what prevent conflation (a billing number acting as a match key or a signing credential)
  and key-leakage equivalents. They sit beside the §4.4 floor and the §5.2 matcher boundary.

## Spec & ADR changes

- **New §4.6 "Entities, relationships, and provider identifiers"** in `demographics.md`: the
  entity model, the reified relationship, subject-kind partitioning, the WorkCover
  position-not-value example, matching/degradation reuse, floor invariants, safety class.
- **Update the §4.4 professional-ID boundary paragraph** — its "provider number is relational
  … that model is deferred" sentence now points to §4.6 (no longer deferred).
- **Cross-ref from identity §5.2** (matching) to the subject-kind partition rule; from §7.5
  (actor registry) to the one-way `actor_ref` + the position-not-value non-conflation note;
  from §7.7 (revocation cascade) to per-edge scoping.
- **New ADR-0035** "Entities, relationships, and the provider-number relational model" —
  **refines ADR-0033** (closes its deferred provider-number model) and **ADR-0014** (reuses
  the profile/distribution-plane bundle). Records the four brainstorming decisions, subject-kind
  partitioning as structural non-conflation, position-not-value, and the fit-for-purpose-data /
  safety-critical-partition split.
- **Bump spec 0.35 → 0.36** in `index.md`; add ADR-0035 to the decisions index + mkdocs nav.
- **HANDOVER / ROADMAP** currency: demographics gap B fully closed; Phase 4 "provider-number
  person×org model" follow-on resolved.

## Why no new founding principle

This is an application of existing principles — 1 (evidence preservation / append-only),
2 (never merge/erase — the `actor_ref` overlay), 4 (advisory uncertainty / `unknown`),
11 (legibility twin), 12 (culture-neutral floor) — plus reuse of ADR-0014's profile machinery
and the §5.7/§7.5 algebra shape. The one genuinely new structural element — the `subject_kind`
partition tag making ADR-0033's non-conflation *enforced* rather than *stated* — is a
safety-preserving refinement of an existing invariant, not a new architectural axis. **No new
founding principle.**

## Testability (checkable invariants for when the floor + matcher are implemented)

- **Floor:** rejects an entity with no `kind` or empty twin; rejects a relationship naming a
  non-existent entity-UUID, or with no `type` / empty twin; rejects an identifier element that
  breaks the §4.4 structural rules; **rejects mutation of the `subject_kind` tag**; never
  rejects on a bad checksum or unknown `kind`/`type`.
- **Partitioning:** a `medicare-provider` identifier on a relationship is **never** returned to
  the §5.2 patient-match pipeline; a same-`system` veto fires only within one partition.
- **`actor_ref`:** present only on `kind = person` entities; absent paths never grant signing;
  a wrong `actor_ref` is correctable by overlay with no data loss.
- **Position-not-value:** the same AHPRA value is accepted simultaneously as a §7.5 actor
  credential and a §4.6 billing identifier without either being treated as the other's role.
- **Per-edge revocation:** ending the (person, Clinic-A) relationship leaves the
  (person, Clinic-B) relationship and its identifiers intact.
