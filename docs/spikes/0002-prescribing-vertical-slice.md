# Spike 0002 — Prescribing Vertical Slice (the active-write model, end to end on the Pi)

- **Status:** Proposed (build-prep)
- **Date:** 2026-06-18
- **Validates:** [ADR-0020](../spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)
  (active-write / thin-encounter / the legibility twin **born at authoring time**),
  [ADR-0022](../spec/decisions/0022-validated-submit-surface-the-write-path.md) (the validated `submit_event`
  surface — the floor in the DB), [ADR-0008](../spec/decisions/0008-point-of-care-identity-possession-and-salvage.md)
  (the armed write-context binding `(clinician, patient)`), and the governing principles it all serves:
  **3 (paper-parity — the make-or-break bet)**, **4 (acknowledged uncertainty / forced-manual dosing)**, and
  **11 (legibility twin = the one artifact)**. Re-exercises [ADR-0001](../spec/decisions/0001-fat-postgres-thin-daemon.md)
  (projection → chart-read cost) from *inside a real workflow* rather than on synthetic load.
- **Builds on:** the Spike 0001 walking skeleton (`poc/walking-skeleton/`). It **reuses** the signed event
  envelope, sign/verify, the trigger-maintained projection, and the chart read; it **adds** the write path
  (`submit_event` + a registered validator + twin-at-authoring) and the type-through front-end. Same "seed of
  the real implementation," grown by one vertical slice.
- **Does not ratify anything yet.** Validate-then-ratify, exactly like Spike 0001: this is how we learn whether
  the active-write model survives contact with implementation and a real clinician. The ADR updates are written
  *after* the run, citing its numbers.

> [!NOTE]
> Build-prep, not architecture. The numbered spec (§1–§11) and the ADR log describe a *decided* design; this
> spike *exercises* that design against reality — one real clinical workflow, on the Pi, against the stopwatch
> principle 3 demands.

---

## 1. Why this spike, and why now

The design has been pressure-tested hard against **clinical case-mining** — and it has absorbed every case. It
has **never** been tested against **implementation reality**, which is where most of these decisions actually
get decided. This is the cheapest, highest-information collision available: take one real clinical task —
**writing a prescription** — end to end through the layers the walking skeleton stubbed (the write path, the
active-write UX, the twin born at authoring, forced-manual dosing), and **benchmark it against the paper script**
in the time / steps / cognitive-load terms paper-parity ([§1.2](../spec/vision.md)) requires.

Three reasons it is the right slice:

- **It is the test that tells you which ADRs were wrong.** Case-mining proves the model can *represent* the
  workflow; only building it proves you can *author, store, project, and read* it at paper-beating cost. A FAIL
  here is the most valuable signal the project can produce right now.
- **It runs on the same Pi as Bet B** ([Spike 0001 §6](0001-walking-skeleton-wan-sync-and-pi-cost.md#6-bet-b-projection-keystore-cost-on-the-pi-prepared-awaiting-the-board)),
  so it composes with the hardware spike already prepared, and re-confirms ADR-0001 inside a real workflow.
- **It is the artifact that aligns the team:** something concrete for the distributed-systems reviewer to
  red-team, and a real flow for the workflow/UI designer to shape.

**Why prescribing specifically.** It is the highest-frequency active-write task in practice; it spans the full
difficulty range in one workflow (a trivial repeat → a weight-based paediatric dose → a renally-adjusted dose →
a controlled drug); it is the canonical [ADR-0020](../spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)
`rx!` type-through case; and it forces the sharpest open UX collision — **principle 3 (be as fast as paper)
versus principle 4 (refuse to fabricate a dose)** — which is exactly the tension worth resolving against a real
clinician before the team commits.

---

## 2. What this spike is *not*

- **Not a product / not the prescribing module.** No real drug database, no interactions engine, no
  PBS/formulary integration, no electronic transmission to a pharmacy, no controlled-drug regulatory workflow.
- **Not the full submit surface.** Exactly **one** event type (`medication.prescribed`) through the ADR-0022
  `submit_event` shape with **one** registered validator — enough to prove the floor is real, not to build the
  closed operation set.
- **Not the matcher, the FHIR façade, federation, break-glass, or the full keystore.** Those are out of band
  here, as in Spike 0001 §2.
- **Reserve the day-one shapes; stub the depth.** The event shape, the encounter ride, the twin slot, and the
  validator-dispatch *door* are real; the formulary is a hand-seeded handful of drugs and the forced-manual
  rules are a handful of entries.

---

## 3. The slice (what gets built on top of the skeleton)

Reuse, don't rebuild. On the existing `poc/walking-skeleton/`:

1. **Arm a write-context** ([ADR-0008](../spec/decisions/0008-point-of-care-identity-possession-and-salvage.md)).
   Bind `(clinician-actor-key, patient-uuid)` and mint a **thin encounter id**
   ([ADR-0020](../spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md) —
   an opaque grouping id asserting nothing about formality). *Stub:* a CLI `arm` that holds the context; the
   physical possession gesture (§5.11) is out of scope — the **binding shape** is the point, not the hardware.
   *Safety-critical seam* (the `(clinician, patient)` binding).
2. **Type-through front-end** (ADR-0020). An `rx!` parser turning a typed line —
   `rx! amoxicillin 500 tds 5/7` — into a structured prescription draft, non-modal. Promote the
   `poc/walking-skeleton/` / `scratch/ui-sketches/` `rx!` sketches. *Fit-for-purpose (Python).*
3. **The `submit_event` write path** (ADR-0022). Promote the skeleton's `emit_event` into the submit shape:
   dispatch to a **registered `medication.prescribed` validator** → authorship stamp (ADR-0008) → **derive the
   legibility twin** (the human-readable script line — born *here*, at authoring) → canonicalize + sign →
   idempotent append. *Safety-critical (Rust / in-DB).*
4. **The `medication.prescribed` validator.** Type-checks the structured prescription, applies the
   **forced-manual** rule for special populations, and refuses an event that is malformed *or* satisfies a
   required field only by fabrication (principle 4). *Safety-critical.*
5. **Mini-formulary + forced-manual rule table.** A hand-seeded set (≈10 drugs) including one weight-based
   **paediatric**, one **renally-adjusted**, one **pregnancy-category** and one **controlled** drug — enough to
   exercise smart-default-vs-forced-manual. *Fit-for-purpose; depth stubbed; the easyGP source deferred (§8).*
6. **Projection + chart read.** Extend the trigger-maintained projection (a `medication_current` sibling to
   `patient_chart`) so the `AFTER INSERT` path updates the active med list; the chart read assembles the current
   meds + the script line **from the stored twin**. Reuse the skeleton's `chart` command. *Safety-critical
   projection; fit-for-purpose read.*

---

## 4. The bets

C1 is the governing bet; a fail there is a fail of the active-write thesis, not a tuning task.

| # | Question | PASS threshold |
|---|---|---|
| **C1** | **Paper-parity (governing).** Is type-through prescribing no slower / no more steps / no higher cognitive load than the paper script? | Type-through **≤ paper on time AND steps** for the common cases; **within parity** on the hard cases (where forced-manual is a parity-*legal* deliberate step — see §5); cognitive load ≤ paper. Method in §5. |
| **C2** | **One artifact (principle 11).** Is the displayed script line the legibility twin, mechanically derived from the *one* structured event at authoring time — with no second free-text artifact? | Displayed line **==** the stored `plaintext_twin`; the structured event is the **sole** source; event → twin → display is deterministic. No divergent free-text note exists. |
| **C3** | **The floor is in the DB (ADR-0022).** Does a prescription enter the log *only* via `submit_event` → registered validator → signed append? | Valid rx **appended + signed + projected**; a raw/direct `INSERT` of an unvalidated or unsigned prescription is **rejected** by the in-DB floor (validator + content-address CHECK / RLS); a legible reason is returned; **no path** yields an unsigned/unvalidated row. |
| **C4** | **Acknowledged uncertainty / forced-manual (principle 4).** For a special-population case, is the auto-dose withheld and a manual decision forced — while the inputs stay satisfiable by honest uncertainty, never fabrication? | The paediatric/renal path **blocks auto-dose** and demands a manual entry; an **unknown weight is recordable as `unknown`** (distinct from a fabricated number and from not-yet-asked); the event records the uncertainty. |
| **C5** | **Projection → chart read on the Pi (ADR-0001).** Does the trigger maintain the med projection within budget, and does a chart read including the just-written script assemble sub-second, in a real workflow, on the Pi? | Maintenance within the Bet B **B1** budget; chart read sub-second (**B2** budget); the new script appears **immediately and correctly**. |

**Stretch, named-not-built** (reserve the shape, defer the depth): **C6** encounter → order provenance fold
(`result → order → order.encounter`, ADR-0020); **C7** `sign-as` salvage of a mis-armed context (ADR-0008).

**FAIL signals & what they'd mean.** C1 slower/heavier on the *common* case → the active-write promise fails,
ADR-0020's write model needs rework (the highest-value thing this spike can find). C2 a second divergent artifact
→ the principle-11 "twin is the one artifact" claim breaks at authoring. C3 any path to an unsigned/unvalidated
row → ADR-0022's "floor in the DB" is incomplete and the principle-12 compatibility guarantee leaks. C4 a forced
fabrication → a principle-4 violation in the write path.

---

## 5. Measuring paper-parity (the governing method)

Paper-parity is *governing law*, so C1 is measured, not asserted — and you are the right instrument (an EM
physician who writes scripts all day).

- **A fixed basket of ~10 representative real prescriptions** spanning the difficulty range: a trivial repeat;
  a standard adult course; a **weight-based paediatric** dose; a **renally-adjusted** dose; a
  **pregnancy-flagged** drug; a **controlled** drug; a tapering regimen; a PRN; a topical; and a multi-item
  script. The tail is where parity is won or lost, so the basket must include it.
- **For each, in randomised order with a few repetitions:** time and count steps/keystrokes for **(a)** the
  paper equivalent (write the script *and* file it in the chart) and **(b)** the type-through flow. Record a
  simple **1–7 cognitive-load** self-rating per script.
- **Report the distribution, not a single mean** — a model that wins the median and loses the tail has not
  achieved parity.
- **State the principle-3 / principle-4 reconciliation up front, so the hard cases are read honestly.**
  Forced-manual on a special-population dose is **not** a parity regression: the *paper* script requires the
  identical clinical computation (you must work out the renal dose either way); the architecture merely refuses
  to **fabricate** it. So the hard-case bar is *"parity with the thinking paper also demands,"* not *"as fast as
  a trivial script."* (This is the ADR-0020 forced-rationale-gate logic applied to dosing: never block the
  routine; for the genuinely uncertain, demand the decision rather than confirm a default.)
- **FAIL signal:** if type-through is slower or heavier on the **common** case, the active-write promise fails
  and ADR-0020 needs rework — record it plainly; that is the most valuable result this spike can return.

---

## 6. Exit criteria → ratification

When the slice runs (on the Pi, with the basket):

1. **If C1 passes,** ADR-0020's active-write / twin-born-at-authoring model is validated against real
   implementation *and* a real clinician — fold the measured numbers into the ADR-0020 area and the §1.2
   paper-parity prose, and promote the `rx!`/`tx!` parser + forced-manual table out of `scratch/`.
2. **If C2 + C3 pass,** the ADR-0022 submit-surface floor and the principle-11 one-artifact claim hold under
   implementation — the first **end-to-end** proof that *"many front-ends, one record"* (principle 12) is real,
   not just specified.
3. **If C4 passes,** the principle-4 forced-manual reconciliation is shown parity-legal in practice.
4. **If C5 passes,** ADR-0001 is re-confirmed from inside a real workflow (Bet B confirmed it on synthetic load).
5. **Any FAIL is the high-value outcome:** it names which ADR/principle met implementation reality and lost —
   precisely the collision this spike exists to cause. Record it; it routes a question back to the design.
6. Either way, the slice becomes the **second growth ring** of the real implementation — built to keep, like the
   skeleton, not thrown away.

---

## 7. Blast-radius (§9)

- **Safety-critical → Rust / in-DB:** the `submit_event` pipeline; the `medication.prescribed` validator
  (including the forced-manual gate); the twin derivation at authoring; the canonicalize + sign step; the
  projection trigger. A defect here mis-prescribes or silently corrupts the record.
- **Fit-for-purpose → Python / front-end:** the `rx!` parser + type-through state machine; the seeded
  mini-formulary; the chart-read presentation; the paper-parity harness. A defect is caught immediately or is
  cosmetic.
- **The one safety-critical seam:** **parsed-draft → `submit_event`** — the boundary where a fit-for-purpose
  front-end hands a write to the in-DB floor. This is the recurring seam motif, and the concrete instance of
  ADR-0021's *"UIs call submit-functions, never raw `INSERT`."* Get this seam right and a wrong-for-its-clinic
  UI still cannot produce a wire-incompatible or unsigned event.

---

## 8. Dependencies & honest gates

- **Drug data + forced-manual rules.** This slice uses a **hand-seeded mini-formulary** to avoid blocking on the
  easyGP schema access (the real source, deferred — see HANDOVER build-prep). Reserve the shape
  (formulation / dose / special-population flags); stub the depth.
- **Hardware.** Runs on the **same Pi as Bet B** (compose with Spike 0001 §6). The front-end may run on a laptop
  against the Pi's PostgreSQL, or entirely on the Pi.
- **Out of scope here:** the **prefetch / materialization warming daemon** (ADR-0020 deferred / an ADR-0001
  optimization) — measured separately on its own; this slice deliberately measures the **cold** write → read
  path, so the C5 numbers are a floor, not a best case.
