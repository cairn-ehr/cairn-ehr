# ADR-0020 — The active-write model: thin encounters, type-through authoring, and the delete-vs-erase distinction

- **Status:** Accepted
- **Date:** 2026-06-17

## Context

A UI-requirements pass over the clinical write surface (the progress note, the results/reports
inbox, point-of-care ordering and prescribing) was mined against the user's sixteen years of
production experience with easyGP, an earlier FOSS Postgres EHR whose write model is the product of
that long a campaign of keystroke elimination. The exercise was clinical case-mining aimed at the
*how-you-write* surface rather than at a new architectural question, and — as with several earlier
sessions — what it surfaced was not new architecture but several familiar **one-word-hides-many-dials**
conflations, the same motif ADR-0006 found in "scope", ADR-0007 in "signature", ADR-0008 in
"authentication", and ADR-0009 in "priority":

- **"encounter"** silently imports FHIR-`Encounter` / billing *formality*. The killer case: reviewing
  results, a clinician orders a test *with a comment* — no consultation has occurred, yet those events
  belong together. The real atom is a **thin grouping context** that may be a five-second
  results-review, not a visit.
- **"the order's ordering consult"** looks like it needs a bespoke `order.consult_id` feature. It does
  not: it is just the ambient `encounter` scope key an order inherits *by being authored in context*.
- **"the readable note line vs. the structured event"** looks like two artifacts that could drift apart
  (the order says one drug, the prose says another). They cannot, once the line is understood as a
  *rendering of* the event.
- **"delete"** fuses two operations conventional EHRs never separate: suppressing a *rendering* (routine,
  reversible) and erasing *data* (rare, irreversible). The conflation is the source of a whole class of
  silent-data-loss footguns.
- the **confirmation-dialog ban** (principle 3) looked absolute, but the genuinely *irreversible* acts
  (erasure, repudiation) plainly need *some* friction — which forced a clean distinction between the
  banned click-through confirm and an allowed, substantive gate.

The forcing function throughout is **paper-parity at GP/ED pace**: easyGP's `rx!`/`tx!` "type-through"
(fingers never leave the home row; the order is woven into note authoring) is a concrete, battle-tested
paper-parity benchmark the write surface must clear — writing "FBC" in the note *is* placing the order.

## Decision

None of this needs new architecture. It dissolves into existing primitives — the `encounter` scope key
([data-model §3.5](../data-model.md#35-event-storage-model-hybrid-envelope)), the armed write-context
([ADR-0008](0008-point-of-care-identity-possession-and-salvage.md),
[data-model §3.10](../data-model.md#310-session-identity-event-authorship-and-draft-durability)), the
legibility twin ([ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md),
[data-model §3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin)),
visibility-as-overlay ([ADR-0006](0006-visibility-scope-replication-and-the-safety-projection.md)), and
crypto-shred erasure ([ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md)) — plus one
principle-3 reconciliation. **No new envelope field, no new event stream, no new founding principle.**
Canonical home: [data-model §3.15](../data-model.md#315-the-active-write-model-thin-encounters-co-produced-legibility-and-the-delete-vs-erase-distinction);
the forced-rationale gate is [vision §1.2](../vision.md#12-the-paper-parity-test-normative).

1. **The `encounter` is a *thin* context, not a formal visit.** It is an opaque **grouping id that
   asserts nothing about formality** — a small first-class header of the same shape as the event
   envelope (`{ HLC time, place/scope keys, contributor set, ≥1 linked events }`), a lightweight thing
   events point at. Whether the context was a formal consultation, a phone call, or a five-second
   results-review is a **separate, possibly-absent descriptor**, never forced: founding principle 4
   forbids manufacturing a consultation that did not happen. A "virtual encounter" for one annotated
   order is zero-ceremony and first-class. The prose that introduces it must **guard against importing
   FHIR-`Encounter` / billing semantics** — it is a grouping id, full stop. Its author **may be
   non-human** (an automatic recall system spawns a context; the generated orders and letters hang off
   it): authorship is compositional ([ADR-0007](0007-authorship-and-accountability.md)) applied to
   context creation — a machine is a legitimate contributor, signature proving origin, attestation
   absent or proxied to the recall-policy owner.

2. **The encounter rides on the armed write-context; events inherit it ambiently.** It is the grouping
   the events born in an [ADR-0008](0008-point-of-care-identity-possession-and-salvage.md) possession
   context share. `encounter` is *already* a typed envelope scope key, so an event authored in the
   active context inherits it the same ambient way it inherits facility/department — no new field, no
   bespoke wiring.

3. **Order provenance falls out of the encounter key — it is not a feature.** "Reproduce the ordering
   consult" = fold all events sharing that encounter key into the progress-note view; no reconstruction,
   no guessing. The result-returns-later chain is a direct two-hop pointer fold:
   `result → references order → order.encounter → fold that encounter`. The order is the pivot and
   carries the key *for free* because it was authored in context. This also **structurally explains the
   external-results gap**: a referral-in or post-hospital result authored under a foreign node's context
   carries someone else's encounter key or none, so it degrades **honestly** (principle 4) to a
   *labelled* fallback ("most recent · ordering context unknown"), **never silently presented as the
   ordering consult**. Cairn-to-Cairn federation can preserve the link; a foreign system cannot. A later
   AI cross-reference only ever *proposes* a link as a new event, never asserts one (overlay discipline,
   [ADR-0010](0010-additive-vs-suppressing-classification.md)).

4. **Structured event and human-readable note line are co-produced in one keystroke flow — the
   type-through write model.** This is a **UX invariant, not a schema addition**: orders, prescriptions,
   and referrals are authored *inside* an armed encounter, never as free-floating actions. The
   `rx!`/`tx!`+tab trigger opens an entry surface **beside** the note, never over it — *"never modal"
   extends from reading to writing* — and the clinician keeps typing: drug → formulation dropdown → ⇥ →
   dosing → ⇥ quantity → ⏎, back in the note with the action captured as a readable line. Dosing is a
   **smart default (⏎) except where a default could harm**, in which case the manual entry is **forced**
   (paediatric, pregnant, breastfeeding, renal or hepatic impairment): strip keystrokes where safe,
   force attention where it counts — principle 4 and paper-parity together.

5. **The readable note line is a *derived projection* of the structured event — the legibility twin
   ([§3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin)) rendered
   inline, born at authoring time.** There is exactly **one** event; the prose is a rendering *of* it
   and cannot say something different, so the "two artifacts could diverge" worry dissolves at the root.
   This is founding principle 11 made concrete at the point of authoring — the twin is co-produced *in*
   the write flow, not bolted on afterward, and the write surface is therefore also where the twin's
   fidelity is cheapest to guarantee. The clinician's only freedom over the line is its **visibility**
   (ruling 6).

6. **Two distinct verbs the conventional EHR conflates: `delete` (a rendering) vs. `erase` (the data).**
   The governing line: *"delete only ever removes one UI aspect of the data representation, never the
   original data."* That is **never-erase-always-overlay (principle 2) applied to the display layer.**

   | | `delete` | `erase` |
   |---|---|---|
   | acts on | a rendering (visibility overlay) | the data (crypto-shred, [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md)) |
   | reversible | yes — the data is intact | no — keys destroyed |
   | friction | none | the rare forced-rationale gate (ruling 7) |
   | frequency | routine | ≈ never |

   Deleting a note line suppresses a *rendering*; the event's time, author, context and downstream
   processing are untouched (the test is still ordered, resulted, interaction-checked), and the data
   resurfaces because it never left. **The suppression is recorded as an explicit visibility-overlay
   event (who/when)** — turning "detectable by reconciliation" into "directly auditable" at no cost; the
   *why* may stay unstated (often patient confidentiality), but the *that* is always a recorded event.
   The event's own mandatory legibility twin
   ([§3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin)) is **untouched**
   by line-suppression — it remains the signed audit/RAG substrate, just not *rendered* in that view.
   This slots cleanly under [ADR-0006](0006-visibility-scope-replication-and-the-safety-projection.md):
   confidentiality lives in visibility/presentation, never in existence/replication (the STI-screen
   case — the structured event persists, so the safety projection and interaction-checking still protect
   the patient; only the prose narration is withheld). A corollary worth stating: ordinary `delete`
   needs **zero** friction *because* it destroys nothing, which is what makes ruling 7's gate so rare.

7. **The forced-rationale gate is distinct from the banned confirmation dialog — a reconciliation of
   principle 3, not a breach of it.** Banned (principle 3): the *confirmation dialog* ("Are you sure?
   OK/Cancel") as a safety mechanism — it habituates to click-through and fails paper-parity. Allowed
   (≈ once or twice a year): a **forced-rationale gate** on genuinely *irreversible* harm — a different
   mechanism that (a) demands a **substantive, recorded rationale** and therefore *cannot* be
   click-throughed, and (b) is reserved for the irreducible core of irreversible acts. Because
   append-only + overlay make almost everything reversible (ruling 6), the modal-worthy set **collapses**
   to a tiny handful (crypto-shred/erasure [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md),
   repudiation, …), and that collapse is *why* it is a once-or-twice-a-year event — rarity is what
   preserves its signal. The captured rationale is just an accountability event, the same pattern as
   audited break-glass key-*use*. Rule of thumb: **never block the reversible (overlay handles it); for
   the irreversible few, don't confirm — demand a reason and record it.** Canonical home:
   [vision §1.2](../vision.md#12-the-paper-parity-test-normative).

## Consequences

- **Easier.** Order provenance, "the ordering consult", and the legibility twin all fall out of
  primitives already in the spec — the `encounter` scope key, the armed write-context, the legibility
  twin, the visibility overlay — with no new envelope field, no new event stream, and no new founding
  principle. The `delete`/`erase` split removes a whole class of conventional-EHR silent-data-loss
  footguns, and it *de-risks* erasure: routine deletion can be frictionless precisely because it
  destroys nothing, which is what lets the rare erasure earn the one gate Cairn permits. The
  forced-rationale gate resolves an apparent tension in principle 3 and **shrinks** the friction surface
  rather than widening it.
- **Harder / new trusted surface.** The thin-encounter grouping itself is mostly **fit-for-purpose** — a
  mis-grouping is visible and repairable by overlay. But two seams are safety/privacy-relevant and belong
  in the trusted apply surface ([§9 blast-radius](../language-substrate.md)): the **`delete`-is-never-`erase`
  enforcement** (a deletion that silently became a crypto-shred would be irreversible data loss) and the
  **suppression-is-always-a-recorded-overlay-event** invariant (a rendering-suppression that silently
  leaked, or that was never recorded, would be a confidentiality/audit breach). This is the recurring
  seam motif — the one safety-critical path through an otherwise fit-for-purpose surface, structurally
  like the [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
  seal-time projection seam and the [ADR-0008](0008-point-of-care-identity-possession-and-salvage.md)
  proximity → authorship stamp seam. The `rx!`/`tx!` type-through state machine and the forced-manual
  dosing rule table are fit-for-purpose (UI/advisory), to be ported faithfully from easyGP.
- **The bet.** That the thin-encounter grouping + co-produced legibility + the `delete`/`erase` split are
  enough to clear the easyGP keystroke-economy benchmark while keeping the data model honest. We would
  know the bet is wrong if real use frequently needs **sub-note (span-level) provenance** (which would
  strain the [ADR-0008](0008-point-of-care-identity-possession-and-salvage.md) note-level ruling), if the
  thin encounter proves **too thin** (clinicians want formal-visit structure a bare grouping id cannot
  carry), or if the forced-manual dosing rules cannot be expressed as **policy** without hard-coding
  clinical judgment into the trusted layer.
- **Deferred — gated on next-week easyGP schema access (build-prep, not architecture).** The exact
  `rx!`/`tx!` parser and type-through state machine; the formulation/drug data source and the
  renal/hepatic/pregnancy forced-manual rule table; and the **prefetch/materialization warming daemon**
  (continuous background filling of maintained projections from predicted need). The last **validates
  [ADR-0001](0001-fat-postgres-thin-daemon.md)** — fat Postgres + a thin daemon filling maintained
  projections, arrived at independently in production — and splits cleanly into *scavengeable mechanism*
  (the maintained projections + warming daemon, safety-neutral, scales under fractal topology because a
  workstation node is one clinician = a small predictable working set) vs. *swappable prediction policy*
  (easyGP's top-N heuristic, advisory/fit-for-purpose, instrument hit/miss from day one). Details pending.
