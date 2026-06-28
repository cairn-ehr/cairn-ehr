# ADR-0037 — Administrative-sex provenance-first, per-field winner-policy selector, and karyotype as distinct field

- **Status:** Accepted
- **Date:** 2026-06-28
- **Refines:** [§4.2](../demographics.md#42-per-field-projection-policy), [ADR-0036](0036-demographic-name-display-recency-first.md), [ADR-0014](0014-locale-pluggable-matcher-comparators.md)

## Context

[§4.2](../demographics.md#42-per-field-projection-policy) names three sex/gender fields — sex-at-birth,
administrative sex, and gender identity — but leaves the administrative-sex projection rule unspecified.
The table says sex-at-birth is provenance-locked and gender identity is patient-stated/recency-first, with
nothing stated for administrative sex.

Two subsidiary questions were deferred from slice-2:

1. **Which rule governs administrative sex?** The field sits between two extremes: sex-at-birth (a stable
   biological fact, provenance-locked) and gender identity (a legitimately moving, patient-authoritative
   field, recency-first). Administrative sex is the marker that appears on legal and administrative
   documents — a driver's licence, a passport, an insurance card — and it legitimately changes when a
   patient updates their legal documents. The right projection rule depends on the *semantics* of
   administrative sex, not on a desire to be consistent with either neighbour.

2. **How is the winner ordering encoded?** [ADR-0036](0036-demographic-name-display-recency-first.md)
   introduced recency-first as a second distinct ordering alongside provenance-first (DOB). Both orderings
   are now in use. Slice-2's implementation hard-coded the ordering per field; as a third field is settled,
   a generalised mechanism is needed — one that is the authoritative source for both the projection gate
   and the winner ordering, and that future volatile fields (phone, preferred contact method) can plug into
   as recency-first without further ad hoc decisions.

3. **Does a karyotype (chromosomal sex) displace the sex-at-birth field?** Slice-2 left open whether
   a `fact-proven` karyotype result — which sits at the top of the [§4.1](../demographics.md#41-demographic-assertions)
   provenance ladder — would, under a provenance-first rule, displace sex-at-birth. The AIS/Swyer case
   illustrates the problem: a patient assigned female at birth whose later karyotype reveals XY chromosomes
   holds *two distinct facts* about themselves, not one fact correcting another. Conflating karyotype with
   sex-at-birth in the same field would force a false correction where none exists.

## Decision

### Part 1: Administrative sex is provenance-first

The administrative-sex projection rule is **provenance-first** — the same ordering as DOB and sex-at-birth.
Rationale: administrative sex is a document-anchored marker; it is what an official document records as the
patient's sex, and a document-verified assertion must not be displaced by a later unverified self-claim about
the *same* document-based marker.

Recency still resolves ties among equal-provenance assertions: a newer legal document (a passport reissued
after legal sex change) carries a higher HLC timestamp and wins over an older document of equal provenance.
So the full rule is **provenance-first; recency wins among equal provenance**.

The **dignity and patient-authoritative surface** for sex and gender is carried by **gender identity**,
which remains recency-first and patient-stated authoritative. The split between administrative-sex
(provenance-first, document-anchored) and gender identity (recency-first, patient-authoritative) is
**deliberate by design**. Administrative sex is the answer to *"what do your legal documents say?"*;
gender identity is the answer to *"how do you identify?"* — these are different questions with different
appropriate updating semantics, and aligning them to the same rule would erase that distinction.

### Part 2: Per-field winner-policy selector

A single immutable function, **`cairn_demographic_field_policy(field)`**, returns the authoritative winner
policy for each demographic field:

- `provenance-first` — provenance rank wins; recency resolves ties among equal-provenance assertions.
- `recency-first` — HLC timestamp wins; provenance and origin break ties among exact-same-HLC entries.
- `NULL` — the field is **carried-not-projected** (the [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)
  honest-degrade: a node carrying assertions for a field it cannot yet project renders `NULL` and degrades
  gracefully rather than silently dropping or mis-projecting the data).

This function is the **single source of truth** for both:

- the **projection gate**: a `NULL` return means the node carries but does not project the field;
- the **winner ordering**: `provenance-first` or `recency-first` determines how the projection selects the
  display winner from the retained assertion set.

This generalises slice-2's per-field hard-coded ordering into a reusable mechanism. Future volatile fields
(phone, preferred contact method) plug in as `recency-first`; future fields that should not yet be projected
plug in as `NULL`; no further ad hoc winner decisions are needed.

**The function is immutable and in-database.** It cannot be overridden by client code — the winner policy
is a floor property, not a UI soft-policy choice ([principle 12](../index.md#founding-principles-the-lens-for-every-decision),
[ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md)). The set of recognised field names and their
policies evolves **additively** ([principle 11](../index.md#founding-principles-the-lens-for-every-decision)):
a new field is added with its policy; an existing field's policy is never changed without a new ADR.

### Part 3: Karyotype is a distinct field, never a sex-at-birth value

Sex-at-birth is defined as the **sex assigned or observed at birth**. It is not a chromosomal fact — it is
a clinical/administrative determination made at birth, recorded in birth documents, and may be based on
observable anatomy rather than chromosome analysis.

A **karyotype** (chromosomal sex) is a *different* fact: a laboratory result that establishes the
patient's chromosomal constitution. It has its own future field in §4.2 and is **never asserted as a
sex-at-birth value**, even when it falls at the top of the §4.1 provenance ladder as `fact-proven`.

The AIS/Swyer case demonstrates why this matters. A patient assigned female at birth — with female anatomy
at birth — whose later karyotype reveals XY chromosomes holds **two facts that are both true and
non-contradicting**: a sex-at-birth of female (the birth determination) and a karyotype of XY (the
chromosomal finding). Asserting the karyotype as a sex-at-birth `fact-proven` value would force a
correction where the original was not wrong — it was accurate as a birth determination, and the chromosomal
finding does not retroactively make it an error. Recording two facts is the correct model.

**Mechanism:** the `fact-proven` rung remains in the [§4.1](../demographics.md#41-demographic-assertions)
provenance ladder for same-field laboratory confirmation (a blood type re-confirmed by a second assay is a
legitimate same-field `fact-proven` assertion). The projection path by which a `fact-proven` assertion
displaces a prior value within the same field **stays mechanically present** in the floor — it is not
removed. However, well-formed input never places a karyotype result in the sex-at-birth field: this is a
**modelling convention enforced as UI soft-policy**, not a floor gate ([principle 12](../index.md#founding-principles-the-lens-for-every-decision)).
The floor does not know about karyotype; the UI does — and it routes the karyotype result to the karyotype
field, not to sex-at-birth.

## Consequences

- **Additive only:** no event types, floor gates, or table-schema columns change. The three parts of this
  decision are:
  - a **policy encoding** (administrative-sex = provenance-first, added to `cairn_demographic_field_policy`);
  - a **generalised mechanism** (the per-field winner-policy selector replaces per-field hard-coding);
  - a **modelling convention** (karyotype routes to its own field, enforced in the UI not the floor).
- **The dignity/recency surface is preserved.** The gender-identity field remains recency-first and
  patient-authoritative. Administrative sex being provenance-first does not affect a patient's ability to
  assert and have displayed their current gender identity.
- **Recency-first is now the second recognised ordering.** ADR-0036 introduced it for names; this ADR
  generalises it as a first-class policy option alongside provenance-first.
- **Future fields have a clean plug-in path.** Any new demographic field with an obvious winner policy
  adds one row to `cairn_demographic_field_policy` with no architectural discussion needed.
- **The AIS/Swyer case records correctly.** Two facts are recorded in two fields; neither overwrites the
  other; both are retained in the append-only log; the matcher uses both as distinct evidence.
- **`fact-proven` in sex-at-birth remains valid.** A sex-at-birth initially recorded as
  `clinician-observed` and later confirmed by neonatal screening is a legitimate same-field
  `fact-proven` assertion — the rung is not removed, only the mis-routing of karyotype results to it.
- **The bet:** provenance-first for administrative sex means an unverified patient claim cannot displace a
  document-verified marker. The downside risk is that a patient who has legally changed their administrative
  sex marker but has not yet presented updated documents at this clinic will see the old marker in their
  chart. Mitigation: the gender-identity field — patient-authoritative and recency-first — is where the
  patient's current identity appears; the administrative-sex field tracks what the documents say. A UI
  may surface both. A patient who has updated their documents and presents them triggers a new
  document-verified assertion that displaces the old one under the provenance-first / recency-among-equals
  rule.
- **How we'd know the bet fails:** systematic reports of patients seeing a stale administrative-sex marker
  after updating their legal documents, where the root cause is the provenance-first rule holding an older
  document-verified value over a correct patient-stated one. This pattern would motivate re-examining
  whether administrative sex should move to recency-first (requiring a new superseding ADR) or whether a
  UI-layer review flag is sufficient.
