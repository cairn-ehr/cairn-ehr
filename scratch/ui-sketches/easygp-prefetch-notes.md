# easyGP → Cairn: prefetch & materialization scavenge notes

> Scratch / conversation aid — NOT canonical. Pre-read for the next-week session
> when HH has access to the full easyGP Postgres schema + PL/pgSQL + PL/Python.

## Context
UI requirements discussion (clinical dashboard + results/reports inbox) surfaced
the performance machinery easyGP used. Goal: scavenge what scales to Cairn's
larger/federated/offline-first deployments; discard what only worked at
single-practice scale.

## What easyGP did (from HH, production experience)
- **Daemon continuously filled materialized tables from algorithmically predicted needs.**
- Inbox/cockpit context: **background prefetch of the first N items, top-to-bottom**
  (assumes clinicians work down the urgency-sorted list). Pragmatic compromise — no
  crystal ball, can't prefetch too many.
- **Order provenance:** tests ordered *within* easyGP automatically captured a direct
  reference to the ordering consult → vast majority of results had it. Results from
  *outside* (referrals in, post-hospital) lacked it → fell back to **most recent consult**,
  no smart guessing.
- Hit/miss of prediction was **never measured** — "worked as intended" at a handful of
  clinicians, small practice scale.

## Architectural reads (Cairn)
1. **Validates [ADR-0001](../../docs/spec/decisions/0001-fat-postgres-thin-daemon.md)** —
   fat Postgres + thin daemon filling materialized tables, arrived at independently in
   production.
2. **Scale worry likely inverts under fractal topology.** Small-predictable-working-set
   is a property of *node role*, not of *being small*: a workstation node = one clinician =
   the easyGP regime, guaranteed at every tier. Cairn never asks one node to predict for
   everyone (the non-scaling case).
3. **Mechanism/policy split** = the de-risking lever:
   - *Mechanism* (maintained projections + warming daemon) → safety-neutral, scales, ADR-0001.
     **This is what we scavenge from easyGP.**
   - *Prediction policy* (what to warm; easyGP's top-N heuristic) → advisory/fit-for-purpose,
     swappable with zero blast radius. Start with the exact easyGP heuristic.
4. **Instrument hit/miss from day one** (prefetch-used vs. evicted-unused). Nearly free;
   essential at scale; doubles as drift/tuning signal — same pattern as the matcher in
   [ADR-0014](../../docs/spec/decisions/0014-locale-pluggable-matcher-comparators.md).
5. **Principle-4 on the missing-provenance fallback:** label the fallback note
   ("most recent · ordering consult unknown"), don't silently present it as the ordering
   consult. Federation narrows the gap (Cairn-to-Cairn results may carry their own
   order→encounter link). Later AI cross-referencing only *proposes* a link as a new event,
   never asserts one (overlay discipline).

## Order provenance — RESOLVED: it's already in the envelope, not a new field
HH described the easyGP order-capture path: context = current consultation; order triggered by
button **or** by typing `tx!`+tab inline in the progress note (keyboard-only, fingers never
leave home row); the order form is prefilled from the current consultation.

Mapping to Cairn — this is **not** a bespoke "order.consult_id" feature:
- **`encounter` is already a typed envelope scope key** ([data-model §3.1](../../docs/spec/data-model.md),
  alongside facility/department). An order authored in the active consult inherits that key
  ambiently, like every other event born in that context.
- **The "current consultation" = the armed write-context** ([ADR-0008](../../docs/spec/decisions/0008-point-of-care-identity-possession-and-salvage.md),
  [data-model §3.10](../../docs/spec/data-model.md#310-session-identity-event-authorship-and-draft-durability)):
  possession binds `(clinician, patient)`; the draft/context store is keyed `(author, patient)`,
  durable across re-auth. The active encounter rides on top.
- **Reproducing "the ordering consult"** = fold all events sharing that encounter key into the
  progress-note view. No reconstruction, no guessing.
- **Result-returns-later chain (two hops):** `result → references order → order.encounter →
  fold that encounter`. The order is the pivot; it carries the key for free because authored in-context.

**Therefore the day-one requirement is a UX invariant, NOT a schema addition:**
> Orders are always authored *inside* an armed encounter context, never as a free-floating
> action. The `tx!`+tab inline trigger is that invariant made ergonomic — the order is woven
> into note authoring (structured order event + note prose = two events, one encounter scope,
> one possession). Paper-parity: writing "FBC" in the note IS placing the order.

**External-results gap is now structurally explained:** referral-in / post-hospital results were
authored under a foreign node's encounter context (or none) → the `encounter` key is someone
else's or null. Cairn-to-Cairn it can survive federation; from a foreign system it can't.
Honest degradation (principle 4), labeled fallback to most-recent note.

### Answered (HH), with Cairn implications

**1. Granularity — the context id is a "progress note" item, NOT a formal consultation.**
Killer case: reviewing results, you order a test *with a comment*, no consultation occurred — yet
those events share a context. easyGP captured that as a progress-note item. → **Cairn's `encounter`
envelope key must be semantically THIN: an opaque context-grouping id that asserts nothing about
formality.** Whether the context was a formal consult / phone / 5-sec results-review is a *separate,
possibly-absent descriptor*, never forced (principle 4 — don't manufacture a consultation that
didn't happen). A "virtual encounter" for one annotated order is zero-ceremony and first-class.
Risk to avoid: the name "encounter" imports FHIR-Encounter / billing formality — guard against that
in the data-model prose when written (it's a grouping id, full stop).

**2. The order stored a direct FK to the context** (not inferred). → In Cairn the order event carries
the context id directly in its envelope, so `result → order → context` is a direct pointer hop, fast
by construction.

**3. Type-through write model (16 yrs of keystroke elimination) — see sketch
`active-write-typethrough.svg`.** `rx!` opens a prescribe tab BESIDE the note (non-modal, both
visible); keep typing the drug → dropdown of formulations → ⇥ select → dosing (invariant default ⏎,
or FORCED manual for paediatric/pregnant/breastfeeding/renal/hepatic) → ⇥ qty → ⏎ → back in note,
Rx captured as a readable line, keep typing. Same pattern for `tx!`, referrals, most orders.
Cairn principle alignment:
- "Never modal" extends from *reading* to *writing* — the entry tab is a side panel, never an overlay.
- Structured event + human-readable note line are **co-produced in one flow** = the **legibility twin**
  ([principle 11](../../docs/spec/index.md)) born at authoring time, not bolted on.
- Smart defaults / forced-manual = principle-4 + paper-parity: strip keystrokes where safe, force
  attention where a default could harm.

### Further refinements (HH)

**1a. The thin context id is a small first-class entity: `{time, place, author, ≥1 linked events}`.**
- Same shape as the event envelope (HLC time, scope/place keys, contributor set) → it's a lightweight
  *header* that events point at via the `encounter` key.
- **Author can be non-human** (e.g. an automatic recall system spawns the context; generated
  orders/letters hang off it). Lands on [ADR-0007](../../docs/spec/decisions/0007-authorship-and-accountability.md):
  authorship is compositional, a machine is a legitimate contributor; signature = origin (the algorithm),
  attestation absent or proxied to the recall-policy owner. Contexts are not always human-initiated.

**3a. The narrow modal exception is a RECONCILIATION, not a breach of principle 3.**
- Banned (CLAUDE.md principle 3): *confirmation dialogs* ("are you sure? OK/Cancel") as a safety
  mechanism — click-through fatigue, fails paper-parity.
- Allowed (HH, ~1–2×/year): a **forced-rationale gate on irreversible harm**. Distinct mechanism:
  - demands a *rationale* (substantive, recorded) — cannot be click-throughed;
  - reserved for the genuinely *irreversible*. Append-only + overlay make almost everything reversible,
    so the modal-worthy set collapses to the irreducible core (crypto-shred/erasure
    [ADR-0005](../../docs/spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md), repudiation, a
    tiny handful). That collapse is *why* it's a once-or-twice-a-year event; rarity preserves its signal.
  - the captured rationale is just an accountability event (same pattern as break-glass key-use).
- Rule of thumb: **never block the reversible (overlay handles it); for the irreversible few, don't
  confirm — demand a reason and record it.** Candidate phrasing for spec/ADR; distinguish
  "confirmation dialog (banned)" from "forced-rationale gate (rare, allowed)".

### Still to verify / pull next week against easyGP
- [ ] The `rx!`/`tx!`+tab parser & the type-through state machine (port faithfully — battle-tested).
- [ ] How the progress-note item (context id) was spawned — cost, lifecycle, when a new one starts.
- [ ] Formulation/drug dropdown data source + the renal/hepatic/pregnancy forced-manual rule table.
- [ ] Confirm the order FK pointed at the progress-note item (= the thin context), not a formal visit row.

## To look at next week (in the easyGP code)
- [ ] Materialized table structures behind inbox + cockpit (schema, indexes).
- [ ] The daemon: what it predicts, eviction policy, how N was chosen.
- [ ] PL/Python functions doing the prediction / context assembly.
- [ ] Order→consult capture path (how the reference was written at order time).
- [ ] Which optimizations are scale-sensitive (assumed single-node, low concurrency, no partition).
- [ ] Trajectory/series queries (Cairn needs these as folds over append-only, latest-truth-per-timepoint).
