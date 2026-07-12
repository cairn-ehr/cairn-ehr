# Design — Medication dose overlay (slice 2 of the `clinical.medication` surface)

**Date:** 2026-07-12 · **Branch:** `feat/medication-dose-overlay-slice-2` · **Status:** design, pending
implementation plan.

**Scope of change:** additive Rust (`cairn-event::medication`, `cairn-node::medication` + CLI) + one new
DB migration (`db/032_medication_dose.sql`: floor + projection). Adds two verbs to the medication surface
opened by [slice 1](2026-07-11-medication-recording-design.md). **No new founding principle, no new
envelope field, no ADR, no spec bump** — this graduates the slice-1 §8 deferral ("dose-correction /
change-of-dose overlay") into product code, reusing the settled event-overlay + §3.9 additive-vs-suppressing
primitives.

---

## 1. What this is, and why now

Slice 1 records a medication as a thread (`clinical.medication.asserted`) that a cessation ends. It froze
the *initial* dose on the assert and explicitly deferred **dose change over time** and **dose correction**
(§8). In real practice a dose is the most-mutated field on a medication: it is titrated up and down, and it
is mis-recorded. Slice 2 makes those two acts first-class, with a **queryable dose history** (the titration
trail — "warfarin 5→3 mg last week"), while keeping the slice-1 posture: **safety-critical write path**
(pure Rust builders + an in-DB floor + a projection), advisory reconciliation left where slice 1 put it.

The two acts are **clinically and temporally distinct**, and Cairn already has the vocabulary for the
distinction (§3.9 / [ADR-0010](../../spec/decisions/0010-additive-vs-suppressing-classification.md)):

- **Dose change** (titration) — *"atorvastatin increased 40→80 mg on 12 Jun."* The 40 mg was **true then**;
  the 80 mg is true from an effective date. **Both are history.** On paper: a **new line**. → *additive*.
- **Dose correction** (data-entry fix) — *"I recorded 40 mg but it was 20 mg all along."* The 40 mg was
  **never true**. On paper: a **strike-through + initials**. → *corrective* (but see §4 — still an
  *additive* event in the `event_type_class` sense; it does not foreclose the original).

Modelling them as one "amend dose" would lose the paper-parity distinction and make a future CDS/audit
unable to tell a titration from a mistake. Slice 2 keeps them as two verbs.

**Blast radius → substrate ([§9](../../spec/language-substrate.md)).** The write path can silently corrupt
the clinical record, so it is safety-critical: pure Rust builders + an **in-DB floor** + a projection.
Structure mirrors slice 1 exactly.

## 2. Event vocabulary — two new verbs over the existing thread

Both reference the existing immortal `medication_id` (never mint a new thread — a dose event is *about* an
existing medication), and both preserve slice-1's `.asserted` speech-act suffix with the noun slot carrying
the verb (as `medication` vs `medication-cessation`):

| Event type | schema_version | Body carries |
|---|---|---|
| `clinical.medication-dose-change.asserted` | `clinical.medication-dose-change/1` | the new dose + when it changed |
| `clinical.medication-dose-correction.asserted` | `clinical.medication-dose-correction/1` | the corrected dose + which dose event it fixes |

- Both are **append-only overlays**, never a mutation of the assert or of each other.
- Later slices add cross-thread reconciliation resolution (a link between two threads) — a different
  *shape* (two threads, not one), deliberately **out of scope here** (§8).

## 3. The signed body (day-one shape)

The dose value and the correction target ride the **signed event**, so their shape is the
can't-cheaply-retrofit piece (the [ADR-0042](../../spec/decisions/0042-concrete-attachment-reference-shape.md)
lesson). Fixed day-one:

```
clinical.medication-dose-change.asserted:
  medication_id : UUID                       # references the thread (NOT minted here)
  patient       : UUID
  dose:
    amount      : decimal-string | omitted   # honest-unknown — "upped it, dunno to what" (§3.7)
    unit        : DoseUnit       | omitted
  effective     : { value, precision } | omitted   # §3.6 t_effective — when the dose changed
  info_source   : patient-reported | clinician-observed | external-record | unknown   # REQUIRED (a new clinical claim)
  reason        : string | omitted           # "titration", "renal dosing"

clinical.medication-dose-correction.asserted:
  medication_id : UUID                       # references the thread
  patient       : UUID
  corrects      : UUID                       # the dose event being fixed (the assert's event_id = point 0, or a prior change's event_id)
  dose:
    amount      : decimal-string | omitted   # correct-to-unknown ALLOWED (principle 4: strike a false precision)
    unit        : DoseUnit       | omitted
  info_source   : ... | omitted              # OPTIONAL (a record fix, not necessarily a new claim)
  reason        : string | omitted           # "mis-keyed"
```

**The uncertainty floor (principle 4, §3.7).**

- A **dose-change with an unknown amount is a first-class write** — *"they upped my metformin, don't know to
  what"* is a real, common ED collateral-history statement (a known change, an unknown target). The change
  is meaningful even without the number.
- A **correction to *unknown* is legitimate** — downgrading a false precision (*"the 40 mg I typed was a
  guess; strike it, dose unknown"*) is exactly principle 4's "an imprecise near-truth beats a precise
  untruth." So the correction's `dose` is optional, not required.

**`corrects` is a plain UUID, and its target is NOT required to exist locally.** In set-union sync a
correction may replicate before the dose event it fixes, or that event may live on another node. Requiring
local presence would break the availability floor (AP,
[ADR-0001](../../spec/decisions/0001-fat-postgres-thin-daemon.md)) — the same reasoning that lets a
cessation precede its assert in slice 1. **Deliberately NOT the built-in `target_event_id` field**, whose
`submit_event` path (db/005) forces the target to exist locally and is tied to
`targets_other_author=TRUE`; a distinct `corrects` field keeps corrections offline-first and additive.

**`DoseUnit`** reuses slice-1's controlled-token-plus-free-text-escape-hatch (units confusion is a
med-error class, so units are not bare free-text). **`effective`** reuses the existing uncertainty-capable
date representation (`{value, precision}`) established by `started`/`stopped` — no bespoke date type.

**Legibility twin (§3.13).** Every event carries the mandatory mechanically-derived plaintext twin —
change → *"Dose changed to 80 mg (effective 2025-06) — titration [clinician-observed]"*; correction →
*"Dose corrected to 20 mg — mis-keyed"* (the old value lives on the corrected event, so the twin states the
corrected value; always non-empty). There is exactly one event; the prose is a rendering *of* it.

## 4. The in-DB floor (the safety-critical seam)

Maximally permissive on clinical content (principle 4 / paper-parity — **never block a dose write** beyond
structural integrity), strict on structure. One shared `cairn_check_medication_dose(p_type, b)` (mirrors
slice-1's one-function-two-verbs `cairn_check_medication_assertion`):

- **both verbs:** well-formed `medication_id` + `patient` UUIDs; append-only.
- **dose-change:** non-empty `info_source` (mirrors the assert — a new clinical claim carries its
  provenance) **and** at least one substantive field present (`dose.amount` / `dose.unit` / `effective` /
  `reason`), so a pure no-op `{medication_id, patient}` event is refused. Nothing else forced.
- **dose-correction:** well-formed `corrects` UUID (existence NOT checked — offline-first). `dose` optional
  (correct-to-unknown). `info_source` optional.

**Classification — both are `additive`, `targets_other_author=FALSE`** (registered in `event_type_class`).
The clinical call this encodes: **a correction is additive, not a suppression.** It does not
foreclose/hide the corrected event (which stays verbatim in `event_log` and in the dose history, flagged
`corrected`); it only wins the *current* dose in the projection — exactly as a corrected DOB works in
demographics today. Therefore the [ADR-0043](../../spec/decisions/0043-suppression-self-only-disagreement-is-additive.md)
suppression owner-gate **does not apply**: clinician B may correct clinician A's mis-keyed dose (normal
clinical practice; the original is preserved for audit). *(If cross-author dose correction ever needs
gating, that is a later, separate policy decision — not baked in here.)*

**Twin dispatch.** Two more branches are added to the `cairn_event_twin` dispatch per the existing
verbatim-branch pattern. This grows the [#173](https://github.com/cairn-ehr/cairn-ehr/issues/173)
dispatch-fragility already filed; slice 2 follows the idiom rather than expanding scope into that refactor.

## 5. The projection — a dose timeline (`db/032_medication_dose.sql`)

A medication thread gets a **dose timeline**: the initial dose from the assert (point 0) plus one point per
change, ordered by *effective* date. Two trigger-fed tables mirror slice-1's statement/cessation split, so
folds are **arrival-order-independent and out-of-order convergent**:

- **`medication_dose_event`** — one row per dose *point*, keyed by `dose_event_id` (= the event's
  `event_id`, available as `NEW.event_id` on the trigger). Point 0 is seeded from the assert by a **second,
  additive trigger** on `clinical.medication.asserted` (so all dose logic lives in one table; **db/031 is
  untouched**). Columns: `medication_id, patient_id, amount, unit, effective_value, effective_precision,
  is_initial, info_source, reason` + HLC overlay columns (`hlc_wall, hlc_counter, origin, content_address`).
- **`medication_dose_correction`** — corrections, keyed by the **target** `dose_event_id` they fix
  (a correction overlays a specific point). HLC-wins if one point is corrected twice; converges if a
  correction arrives before its target.

**Views** (winner/ordering tiebreaks over TEXT keys use `COLLATE "C"` per
[ADR-0045](../../spec/decisions/0045-collation-independent-projection-tiebreaks.md) — two nodes must pick
the same current dose):

- effective per-point value = `COALESCE(correction.amount, event.amount)` (+ a `corrected` boolean).
- **`patient_medication_dose_history`** — the ordered titration trail for a thread; exposes `dose_event_id`
  (so the CLI can target a point) and the `corrected` flag.
- **current dose** = the point with the latest `effective` date (ties broken by
  `(hlc_wall, hlc_counter, origin, content_address)`, `COLLATE "C"`).
- **`patient_medication_current`** is reworked to source its dose from the current point (was slice-1's
  frozen assert dose) **and to expose `dose_event_id`** so "correct the current dose" needs no id.

### 5.1 The current-dose winner rule (bitemporal — §3.6)

"Current" must follow the **effective** timeline, not the recording timeline — otherwise a clinician
backfilling history (*"actually it's been 40 mg since 2020"*, recorded today) would wrongly override a real
later increase to 80 mg. So:

1. **Primary key = effective time, descending.** The latest-effective point is current. Effective values
   are the existing uncertainty-capable ISO-ish strings (`"2024"`, `"2025-06"`), which **sort chronologically
   as plain strings** (the ISO-8601 property) under `COLLATE "C"` — no date parsing in the projection.
2. **A point with no stated effective date** is assigned an effective sort key from its **recording time**
   (`hlc_wall`, rendered as an ISO string) — an honest lower bound (you knew it at least by when you recorded
   it). This makes an undated change current over older-effective points but never over a genuinely
   later-effective one.
3. **Ties** (equal or both-derived-from-recording effective keys) break by
   `(hlc_wall, hlc_counter, origin, content_address)` under `COLLATE "C"` — fully convergent, all inputs
   event-carried and identical on every node.

**Documented approximation:** a `year-range` effective value (`"2020/2024"`) sorts by its min-endpoint
prefix under the string comparison. Acceptable and low-stakes — a dose *change* effective date is
near-always a single date, not a range; refining range ordering is deferred.

An **orphan correction** (target not yet local) lands in `medication_dose_correction` and lights up its
point only once the target arrives — no data loss, no dishonest placeholder (the slice-1 orphan-cessation
treatment). "Current dose" stays a **projection convenience, not a truth claim** (principle 4).

## 6. Node orchestrators & CLI (`cairn-node`)

Mirrors slice 1; **device-additive** throughout (`recorded` contributor, no responsibility attestation —
consistent with slice 1, since human-attested clinical responsibility is not built yet):

- **`crates/cairn-event/src/medication.rs`** — pure `build_dose_change_body` / `build_dose_correction_body`
  + `render_*_twin`. This file is ~316 lines already; adding two builders + twins + tests may push it past
  the 500-line guideline → **split into a `medication/` module** (`assert`, `cessation`, `dose`) if needed.
- **`crates/cairn-node/src/medication.rs`** — async `change_dose(...)` and `correct_dose(...)` orchestrators
  (mint `event_id`, stamp HLC, sign, 1-arg `submit_event`).
- **CLI:** `medication change-dose <medication_id> --amount --unit [--effective --precision --reason
  --info-source]`; `medication correct-dose <medication_id> [--target <dose_event_id>] [--amount --unit
  --reason --info-source]`. **`--target` defaults to the current dose point** (looked up from
  `patient_medication_current.dose_event_id`), so the common bedside "fix what's shown" case needs no id.

## 7. Testing (TDD, failing-test-first)

Pure builder tests (no DB): bodies omit-not-null; correct-to-unknown carries no `dose`; unknown-amount
change is accepted; twins non-empty and read naturally. **DB-gated** integration tests: a change appears in
`patient_medication_dose_history` and moves `patient_medication_current`'s dose; effective-date ordering
picks the right current dose across out-of-order arrival; **a backdated change does NOT become current over a
later-effective change** and an **undated change** becomes current over older-effective points (the
bitemporal §5.1 rule); a correction overlays its target and sets `corrected`; **a correction arriving before
its target converges** (offline-first); correct-to-unknown; two corrections of one point resolve by
HLC-wins; the empty-dose-change reject; point-0 seeding from the assert; a positive control. Crypto material in tests is **runtime-derived**, never literal (house rule 6 / issue
#146).

## 8. Explicitly out of scope for slice 2

Deferred, each its own later slice:

- **Cross-thread reconciliation resolution** ("these two threads are the same real medication" — a
  link/supersede between threads; the never-merge-always-link shape) — slice 3.
- **Correcting a dose event's *effective date* or *reason*** (slice 2 corrects the dose *value* only).
- **Human-attested clinical responsibility** on a dose event (device-additive throughout, like slice 1).
- **Fuzzy reconciliation, Tier-A dictionary, DDI, structured sig/frequency, route** — unchanged from slice-1
  §8.
- **The #173 twin-dispatch registry refactor** — slice 2 adds two branches the old way; the refactor is its
  own slice.
- **Extending the [#157](https://github.com/cairn-ehr/cairn-ehr/issues/157) HLC-collision advisory onto the
  new dose projections** — a consistency follow-on, deferred with the medication projections generally.

## 9. Design tensions recorded (resolved)

- **(A) change vs correction** → kept as two machine-legible verbs (paper-parity: new line vs
  strike-through); both `additive` in the classification sense.
- **(B) correction target** → an explicit `corrects` UUID on the wire (any point fixable, no future wire
  change), existence not required (offline-first); the CLI defaults it to the current dose for ergonomics.
- **(C) cross-author correction** → ungated (a correction is not a suppression; the original is preserved) —
  the maintainer's clinical call.
- **(D) current dose is stale-prone** → inherited from slice 1; the dose history makes the *trail* visible;
  active-review/last-confirmed still deferred.
- **(E) which dose is "current" under uncertain/backdated dates** → the bitemporal §5.1 rule (latest
  *effective*, ISO-lexical string sort so no date parsing; null-effective → recording time; HLC/content
  tiebreak), so a backfilled history never overrides a real later change; year-range ordering approximated
  (min-endpoint) and refinement deferred.
