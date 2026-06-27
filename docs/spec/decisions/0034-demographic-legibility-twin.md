# ADR-0034 — The demographic legibility twin: every demographic assertion stays human-readable without its profile

- **Status:** Accepted
- **Date:** 2026-06-27
- **Refines:** [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)

## Context

[§3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin) ([ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)) mandates a signed, mechanically-derived plaintext **legibility twin on every event**, so a node generations behind — or lacking a profile — can still *read* the event as a clinician reads a progress note ([principle 11](../index.md#founding-principles-the-lens-for-every-decision)). Demographic assertions ([§4.1](../demographics.md#41-demographic-assertions)) **are** events, so they already inherit the twin at the envelope level. But §4 never said so, and the two representation gaps closed alongside this one each invented a **field-level** facet that overlaps the **event-level** twin without reconciling them:

- [ADR-0032](0032-culture-neutral-address-representation.md) called the address **`display`** facet "the principle-11 legibility twin" (materialised at authoring, profile-independent).
- [ADR-0033](0033-patient-identifier-representation.md) called the identifier **`value`** facet "the principle-11 legibility analogue."

Three gaps remain. **(1) Unreconciled levels** — a reader cannot tell whether the field-level facet and the §3.13 event twin are one thing or two that can drift. **(2) Most fields have no stated twin** — names, DOB, sex/gender, phone, deceased status, photo ([§4.2](../demographics.md#42-per-field-projection-policy)) carry no legibility statement. **(3) No forward guarantee** — nothing forces a *future* jurisdiction-defined demographic field shape to be legible without its profile, which is exactly how [principle 11](../index.md#founding-principles-the-lens-for-every-decision) silently regresses for demographics: a profile-dependent field is added, a profile-less node renders it as opaque structured noise, and no rule was broken.

## Decision

Demographics is bound to the §3.13 legibility twin by one **uniform rule** — canonical home [demographics §4.5](../demographics.md#45-the-demographic-legibility-twin); the twin mechanism itself is unchanged from [§3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin).

1. **Every demographic assertion carries the §3.13 twin — no exceptions.** A demographic assertion is a [§4.1](../demographics.md#41-demographic-assertions) event, so it already carries the mandatory signed, mechanically-derived plaintext twin. The twin renders **this demographic fact** as profile-independent plaintext — field + human-readable value + `use`/provenance context (*"Address (residential), document-verified: 12 Smith St, Darwin NT 0800, Australia"*; *"NHS number, document-verified: 943 476 5919"*; *"Date of birth (patient-stated): about 1980 (year only)"* — imprecise facts ([principle 4](../index.md#founding-principles-the-lens-for-every-decision)) render legibly too). For a self-legible scalar (a name string, a DOB) the twin is mechanically the value's own plaintext rendering — the uniformity, not the redundancy, is the point.

2. **It is materialised at authoring and profile-independent.** The twin never requires the field's profile/schema to render, and is carried in the signed event — generalizing [ADR-0032](0032-culture-neutral-address-representation.md)'s "materialise `display` into the signed event at authoring" from one field to all of §4.

3. **`display` and `value` are named instances, not separate twins.** [§4.3](../demographics.md#43-address-the-three-facet-value) `display` and [§4.4](../demographics.md#44-identifiers-representation) `value` are the **value-core** the twin wraps for those fields. There is one twin per assertion; it cannot diverge from a second ([§3.15](../data-model.md#315-the-active-write-model-thin-encounters-co-produced-legibility-and-the-delete-vs-erase-distinction) "one twin, born at authoring").

4. **Forward guarantee.** Any future jurisdiction-defined demographic field shape inherits this rule by construction: it cannot be introduced in a form a profile-less node renders as opaque noise. This is [principle 11](../index.md#founding-principles-the-lens-for-every-decision) made un-forgettable for demographics — the demographic analogue of ADR-0012's additive-only schema evolution.

5. **Culture-neutral floor; advisory verification** ([principle 12](../index.md#founding-principles-the-lens-for-every-decision)). The in-DB floor enforces only the structural invariant — *every demographic assertion carries a non-empty plaintext twin* — and never validates the twin's content, never holds a profile, never runs a formatter. A profile-holding node may re-derive the twin from the structured value and flag drift (`twin == render(value / parts)`), advisory only, never a floor gate — the same treatment §4.3 gives `display == formatter(parts)` and §4.4 gives `normalized == normalizer(value)`.

6. **Legibility is not matching.** The twin is for **reading**, never a matching shortcut. A profile-less node reads the twin but **still** degrades matching to human review per [ADR-0032](0032-culture-neutral-address-representation.md)/[ADR-0033](0033-patient-identifier-representation.md) and [identity §5.2](../identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split). The matching keys (`normalized`, `geo`, structured `parts`) stay separate from the twin and are unchanged here. Twin readability never upgrades or downgrades a link decision.

## Consequences

- **Easier:** any future demographic field shape is legible on any node by construction; the field-level/event-level twin ambiguity is resolved (one twin per assertion); names/DOB/phone gain an explicit legibility statement they lacked; auditing/RAG/full-text over demographics inherits the §3.13 substrate for free.
- **Harder / the bet:** authoring code must materialise a faithful plaintext twin for *every* demographic field, including future ones — the discipline ADR-0032 applied to `display`, now fleet-wide for §4. We bet this is cheap (the twin already had to exist per §3.13) and that mechanical derivation keeps twin and value from drifting (the same §3.13 bet).
- **How we'd know the bet fails:** a demographic assertion is observed whose twin requires a profile to render (a profile-less node shows opaque noise — the rule was violated at authoring); or twin and structured value drift in practice despite mechanical derivation (poisoning audit/RAG — the §3.13 risk, surfaced by the advisory cross-facet check).
- **No new founding principle; no new envelope field.** This is an application of [principle 11](../index.md#founding-principles-the-lens-for-every-decision) (legibility across time) and [principle 12](../index.md#founding-principles-the-lens-for-every-decision) (culture-neutral floor) reusing the existing [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md) twin mechanism. The contribution is unification + a forward guarantee, not a new mechanism.
