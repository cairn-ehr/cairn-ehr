# Case Study 0001 — 16 GP-software failure modes vs. the Cairn primitives

- **Status:** **Mined 2026-07-11.** All 16 cases absorbed by existing primitives; **no new architecture
  required.** Three items are flagged for action (§4), one of which touches the demographics slice under
  construction *right now*.
- **Source:** a 16-part column on shortcomings of the clinical software used in Australian general
  practice, written by **Dr Oliver Frank** (a GP and, at the time, a member of the RACGP Expert Committee
  — eHealth and Practice Systems) and published monthly in an Australian doctors' magazine between
  November 2015 and June 2017. Each instalment states a real workflow failure, proposes a fix, and carries
  responses from the incumbent software vendors and, often, unfiltered reader comments from practising
  GPs. Shared with the project by a colleague as case-mining material. The failure modes below are
  paraphrased; the incumbent products are deliberately **not named** (product neutrality —
  see the mission's anti-capture stance).
- **Method:** for each instalment, extract the underlying *record-model* failure (not the UI dressing),
  then ask whether an existing Cairn primitive absorbs it or whether it forces new architecture. The
  reader comments are treated as first-class field data — practising clinicians reacting, often bluntly,
  to exactly the kind of change Cairn proposes.
- **Validates (in aggregate):** append-only + causal ordering (principle 1); identity-is-a-claim /
  link-never-merge (principle 2); paper-parity, incl. the *confirmation-dialog prohibition* (principle 3);
  acknowledged uncertainty (principle 4); policy-neutral mechanism (principle 9); compositional authorship
  (principle 10); legibility across time / additive evolution (principle 11); uniform core, plural edges
  (principle 12). Plus [ADR-0003](../spec/decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md)
  (bitemporal time), [ADR-0009](../spec/decisions/0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md)
  (salience / the acknowledgment floor), [ADR-0013](../spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md)
  (attachments), [ADR-0020](../spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)
  (active-write model), and [ADR-0025](../spec/decisions/0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md)
  (canonical interlingua + local-terminology overlay).

> [!NOTE]
> A colleague reasonably read this column as "mostly UI/workflow, Australia-specific." At the surface that
> is true. Underneath, **every instalment is a workaround for the same defect**, and it is not a UI defect
> — see the meta-finding.

## 1. The meta-finding

Across all 16 instalments there is **one** underlying defect, and it is architectural, not cosmetic:

> The incumbent packages store clinical facts as **flat, overwrite-in-place, undated, unattributed,
> uncoded plain text** — with no provenance, no validity window, and no first-class representation of
> *absence*, *refusal*, or *uncertainty*.

Every proposal in the column is a per-feature bolt-on for something that falls out **for free** from an
append-only, bitemporal, signed, attributed event model. Cairn does not need sixteen features to answer
this column; it needs the four governing principles and the event core it already has. That is the
case-mining payoff: **sixteen independent, real, field-sourced failure modes, zero new architecture** —
and several of them are crisp illustrations worth citing in the spec.

The most striking confirmation is not in the articles but in the **reader comments**: the single most
consistent GP objection, across unrelated instalments, is to **prompt-fatigue and mandatory-field
coercion** ("more work, more typing", "another click to get rid of the message", "insulting to solo GPs",
"software that will prevent us from doing certain things"). That is live field evidence for two Cairn
positions at once — principle 3's ruling that **confirmation dialogs are not an acceptable safety
mechanism**, and principle 9's **mechanism-not-policy** stance (a quality signal must be advisory to a
human, never a hard block).

## 2. Per-instalment map

Ordered by the column's own numbering. "Verdict" is *Absorbed* unless noted; the "→ flag" column points
at the action items in §4.

| # | Failure mode (paraphrased) | Absorbing primitive | Notes / → flag |
|---|---|---|---|
| 1 | Provider directory treats every professional as a soloist; a practice/organisation is not an entity, so shared address/contact details are re-keyed per person | Entities are first-class; a person **links to** an org (append-only link event), never a denormalised copy — the identity algebra (principle 2) generalises from patients to orgs | A reader independently asks for **referral-letter tracking** ("no tracing of referrals … too easy to lose track", cites the Malycha case) → **flag ②** |
| 2 | Contact/family/social details drift; software never prompts to re-confirm; stored as plain text with no "last confirmed by whom/when" | Bitemporal time + principle 4; **a re-affirmation is its own event** | Directly hits the demographics §4.4 slice → **flag ①** |
| 3 | No structured way to record a patient's **refusal of** or **ineligibility for** care; temporary vs permanent; must suppress reminders | Principle 4 generalised: absence has ≥4 *positive* causes (not-offered / refused-temporarily / refused-permanently / not-indicated), each an event; reminder suppression is a projection | **Best principle-4 exemplar in the batch** |
| 4 | Appointment system records the appointment *given*, never the one *requested* | "Intent event" distinct from "outcome event"; the gap is the analytics substrate (rhymes with ordered-vs-resulted, referred-vs-completed) | Mostly practice-management layer — **weakest case for Cairn's clinical core**; the colleague's "UI-specific" read is most accurate here |
| 5 | Family history unstructured/uncoded; no way to **link** relatives so a diagnosis on one updates the family history of the others; privacy of the relatives | Cross-patient **link** event (patient-is-parent-of-patient) + a *derived* family-history assertion whose provenance points at the source event, **gated by a visibility/consent event** ([ADR-0006](../spec/decisions/0006-visibility-scope-replication-and-the-safety-projection.md)); coding is additive ([ADR-0025](../spec/decisions/0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md)) | "record age *of onset* / *of death*" = t_effective anchoring again |
| 6 | No document records what **changed** when a medicine is added/replaced/re-dosed; "is this new drug additional or a replacement?" is ambiguous | Change-highlighting is a **diff projection over the event stream** — trivial when history is append-only; a replace is a stop event *referencing* the prior med | Legacy pain is a direct symptom of overwrite-in-place |
| 7 | Patients take medicines with no in-practice prescription (OTC, prescribed elsewhere, trial meds); GP never learns of stops | Temporal currency + **compositional authorship** (principle 10): the med list is a merge of many contributors' assertions; a blinded trial med ("aspirin *or placebo*") is a genuine principle-4 unknown | "GP confirmation recorded and stays visible" = re-affirmation event → **flag ①** |
| 8 | Health-summary med list is alphabetical, undated, no indication, no reason-for-stopping, can't show past meds | Several timestamps per med assertion (added / last-represcribed / last-confirmed-current), typed stop-reason, med→problem indication link — all fall out of the event model | Third instalment to explicitly need **"last confirmed as current"** → **flag ①** |
| 9 | GPs can't easily find the cheapest dispensing option for a patient | — | **Out of scope**: a pricing/commercial concern, not a record-model problem. Included only as the clearest "not a Cairn concern" boundary marker |
| 10 | No prescribing alert when a nephrotoxic/renally-cleared drug is given with a low or **absent** recent eGFR | **Advisory-actor layer** (§9 fit-for-purpose), *not* the safety-critical core; Cairn's job is the absence-aware eGFR substrate the advisor reads | "no eGFR in 6 months" must be a *queryable* state, not a null silently read as "fine" (principle 4). A GP's manual workaround — pinning a dated note "so I see it every time" — is a human doing the salience layer's job by hand ([ADR-0009](../spec/decisions/0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md)) |
| 11 | Pathology requests carry no clinical detail; propose requiring a structured reason (screening / dx / monitoring / admin), coded from the problem + medicine lists | The request event **references** the problem and medicine events that motivate it; the reason is reusable (a reader notes it doubles as the consult note); legibility twin makes it human-readable | A request is an **open loop** until a result returns → **flag ②**. Vendors + readers push back hard on *mandatory* fields → keep it additive/soft-policy (principle 9) |
| 12 | GP can't see a patient's appointments with *other* providers; care coordination suffers. Vendors: capture accuracy will be low; readers: "patients lie" | Principle 4 + principle 2: record the external appointment as a **low-confidence, patient-sourced claim with provenance**, never as fact; cross-provider visibility is the federation/sync story | The vendors' own objection ("errors of omission will be huge") is the argument *for* claim-with-confidence over fact |
| 13 | Patients have no way to add their **own** notes / reason-for-visit; the GP is an imperfect scribe (bias, mis-emphasis, selective recording) | **Compositional authorship** (principle 10): the patient is a contributor; their entry is separately attributed and **append-only** | A reader states Cairn's model unprompted: patients "may add … but should not have the power to remove what other people [have] written" — that *is* principle 1 + 10. Reception-visibility worry = principle 6 |
| 14 | Impossible values accepted (temp −30, BP 999/1); free-text vs coded diagnosis; an open critical-result recall can be ignored; record fragmentation across tabs | Multiple — the **richest** instalment | See **flags ②③**; legibility twin (free text *always* legal, coding additive, principle 11); "can't close the chart with an unacknowledged critical recall open" is the open-loop case *and* a paper-parity trap → **flag ②** |
| 15 | No patient-facing summary of preventive care due; and no record that such a summary was **produced and given**, nor a copy of what was given | An immutable event capturing the communication artifact (what was handed over, when) + the artifact as a content-addressed blob ([ADR-0013](../spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md)) | "store a copy of the information that was given" = append-only + attachment + legibility |
| 16 | Self-prescribed substances siloed in "Social History" — the wrong bucket ("many use these while alone"); want one stream, many views | **Uniform core, plural edges** (principle 12) + principle 11: store attributed substance-use assertions, **derive** many displays (by source, by therapeutic class); patient-entered items **labelled by contributor** (principle 10) | Good illustration that a rigid schema *bucket* is an anti-pattern; the view is a projection, not the storage shape |

## 3. Cross-cutting themes

The value is less in any single instalment than in what **recurs**:

1. **"Last confirmed as current" / re-affirmation without change** — instalments **2, 7, 8** each demand
   it explicitly. This is the single most-repeated ask in the column. See **flag ①**.
2. **Open-loop obligations / closing the loop** — instalment **14** (critical-result recall), **1**'s
   reader (referral tracking, Malycha case), **11** (request → result), **12** (external appointment as a
   pending event). Recurs four times across unrelated topics. See **flag ②**.
3. **Absence / refusal / ineligibility as positive, recordable states** — **3, 10, 11, 15**. Principle 4,
   strongly validated from the field.
4. **Provenance / compositional authorship** — **5, 7, 12, 13, 16** (external-sourced, self-prescribed,
   patient-authored). Principle 10.
5. **Append-only / overlay / immutability, stated by clinicians who've never heard of Cairn** — **13**
   ("add but never remove others' entries"), **6** (change-highlighting needs history). Principle 1.
6. **Entities & links, not merges** — **1** (org as entity), **5** (family links). Principle 2 generalises
   beyond patients.
7. **Legibility twin + additive coding, never a hard gate** — **14, 11, 5, 16**: free text is always
   legal, code is an additive suggestion, mandatory-coding is rejected by working GPs. Principle 11 +
   the constraint rule in **flag ③**.
8. **Mechanism-not-policy / advisory-not-blocking** — the loudest, most consistent reader signal across
   the whole column: prompt-fatigue and mandatory fields are hated. Principle 3 (no confirmation-dialog
   safety) + principle 9 (mechanism, not policy).

## 4. What's worth acting on

Being adversarial, as the case-mining discipline requires — these are where the design is *not* trivially
done, or where the slice under construction is directly implicated.

### ① Re-affirmation-without-change — verify the demographics slice actually carries it

Overlay cleanly handles a *changed* value. But the literal subject of instalments **2, 7, and 8** is the
*unchanged* value whose **currency** is in doubt: "I checked; still true; as of today." That needs **two
timestamps on one fact** — `asserted-since` (the first assertion's `t_effective`) *and*
`confirmed-current-as-of` (the latest no-op re-affirmation). A projection that only tracks "the last event
that *set* this field" collapses the two and cannot tell *"stale, last touched three years ago"* from
*"re-affirmed yesterday, value unchanged."*

This is exactly the demographics §4.4 identifier tier being built now (`db/010…` + `cairn-event`).
**Action:** confirm the current event shape and projection represent a **re-affirmation event** distinct
from a value-change event, and that the projection can expose `confirmed-current-as-of` separately from
`asserted-since`. This is the one item to check against code, not just wave at.

> [!NOTE]
> **Checked against code (2026-07-11) → [issue #163](https://github.com/cairn-ehr/cairn-ehr/issues/163).**
> Good news: the append-only envelope already *records* a re-affirmation (it is another
> `demographic.field.asserted` with the same value and a later HLC; distinct `content_address`, both persist)
> — **no can't-retrofit gap**. The gap is at the **projection** layer: every `patient_*` demographic
> projection (`db/010`–`db/014`) keeps a **single winner-HLC triple** (`asserted_hlc_*`, named `last_hlc_*`
> in names) that a re-affirmation *overwrites*, collapsing asserted-since into confirmed-current-as-of; the
> only other time columns (`first_seen`, `updated_at`) are local `clock_timestamp()` stamps — non-convergent
> and clinically meaningless. So neither currency question is answerable from the projection today. Design
> questions (t_effective-vs-HLC, same-value equality, event-type choice) are tracked in #163.

### ② A possible open-loop / obligation projection

An order/recall/referral that has no terminal, referencing acknowledgment event is an **open obligation**,
and it must be surfaced at the next point of contact (instalment 14's critical-result recall; the referral
and pathology cases). Two things to settle:

- **Does it need naming?** It is *not* one of the enumerated identity/actor algebras. It *probably* falls
  out of generic event-reference chains (an order with no closing reference = open), routed and prioritised
  by the salience machinery of
  [ADR-0009](../spec/decisions/0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md).
  But "probably falls out" is an assertion to **stress-test**, not assume — it may warrant an explicit
  spec paragraph or a named "loop-closure" projection.
- **The paper-parity trap.** Instalment 14's reader wants a hard "you *can't* close the chart with an
  unacknowledged recall open" — which is precisely the confirmation-dialog mechanism principle 3 forbids.
  The Cairn-correct answer restores the **physical affordance** (the flagged chart, the pile of
  results-to-action) via structure and salience, not a nagging modal. That is real design work and the
  closest thing to genuinely new surface in the whole batch.

### ③ The impossible-vs-uncertain constraint boundary

Instalment 14 wants the database to reject temp −30 / BP 999/1. Correct — that belongs in the in-DB floor
(constraints). But principle 4 forbids rejecting the merely *uncertain or imprecise* ("around 140/90",
"unknown"). So constraint authors have a concrete rule to honour:

> DB constraints reject only the **physically or type-impossible**; everything improbable-but-possible is
> accepted and, at most, **advisorily flagged** — never blocked. Flagging ≠ rejecting.

The solo-GP backlash in the comments ("software that will prevent us from doing certain things") is the
political case *for* this line: quality monitoring must be advisory-to-a-human (principle 9), never a hard
veto.

**Two spec-sentence freebies.** A reader who records *"married 2006, child 2008"* rather than *"married
10y, child 8y"* has independently reinvented `t_effective` anchoring (record the durable fact, derive the
volatile one). And the *record-fragmentation* complaint — "I want all entries contiguous, as on paper" —
is a direct field endorsement of the legibility-twin narrative view (principle 11 / paper-parity).

## 5. Outcome

- **No new architecture.** The primitives absorbed all 16; the append-only + bitemporal + attributed event
  core dissolves the entire class of failure the column documents.
- **Follow-ups**: **flag ①** checked against code and filed as
  [issue #163](https://github.com/cairn-ehr/cairn-ehr/issues/163) (envelope already records re-affirmation;
  the projection layer collapses the two timestamps — design questions tracked there). **flag ②** — a
  decision on whether open-loop/obligation needs naming, plus the surface-without-a-modal design — still
  open. **flag ③** captured here as a constraint-authoring rule for the in-DB floor.
- **Reusable illustrations** for the spec/essays: instalment 3 (principle-4 exemplar — absence has four
  positive causes); instalment 13 (a clinician stating principles 1 + 10 unprompted); the column-wide
  prompt-fatigue backlash (field evidence for the confirmation-dialog prohibition).
