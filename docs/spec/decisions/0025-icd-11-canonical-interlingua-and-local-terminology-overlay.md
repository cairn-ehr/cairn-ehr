# ADR-0025 — ICD-11 as the canonical classification interlingua and the local-terminology overlay

- **Status:** Accepted
- **Date:** 2026-06-19

## Context

Medication is as central as disease, and both demand the same discipline: **the decision-making pathway must key
on stable concept identifiers, never on free-text names** that are spelled differently across sources and drift
over time. The drug axis resolved this by anchoring substance identity on the WHO INN
([ecosystem eval 0003](../../ecosystem/0003-reference-data-sourcing-medicines-and-terminologies.md)). This ADR
settles the morbidity/injury axis — the disease/injury classification — which that same evaluation surfaced as an
architectural commitment (the evaluation only flags; ratification happens here).

**Why ICD-11, and why not SNOMED.** SNOMED CT is clinically the richest option but is excluded on the mission: it
is member/affiliate-gated, charges fees in non-member territories, and forbids sub-licensee redistribution — a
paywall around a public good, the identical defect that put AMT out in eval 0003 §3. ICD-11 is chosen as the
canonical worldwide pivot because it is the one classification whose identifiers are stable enough to anchor an
immortal append-only record: **persistent entity URIs** rooted at `https://id.who.int/icd/entity/{id}`, codeable
MMS **stem codes**, and a **free official offline Docker container** (`whoicd/icd-api`) that mirrors those URIs
locally with no cloud dependency — a clean fit for the fractal-topology node
([ADR-0001](0001-fat-postgres-thin-daemon.md)) and the availability floor.

Four forces shape *how* it is stored, not just *which* one:

- **Stable-identifier requirement (principle 11).** A coded event must stay legible and re-aggregatable for as
  long as it exists, across ICD-11's annual revisions. The durable thing on the event must be an identifier, not
  a label.
- **The CC BY-ND licensing reality (eval 0003 §8).** WHO ICD-11 is **CC BY-ND 3.0 IGO**: codes/URIs may be
  redistributed verbatim, commercially, with attribution — but Cairn **may not ship its own ICD-11↔X crosswalks**
  (translations and adaptations need a separate WHO agreement). A mapping Cairn cannot freely own is one it must
  be able to **recompute** — which is only possible if the clinician's source assertion is retained as a
  first-class fact, not flattened to an annotation.
- **Clinician acceptance is a paper-parity problem (principle 3).** A state-mandated coding system imposed at the
  point of write is resented and gamed. Validating case (Norway, surgical coding): surgeons met the mandated
  procedure-coding system with passive aggression and endless friction. What flipped adoption was a thin
  translation layer that **captured the surgeon's own free-text term, checked whether that term was already
  mapped to the government code, forced a mapping exactly once for a novel term, then remembered it forever and
  translated silently thereafter.** Universally liked once in use. The lesson: pay the coding cost **once per
  novel term, at the natural moment**, never per write. **Corollary (the second Norway lesson): never *force* a
  guess when the clinician is unsure.** A clinician pressed to pick a code they cannot vouch for manufactures a
  precise-untruth binding that a professional coder must later unpick — coding debt. Where a professional coder is
  eventually in the loop, an explicitly **open** mapping (deferred to that coder) beats a forced one.
- **Coding cannot be a blocking required field (principle 4).** An uncoded or free-text diagnosis must be
  recordable; no required field may be satisfiable only by fabrication
  ([§3.7](../data-model.md#37-acknowledged-uncertainty-uncertainty-capable-value-types)). Paper let you write a
  diagnosis you could not yet code.

## Decision

Canonical home: **[data-model §3.16](../data-model.md#316-clinical-concept-coding-the-icd-11-interlingua-and-the-local-terminology-overlay)**.

1. **ICD-11 is the canonical classification *interlingua* — at the node/data-model layer, not the wire core.**
   The signed event core stays terminology-agnostic (principle 12, [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md));
   ICD-11 is the canonical pivot **for interop, decision-support, reporting, and the safety projection**
   ([identity §5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)),
   which every node can rely on. It is the FHIR-façade-grade lingua franca ([§3.4](../data-model.md#34-interoperability)),
   one layer below the application, not a constraint baked into the wire.

2. **A coded diagnosis event stores the ICD-11 identifier as the primary structured classification value *and* a
   structured tag referencing a local-terminology concept.** The source is never demoted to bare free-text: the
   tag is a first-class structured reference, so the clinician's own language and the canonical pivot coexist.
   This is the user-chosen *"ICD-11 primary, structured tag"* model, made non-lossy.

3. **The local terminology is an append-only, on-site-curated collection of free-text terms — principle 1 applied
   to vocabulary.** It is emergent and deployment-owned: the *plural edge* (principle 12). The tag a coded event
   carries points into this collection; a local term may itself gather a cluster of captured free-text surface
   forms curated by users. Each local term binds to an ICD-11 entity through an **append-only, overlay-able
   mapping assertion**.

4. **Map-once-remember-forever — *offered*, never forced (the Norway ergonomic + its corollary).** The first use
   of a novel local term *offers* a one-time binding to an ICD-11 entity; once a confident binding exists, every
   subsequent use auto-translates silently. But the clinician may always decline and leave the mapping
   **deliberately open** — a first-class *unsure / pending-professional-coding* state, the honest *not-yet-coded*
   distinct from *unknown* and *refused* ([§3.7](../data-model.md#37-acknowledged-uncertainty-uncertainty-capable-value-types)).
   The binding, whenever made, is a curated, correctable assertion — a wrong or stale map is repaired by a **new
   overlaying mapping assertion** (never erase, always overlay, principle 2), and a projection re-derives the
   corrected ICD-11 for historical events that used that term. This is
   [§3.15](../data-model.md#315-the-active-write-model-thin-encounters-co-produced-legibility-and-the-delete-vs-erase-distinction)
   type-through authoring extended with a vocabulary overlay.

5. **Best-effort overlay, never blocks; the mapping is a separable, separately-authored act (principle 4 +
   paper-parity + compositional authorship).** The clinical write is never gated on coding. A diagnosis may be
   recorded as free-text or an unmapped local term with no ICD-11 yet; the ICD-11 binding is then added —
   **automatically** for a known term, **left open and routed to a coder worklist** when the clinician is unsure
   or mid-emergency, or **human-assisted**. Crucially, the mapping assertion is **authored by whoever makes it**:
   the clinician at point of care, or a **professional clinical coder** later, as a *distinct contributor* under
   the [§3.9](../data-model.md#39-authorship-and-accountability) compositional-authorship model
   ([ADR-0007](0007-authorship-and-accountability.md)) — separating the clinical claim (the clinician's) from the
   coding claim (the coder's) without either overwriting the other. Deferring a guess is therefore not lost work
   but the *correct* division of labour; forcing the clinician to guess would manufacture the precise-untruth
   principle 4 forbids and create downstream coding debt. The whole posture is "surface, don't block," like the
   acknowledgment floor ([ADR-0009](0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md))
   and the advisory duplicate sweep ([ADR-0014](0014-locale-pluggable-matcher-comparators.md)). The open-mapping
   worklist is itself an additive signal ([ADR-0010](0010-additive-vs-suppressing-classification.md)): it only
   raises *"these terms await coding,"* never hides or auto-decides.

6. **Two ICD-11 views, bitemporally ([ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md)).** The
   event carries the **as-asserted** ICD-11 code, version-pinned (which ICD-11 release and which mapping version
   produced it) and immutable; the **current-best** ICD-11 is re-derived through the live mapping when ICD-11
   revises or a binding is corrected. Both coexist; neither erases the other — the same as-recorded-vs-as-it-
   happened duality the bitemporal model already draws.

7. **The legibility twin carries the human label as asserted.** The stable ICD-11 URI is the durable anchor; the
   plaintext [legibility twin](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin)
   preserves the local term and its label *as written at the time*, so the event stays human-readable even as the
   classification and the mappings move beneath it (principle 11).

8. **Licensing posture: verbatim codes in, no bundled crosswalks out.** Cairn ships ICD-11 codes/URIs **verbatim
   with WHO attribution** (the offline container is the distribution vehicle) and **does not bundle a
   Cairn-authored ICD-11↔SNOMED / ICD-10 / external crosswalk** into the AGPL corpus — such maps are node-local,
   version-pinned plug-ins on the distribution plane ([ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)),
   some possibly under separate WHO/source licences. The **local-term→ICD-11 bindings authored on-site are the
   deployment's own data** (original mappings of the node's own vocabulary that merely *reference* WHO codes
   verbatim), free of the ND restriction — which is precisely why retaining the source assertion (decision 2)
   also keeps Cairn licence-clean.

9. **Alternative classifications attach as pluggable translation layers, never on the inter-node path.** Any
   external system — ICPC-3, SNOMED CT, ICD-10-AM, a national scheme — attaches as a node-local layer that
   *produces ICD-11* (and may populate local-term bindings in bulk). The **UI presents the terminology of
   choice**; what is canonical and what crosses between nodes is the mapped ICD-11 plus the structured source
   tag. This is the [ADR-0014](0014-locale-pluggable-matcher-comparators.md) locale-pluggable-comparator posture
   applied to classification: pluggable edges, a uniform pivot, code on the distribution plane and never the
   clinical mesh.

## Consequences

- **Easier:** worldwide interop, decision-support, and the safety projection all key on one stable, public-good
  pivot; clinician acceptance rides a real-world-validated ergonomic (map-once, then silent); the drug INN anchor
  and the disease ICD-11 anchor give one coherent *"stable identifier underneath, plural terminology on top"*
  story; and the design is licence-clean by construction (verbatim codes, offline container, zero bundled
  derivative crosswalks).
- **Harder / trusted surface:** the map-once binding gate, the never-block-the-write floor, and the
  as-asserted-vs-current-best projection are safety-critical (in-database / Rust, [§9](../language-substrate.md));
  the silent auto-translate of known terms and the external translation plug-ins are fit-for-purpose/advisory (a
  defect mis-codes but, being additive and human-correctable, never hides — caught because the source term and
  label still show, [ADR-0010](0010-additive-vs-suppressing-classification.md)). The local mapping collection is
  node-local data the deployment must steward — a curation burden, but one paid once per novel term.
- **The bet:** that ICD-11's identifier stability plus the offline container hold up as a worldwide anchor; that
  map-once-remember-forever drives the curation cost low enough to be welcome rather than resented; and that the
  lossy ICD-11 *projection* is acceptable **because** the structured source term is always retained and the
  mapping is always recomputable. We would know the bet is wrong if ICD-11 churn breaks historical mappings
  faster than re-derivation repairs them, if local-term collections fragment unmanageably across a deployment, or
  if clinicians reject even a once-per-term prompt.
- **Mission / anti-capture:** the canonical pivot is a WHO standard under a redistributable (if no-derivatives)
  licence, not a vendor product; SNOMED CT and ICD-10-AM are excluded from the bundle and available only as
  node-local, separately-licensed plug-ins; no proprietary classification sits on the inter-node path. The
  remaining live question is which pluggable primary-care layer leads — gated on the ICPC-3 licence variant
  flagged in [eval 0003 §8.4](../../ecosystem/0003-reference-data-sourcing-medicines-and-terminologies.md).
- **Policy-neutral (principle 9):** Cairn ships the interlingua, the local-terminology overlay, the map-once
  mechanism, the never-block floor, and the bitemporal projection; it takes no side on which external
  classifications a deployment plugs in, on the local vocabulary's content, or on who may author mappings.
