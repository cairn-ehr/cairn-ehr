# ADR-0059 — Medication drug-identity coding: the drugref immortal-moiety anchor and the local-term overlay

- **Status:** Accepted
- **Date:** 2026-07-25
- **Refines:** [ADR-0025](0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md) (the drug-axis
  companion), [ADR-0047](0047-medication-reconciliation-resolution.md) (sharpens the dup-key + group
  display); applies [ADR-0007](0007-authorship-and-accountability.md), [ADR-0014](0014-locale-pluggable-matcher-comparators.md),
  [ADR-0052](0052-born-sealed-clinical-bodies.md) (decision 8), [ADR-0057](0057-generic-reprojection-registered-apply-dispatch.md)
  (decision 8); upholds the [identity §5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
  safety floor (decision 4).

## Context

[ADR-0025](0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md) settled the **disease**
axis — code on a stable identifier, never a free-text name, with the clinician's own term retained and a
map-once-remember-forever ergonomic — and recorded in passing that the drug axis had already *"resolved
this by anchoring substance identity on the WHO INN,"* citing
[ecosystem eval 0003](../../ecosystem/0003-reference-data-sourcing-medicines-and-terminologies.md) (the
evaluation that fed ADR-0025 and is where that INN position actually originates). This ADR settles the
**drug** axis concretely and **refines that position**: the anchor becomes an immortal identifier and the
INN demotes to display. It is a **design-only decision** — it fixes the wire-content **shape** of a
medication's drug coding before any code carries it, because that shape is expensive to retrofit on an
append-only clinical stream (the whole reason it is done first).

Three things have changed since ADR-0025 that make this a real decision now, not a restatement:

- **The reserved slot exists but is under-specified.** The medication-recording design note
  (`docs/superpowers/specs/2026-07-11-medication-recording-design.md`, working scaffolding, not published)
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
  flagged … Cross-coding-state matching waits on the [dictionary]."* drugref **is** that dictionary — though
  note carefully what it does and does not close (decision 5): a shared anchor closes *coded↔coded*
  immediately; resolving an **uncoded** member's free text to a moiety needs term→anchor lookup, which is the
  later drug-matcher slice, not the key itself.
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
   INN is the **display**, never the key. The event carries the clinician's **coding *claim*** — an
   identifier plus the small values resolved at coding time — never drugref's **dataset** (see decision 7,
   which draws that line precisely, because a naive *"no drugref data"* reading would forbid the very
   captured fields that make honest degradation work). The immortal
   UUID makes this axis *more* stable than the ICD-11 axis: the identifier itself never revises (drugref
   pins on first sight); only labels and derived crosswalks refine.

2. **The coding is a structured `substance.coding` object `{ system, code, display }`, *replacing* the
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
     white pill"* floor).
   - **`inn_code` is retired, not kept alongside.** `coding` *replaces* the reserved slot rather than joining
     it: the payload builder stops emitting `substance.inn_code`, and nothing — dup-key, projection winner,
     display — reads it again. This is safe precisely because the slot was reserved and never populated in
     anger: the project is **pre-clinical** (no production events exist to migrate, see `docs/HANDOVER.md`),
     so this shapes the slot for the first time rather than changing a shape in flight. The *projection*
     column `patient_medication.inn_code` is nevertheless **deprecated in place and never dropped** — the
     `sync_state.hlc_wall` treatment in `db/036` — because a DROP is the non-additive move ADR-0012 /
     principle 11 forbid. From here the discipline is **additive-only**; this is the last non-additive
     moment, and it is available only because nothing has been written yet.

3. **Coding is a separable, separately-authored act** ([§3.9](../data-model.md#39-authorship-and-accountability)
   compositional authorship, [ADR-0007](0007-authorship-and-accountability.md)). It may appear **inline on
   the assertion** (auto-filled from a drugref type-ahead at authoring time) **or as a later
   coding-overlay event** — a new event type **`clinical.medication-coding.asserted`**, corrected by a
   **`clinical.medication-coding-correction.asserted`** overlay (never erased, always overlaid) — authored by
   **whoever codes it**: the clinician at point of care, or a pharmacist / professional coder later, as a *distinct
   contributor* whose coding claim never overwrites the clinician's clinical claim. **Map-once-remember-
   forever, *offered* never forced** (the ADR-0025 Norway ergonomic): a novel term offers a one-time
   binding, then auto-fills silently; the clinician may always decline and leave the coding **deliberately
   open** — an honest *not-yet-coded* state routed to a coder worklist, never a forced guess that becomes
   coding debt.
   Both names follow the corpus's existing event-type grammar rather than coining a new one: a correction is
   **its own noun stream ending in `.asserted`** — exactly as `clinical.medication-dose-correction.asserted`
   already is (ADR-0050) — never a `.corrected` verb. Every registered clinical type in `db/` ends in
   `.asserted`; the only non-`.asserted` verb anywhere in the corpus is `identity.dispute.resolved`. Both
   names also sit under the `clinical.%` prefix the `db/005` submit gate keys on.

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
   - **The [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
     safety projection is derived at *coding* time on the *coding* node, and travels — it is never
     re-derived by the reader.** §5.9 specifies that projection's content as coarse safety **classes**
     (interaction/allergy class, contraindication flags) and states the mechanism plainly: *"a coded drug's
     interaction class is a property of the code."* A property *of the code* is a drug-knowledge lookup — so
     "derivable from the event's own fields" cannot mean the **reader** re-derives it, which would make the
     §5.9 floor depend on drugref after all. The resolution is the same move decision 2 already makes for
     `display`, and it works because **a node that could produce a coding necessarily had a coding authority
     at that moment**: the class is computed *pre-seal on the coding node* (§5.9 already calls this "a normal
     pre-seal projection step"), captured, and carried on the projection, which §5.9 replicates **in the
     clear**. A drugref-less node therefore honours the §5.9 floor for a **coded** medication without ever
     holding drugref.
   - **What the floor does and does not promise.** For an **uncoded** medication there is no class on *any*
     node, drugref present or not — that is the principle-4 *"little white pill"* floor being honest, not a
     degradation drugref's absence caused. Where a deployment's coding authority (decision 7 permits others)
     supplies identifiers but **no** class, the signal **coarsens down the §5.9 disclosure ladder** — it
     never disappears: §5.9's safety-floor invariant is that grade and gaps control the projection's
     *coarseness, never its existence*. **Carrying the class, rather than expecting the reader to compute
     it, is the load-bearing constraint this ADR fixes; the class field itself belongs to the safety-
     projection shape (a separate de-identified signal beside the sealed body, §5.9) and is owed by the
     safety-projection slice — not to `substance.coding`, which stays the clinician's identity claim.**
   - Everything *beyond* that floor — DDI alerting, brand↔generic resolution, fuzzy matching — stays
     [§9](../language-substrate.md) advisory tier, present-node-only (a defect mis-advises and is caught by
     the clinician; it cannot corrupt or wedge the signed record).

5. **The anchor sharpens reconciliation — advisorily, and only on the coded↔coded path.** The E1 dup-key
   becomes `coalesce((system, code), normalize(term))` — the coding slot **as a pair**, else the normalized
   term — never a bare `code`. Keying on `(system, code)` is what keeps the reserved levels of decision 2
   safe: once `drugref-clinical-drug` exists, the same substance coded at *moiety* level on one node and at
   *clinical-drug* level on another would otherwise split under a bare-`code` key — the same blind spot one
   level up, and a **cross-node** one. Finer levels are compared by **rolling up to their moiety ancestor**
   before keying (a drugref-present enrichment; a node lacking drugref cannot roll up and honestly declines
   to match rather than guessing — [ADR-0014](0014-locale-pluggable-matcher-comparators.md)'s
   *no-data-is-never-disagreement*).
   **Be precise about what this closes.** A `coalesce` picks per row, so a coded row keys on its anchor and
   an uncoded row keys on its term: this closes **coded↔coded** — including the genuinely valuable
   `Lipitor`↔`atorvastatin` case *once both are coded* — and it does **not**, by itself, close the
   *"same substance asserted once coded and once uncoded"* miss `db/031` records. That case closes when the
   uncoded member **gets coded** (a workflow act — *offered, never forced*, decision 3), or later by
   term→anchor resolution in the drug-matcher slice. Claiming the key alone closes it would be the same
   overstatement one level up, so this ADR does not claim it. Full fuzzy brand↔generic/typo/salt matching
   likewise waits on that matcher; this ADR fixes only the exact-anchor path.
   All of it is **advisory** — a flag, never an auto-merge (reconciliation stays a human link,
   [ADR-0047](0047-medication-reconciliation-resolution.md)).
   The reconciled-group display **prefers a coded member** and shows its INN `display`, with a deterministic
   tiebreak (`(system, code)` then `medication_id`, `COLLATE "C"`, [ADR-0045](0045-collation-independent-projection-tiebreaks.md)).
   **Two *different* anchors inside one reconciled group is an advisory *possible-mis-reconciliation*
   signal — surfaced, never silently resolved.**

6. **Bitemporal coding views** ([§3.6](../data-model.md#36-bitemporal-event-time-recording-time-vs-effective-time),
   [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md)), mirroring ADR-0025 decision 6 but
   simpler because the identifier is immortal: the event pins the **as-asserted** coding — exactly the
   `{system, code, display}` of decision 2, nothing more — immutably; a **current-best** display / crosswalk
   is re-derived through the *live* drugref where present. Since `moiety_uuid` never re-keys, only labels and
   derived data can refine — the anchor never moves beneath a historical event.
   **Deliberately *not* in the shape: a stamp of the drugref release that produced the label.** It would be
   useful provenance ("this display came from a release that later corrected it"), but it is not needed by
   any decision here, and — unlike the shape itself — adding it later is a plain **additive** field that
   ADR-0012 / principle 11 permit outright. It is therefore not retrofit-hard and does not have to be paid
   for now. Keep the shape minimal; earn the field when a case demands it.

7. **Wire and licence posture** (principle 12; ADR-0025 decision 8; drugref's own invariant): drugref's
   **dataset** — its tables, crosswalks, interaction matrices — **never rides Cairn's inter-node wire.** Only
   the coding *claim* does, the same *verbatim-codes-in, no-bundled-data-out* posture as ICD-11.
   - **The explicit carve-out: a small *derived value captured at coding time* is part of the claim, not
     bundled data.** The `display` label of decision 2 and the §5.9 safety class of decision 4 are both
     drugref-*derived*, and both must travel — otherwise the reader has to hold drugref, which is the exact
     failure this ADR exists to prevent. Naming this carve-out matters because a naive reading of
     *"no drugref data on the wire"* would forbid the very fields that make honest degradation work.
   - **It is licence-clean for a reason ICD-11's was not.** ADR-0025's tighter *no-bundled-crosswalks* rule
     is forced by ICD-11 being **CC BY-ND 3.0 IGO** — no derivatives, so Cairn may not ship its own
     crosswalks. drugref is **AGPL-3.0** with no ND clause, so a captured derived label or class carries no
     equivalent restriction. The limit here is architectural (do not turn the wire into a replication channel
     for a drug database), not licensing.
   - **Policy-neutral** (principle 9): Cairn ships the coding mechanism, the honest-degradation floor, and
     the bitemporal projection, and mandates none of it. drugref is the **reference** drug-identity
     authority, **not a mandated dependency** — a deployment may plug a different authority so long as it
     presents stable identifiers with captured display labels; nothing on the wire assumes drugref
     specifically.

8. **The coding events are born-sealed clinical bodies** ([ADR-0052](0052-born-sealed-clinical-bodies.md)):
   they are `clinical.%` content and take the same custody treatment as the assertion they code. Two
   consequences shape the code slice enough to state here. **First**, the captured `display` (and the
   decision-4 class) are what a reader still has when it can read a projection but not unseal a body — which
   is *why* they are captured upstream rather than re-derived downstream. **Second**, a node that installs
   drugref *later* backfills nothing from the wire: because the as-asserted coding is immutable (decision 6)
   and no peer re-sends a richer version, later enrichment is re-derived **locally**, by replaying that
   node's own events through `cairn_reproject` ([ADR-0057](0057-generic-reprojection-registered-apply-dispatch.md)).

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
  brand↔generic duplicates become an advisory catch **once both members are coded** (the coded↔uncoded case
  still waits on coding or the matcher, decision 5); and independent nodes agree on the anchor with no
  coordination (deterministic `UUIDv5`).
- **Harder / trusted surface:** the **honest-degradation floor** is the safety-critical part and must be
  built as such ([§9](../language-substrate.md)) — a coded medication must read, sync, list, and reconcile
  on a node with **no** drugref, and the §5.9 safety projection must fire on such a reader *because the class
  was captured upstream at coding time*, never because the reader re-derived it (decision 4). The advisory
  enrichment (DDI, fuzzy matching) is fit-for-purpose. The two coding event types and their compositional
  authorship join the trusted write path (closed-enum types + twin registration in **both** places +
  authorship binding at both doors) — which is exactly why the shape is fixed here, before code.
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

A future `clinical.medication` code slice, taken brainstorm→plan→TDD, will:

- add the `substance.coding` `{system, code, display}` shape to the `cairn-event` builder + twin + the
  `db/031` floor and projection, **retiring `substance.inn_code` from the builder** while leaving the
  projection column deprecated-in-place (decision 2);
- add the `clinical.medication-coding.asserted` and `clinical.medication-coding-correction.asserted` event
  types — twin-registry registration in **both** places (the Rust registry *and* its `db/tests` mirror) and
  authorship binding at both doors;
- widen the E1 dup-key to `coalesce((system, code), normalize(term))` and the `medication_group_display`
  winner to prefer the coded anchor (decision 5) — **without** claiming the coded↔uncoded catch the key does
  not deliver;
- carry the runnable **§1.2 paper-parity benchmark** measurement owed above.

Cross-node convergence and honest degradation are **first-class test obligations**: a drugref-absent node
must read, sync, list and reconcile a coded medication, and — the load-bearing one — the §5.9 safety
projection must fire on that node from the **captured** class, proving the floor never re-derives. The
safety-class capture itself is owed by the safety-projection slice (decision 4), which this ADR constrains
but does not build.
