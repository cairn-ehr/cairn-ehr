# ADR-0059 — Medication drug-identity coding: the drugref immortal-moiety anchor and the local-term overlay

- **Status:** Accepted
- **Date:** 2026-07-25
- **Refines:** [ADR-0025](0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md) (the drug-axis
  companion), [ADR-0047](0047-medication-reconciliation-resolution.md) (sharpens the dup-key + group
  display); applies [ADR-0007](0007-authorship-and-accountability.md), [ADR-0014](0014-locale-pluggable-matcher-comparators.md),
  [ADR-0052](0052-born-sealed-clinical-bodies.md).

## Context

[ADR-0025](0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md) settled the **disease**
axis — code on a stable identifier, never a free-text name, with the clinician's own term retained and a
map-once-remember-forever ergonomic — and noted in passing that *"drug substance identity anchors on the
WHO INN by the same discipline."* This ADR settles that **drug** axis concretely, and it is a **design-only
decision**: it fixes the wire-content **shape** of a medication's drug coding before any code carries it,
because that shape is expensive to retrofit on an append-only clinical stream (the whole reason it is done
first).

Three things have changed since ADR-0025 that make this a real decision now, not a restatement:

- **The reserved slot exists but is under-specified.** The [medication-recording design](../../superpowers/specs/2026-07-11-medication-recording-design.md)
  deliberately shipped *without* a Tier-A drug dictionary and left `substance.inn_code` as *"the stable
  anchor slot — NULLABLE = not-yet-coded,"* with Tier-A named as future **overlay enrichment (coding a
  previously-uncoded substance), never a wire change.** But it typed that slot as a bare *"INN id"* string.
  **Keying on a name — even an INN — repeats the founding wound** (principle 2): the INN has national
  divergence (paracetamol/acetaminophen), salt-granularity ambiguity, and pre-/never-INN gaps. The anchor
  must be an **immortal identifier**, not a label.

- **`drugref` now provides exactly that anchor.** The sister service
  [`cairn-ehr/drugref`](https://github.com/cairn-ehr/drugref) (an AGPL-3.0, co-equal public-good
  drug-information service) has shipped its identity spine: every active drug moiety carries an **immortal
  `moiety_uuid`** — `UUIDv5` derived deterministically from the substance's UNII and **pinned forever**
  (upstream churn attaches new claims, never re-keys) — with **INN as the preferred display label, a claim
  and never the key.** So two nodes coding the same substance against their own drugref copies derive the
  **same** anchor with zero coordination — the [ADR-0014](0014-locale-pluggable-matcher-comparators.md)
  content-addressed posture, applied to substances.

- **drugref is a *separable service a node may lack* — unlike ICD-11.** ICD-11 is a mandated interlingua
  distributed as a free offline container that **every node can be relied on to have**, so ADR-0025 lets the
  safety projection *depend* on it. drugref carries no such guarantee: it co-resides in a deployment's
  Postgres *or* is simply absent, and it must **never sit on Cairn's signed inter-node wire core** (drugref's
  own first invariant). This forces a posture ICD-11 did not need — **honest degradation** — and it is the
  crux of this ADR (decision 4).

Two concrete gaps in the built medication surface motivate closing this now rather than later:

- **The reconciliation dup-key blind spot.** `db/031` already records that
  the E1 duplicate-detection key `coalesce(inn_code, normalize(term))` *"prefers `inn_code` when present, so
  the SAME substance asserted once coded and once uncoded lands under two different keys and is NOT
  flagged … Cross-coding-state matching waits on the [dictionary]."* drugref **is** that dictionary.
- **The reconciled-group display is arbitrary.** After a clinician links `Lipitor ↔ atorvastatin`, the
  `medication_group_display` projection (`db/033`) shows
  whichever member's UUID sorts lowest — possibly the brand, possibly *"little white pill."* There is no
  stable notion of the group's canonical drug identity.

The four forces ADR-0025 named (stable-identifier / principle 11, clinician-acceptance / paper-parity,
never-a-blocking-field / principle 4, licence-clean-by-construction) apply unchanged and are not restated.

## Decision

Canonical home: **[data-model §3.16](../data-model.md#316-clinical-concept-coding-the-icd-11-interlingua-and-the-local-terminology-overlay)**
(the drug-axis subsection, beside the ICD-11 disease axis), with a pointer from
[§3.3](../data-model.md#33-mutable-non-demographic-state).

1. **The drug-identity anchor is drugref's immortal `moiety_uuid`, never a free-text name** (principle 2).
   INN is the **display**, never the key. The event carries the clinician's **coding *claim*** (an
   identifier plus the label as resolved at coding time) — it **never embeds drugref's data.** The immortal
   UUID makes this axis *more* stable than the ICD-11 axis: the identifier itself never revises (drugref
   pins on first sight); only labels and derived crosswalks refine.

2. **The coding is a structured `substance.coding` object `{ system, code, display }`, generalizing the
   reserved `inn_code` slot.**
   - `system` names the drugref composition-tree level: **`drugref-moiety`** is the only value today (the
     only level drugref has built), with `drugref-clinical-drug` / `drugref-product` **reserved** for later
     drugref slices — so strength/form-level coding lands additively without reshaping the slot.
   - `code` is the immortal identifier (the `moiety_uuid`).
   - `display` is the **INN-preferred label captured at coding time** — the honest-degradation label
     (decision 4) and part of the [§3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin)
     legibility twin (principle 11): a node without drugref still shows the preferred name.
   - `substance.term` (the clinician's own words) **stays mandatory** and is the ultimate legibility floor;
     the coding never replaces it. Uncoded — `coding` absent — stays fully valid (principle 4, the *"little
     white pill"* floor). This shapes the reserved slot for the first time; there is no production data to
     migrate ([pre-clinical posture](../../HANDOVER.md)), and the discipline is *additive-only from here*.

3. **Coding is a separable, separately-authored act** ([§3.9](../data-model.md#39-authorship-and-accountability)
   compositional authorship, [ADR-0007](0007-authorship-and-accountability.md)). It may appear **inline on
   the assertion** (auto-filled from a drugref type-ahead at authoring time) **or as a later
   coding-overlay event** — a new event type **`clinical.medication-coding.asserted`** (correctable by
   **`clinical.medication-coding.corrected`**, never erased, always overlaid) — authored by **whoever codes
   it**: the clinician at point of care, or a pharmacist / professional coder later, as a *distinct
   contributor* whose coding claim never overwrites the clinician's clinical claim. **Map-once-remember-
   forever, *offered* never forced** (the ADR-0025 Norway ergonomic): a novel term offers a one-time
   binding, then auto-fills silently; the clinician may always decline and leave the coding **deliberately
   open** — an honest *not-yet-coded* state routed to a coder worklist, never a forced guess that becomes
   coding debt.

4. **The coding is advisory and honest-degrading — the deliberate divergence from ADR-0025.** Because
   drugref is a separable service a node may lack, **drugref-*the-service* is node-local advisory
   enrichment**, even though the `moiety_uuid` *anchor* is a first-class stable value that syncs on the
   event like any coded field:
   - A node **without** drugref still **reads, syncs, lists, and reconciles** a coded medication — via the
     captured `display` and the mandatory `term`. It simply does not get drugref-powered enrichment
     (drug–drug-interaction alerts, brand↔generic resolution). It degrades to *references-only*, exactly
     like the [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md) byte tier and the
     [ADR-0014](0014-locale-pluggable-matcher-comparators.md) missing-comparator case: uncertainty can only
     *withhold* an advisory, never block the record.
   - The baseline **safety projection** ([identity §5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope))
     must remain derivable from the event's **own** coded fields; drugref *may enrich* it locally where
     present, but the floor never *depends* on drugref being installed. This keeps drug decision-support in
     the [§9](../language-substrate.md) advisory tier (a defect mis-advises and is caught by the clinician;
     it cannot corrupt or wedge the signed record).

5. **The anchor sharpens reconciliation — advisorily.** The E1 dup-key becomes
   `coalesce(moiety_uuid, inn_code, normalize(term))`, so the *"same substance coded once and uncoded once"*
   cross-state miss `db/031` records is now catchable — as an **advisory
   flag**, never an auto-merge (reconciliation stays a human link, [ADR-0047](0047-medication-reconciliation-resolution.md)).
   The reconciled-group display **prefers a coded member** and shows its INN `display`, with a deterministic
   tiebreak (`moiety_uuid` then `medication_id`, `COLLATE "C"`, [ADR-0045](0045-collation-independent-projection-tiebreaks.md)).
   **Two *different* `moiety_uuid`s inside one reconciled group is an advisory *possible-mis-reconciliation*
   signal — surfaced, never silently resolved.** Full fuzzy brand↔generic/typo/salt matching still waits on
   the drug-matcher (a later advisory slice); this ADR fixes only the exact-anchor path.

6. **Bitemporal coding views** ([§3.6](../data-model.md#36-bitemporal-event-time-recording-time-vs-effective-time),
   [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md)), mirroring ADR-0025 decision 6 but
   simpler because the identifier is immortal: the event pins the **as-asserted** coding (`moiety_uuid` +
   `display` + the drugref release that produced it) immutably; a **current-best** display / crosswalk is
   re-derived through the *live* drugref where present. Since `moiety_uuid` never re-keys, only labels and
   derived data can refine — the anchor never moves beneath a historical event.

7. **Wire and licence posture** (principle 12; ADR-0025 decision 8; drugref's own invariant): drugref data
   **never rides Cairn's inter-node wire** — only the clinician's coding claim (`{system, code, display}`)
   does, the same *verbatim-codes-in, no-bundled-data-out* posture as ICD-11. **Policy-neutral**
   (principle 9): Cairn ships the coding mechanism, the honest-degradation floor, and the bitemporal
   projection, and mandates none of it. drugref is the **reference** drug-identity authority, **not a
   mandated dependency** — a deployment may plug a different authority so long as it presents stable
   identifiers with captured display labels; nothing on the wire assumes drugref specifically.

## Paper-parity benchmark (§1.2)

*Not a runnable surface — this ADR fixes an event shape only.* The falsifiable time/step benchmark is
**owed by the future code slice** that first exposes a coding UI, and this ADR pins its governing
constraint so that slice cannot regress it:

- **Paper counterpart:** writing a drug name on a paper medication list. The clinician writes *"atorvastatin"*
  (or *"Lipitor"*, or *"little white pill"*) — **one** human act; nothing on paper forces a code.
- **Architecture-forced steps `M`:** the coding adds **zero** forced human acts (`M = N = 1`). Coding is
  **optional auto-fill** from type-ahead — the moiety anchor + display attach invisibly when the clinician
  picks a suggestion, and an uncoded free-text term remains a first-class recordable value (principle 4).
  Any design that makes coding a *required* field to save a medication is an **architecture defect** to file
  (house rule 5), because it would satisfy a field only by fabrication where the clinician cannot vouch for
  the substance identity.

## Consequences

- **Easier:** one coherent *"stable identifier underneath, plural naming on top"* story across the disease
  (ICD-11) and drug (drugref-moiety) axes; reconciled-group display gains a sensible canonical drug identity;
  the coded-vs-uncoded duplicate blind spot becomes an advisory catch; and independent nodes agree on the
  anchor with no coordination (deterministic `UUIDv5`).
- **Harder / trusted surface:** the **honest-degradation floor** is the safety-critical part and must be
  built as such ([§9](../language-substrate.md)) — a coded medication must read, sync, list, and reconcile
  on a node with **no** drugref, and the baseline safety projection must fire without it. The advisory
  enrichment (DDI, fuzzy matching) is fit-for-purpose. The coding-overlay event and its compositional
  authorship join the trusted write path (a new closed-enum event type + its authorship binding at both
  doors) — which is exactly why the shape is fixed here, before code.
- **The bet:** that drugref's `moiety_uuid` stability and separable-service availability degrade gracefully
  in the field (a node offline from drugref loses alerts, never legibility); that coding-as-optional-auto-fill
  stays *welcome* rather than resented (the validated ADR-0025 ergonomic); and that anchoring on the moiety
  level is the right granularity until drugref builds the clinical-drug/product levels the reserved `system`
  values await. **We would know the bet is wrong** if drugref-less nodes proved unable to render or reconcile
  coded meds acceptably, if clinicians rejected even the once-per-term offer, or if moiety-level coding
  proved too coarse to be clinically useful before the finer drugref levels arrive.
- **Mission / anti-capture:** the drug-identity authority is a **co-equal public good** consumed on the same
  public footing as any third party, never a bundled vendor drug database and never on the inter-node path
  (principle 12); a deployment may substitute another stable-ID authority. No proprietary drug dictionary can
  become load-bearing for interoperability.

## Follow-on (the code slice this ADR unblocks — not built here)

A future `clinical.medication` code slice, taken brainstorm→plan→TDD, will: add the `substance.coding`
`{system, code, display}` shape to the `cairn-event` builder + twin + the db/031 floor and projection; add
the `clinical.medication-coding.asserted` / `.corrected` event types (with twin-registry registration in
**both** places and their authorship binding at both doors); widen the E1 dup-key and the
`medication_group_display` winner to prefer the coded anchor; and carry the runnable **§1.2 paper-parity
benchmark** measurement owed above. Cross-node convergence and honest-degradation (drugref-absent) are
first-class test obligations.
