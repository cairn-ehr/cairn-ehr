# GUI milestone — charting a medication (design)

- **Date:** 2026-11-10
- **Status:** brainstormed with the maintainer 2026-11-10; builder-checked 2026-11-10; maintainer's answers to the builder check pending (§10)
- **Milestone step:** "chart a medication" (HANDOVER ⇒ NEXT, the 2026-10-31 GUI milestone). The milestone's fourth
  decision, *"prescribe" means CHARTED* (no printed or electronic prescription), is in no ADR yet. This slice's
  ADR records it.
- **Builds on:** [data-model §3.3](../../spec/data-model.md#33-mutable-non-demographic-state) (the medication
  stream; bitemporal time §3.6), [ADR-0052](../../spec/decisions/0052-born-sealed-clinical-bodies.md) (born-sealed),
  [ADR-0053](../../spec/decisions/0053-per-write-human-authorship.md) (per-write authorship),
  [ADR-0059](../../spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md) (drugref moiety coding,
  honest degrade), [ADR-0096](../../spec/decisions/0096-the-allergy-list.md) /
  [ADR-0097](../../spec/decisions/0097-allergy-overlays-and-differing-accounts.md) /
  [ADR-0098](../../spec/decisions/0098-allergy-coding-and-the-medication-check.md) (the allergy list, its overlays,
  and the check this design reuses at charting), [ADR-0021](../../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md)
  (hard policy in the database, soft policy in the UI).
- **Closes, when built:** #1054 (check at charting), #1053 (code the tolerated drug on a de-label). It depends on
  #1065 + drugref#231 for names.

Text marked **[maintainer]** was decided by the maintainer in the brainstorm. Text marked **[my call]** is the
designer's, open to correction. Unmarked text restates a standing project rule or a fact about the code.

## 1. Why

The window can show, hold, sign off and stop a medication, but it **cannot add one** (`cairn-gui-tauri`
`commands.rs` has `med_list`, `sign_off`, `cease`, holds; no assert). The node can
(`cairn_node::medication::assert_medication`). This slice gives the clinician the act.

**[maintainer]** The allergy check matters most **at the moment of prescribing a new medication**. When a drug the
patient *already takes* hits a recorded allergy, the record is usually what's wrong. The clinician asks whether
the patient takes it and has had problems. If they tolerate it, the clinician lifts the allergy with a note
("on drug X without adverse effects as of <date>"), and the next chart no longer alerts. Today Cairn checks
only on reading the list (ADR-0098), and nothing at charting offers that repair.

## 2. Goals, non-goals, anti-goals

**Goals**
1. A clinician charts a medication from **one typed line**, e.g. `amox 500mg tds` → *amoxicillin · capsule
   500 mg · 500 mg · three times daily* **[maintainer]**.
2. Every charted line says whether it was **started here** or was **already being taken** **[maintainer: it must
   be distinguished]**, at no extra step (§3.2).
3. The allergy check runs **at charting**, before Sign, on every component of the drug (§3.4).
4. A **stat** dose can be ordered, and its **giving** recorded as a separate signed act with its own time
   **[maintainer]**.
5. Generic names always shown; a brand or local name the clinician typed is kept in brackets after it
   **[maintainer]**.

**Non-goals** (**[my call]**, maintainer saw them and added nothing)
- No dose-by-dose administration record (MAR) for regular medications. Only a stat dose's giving is recorded.
- No printed or electronic prescription.
- No infusions or rates. No PRN maximum beyond what is typed.
- No dose-range or interaction checking. The check is allergy only (ADR-0098's).

**Anti-goals** (what would make this a failure even if it works)
- **A prefilled dose** **[maintainer]**: same failure as click fatigue on popup modals. The dose is always typed.
- **Having to look up formulations** **[maintainer]**: if the drug's forms and strengths exist, they are offered
  in the pick, never searched for.
- **A confirmation dialog** as the safety mechanism (paper-parity law). Warnings sit on the line.
- **A silent reinterpretation** of what was typed: a parse or an alias that changes the drug or dose without
  showing it.
- **A tolerated related drug striking a true allergy**: an allergy de-labelled because a *different* drug that
  shares its group was tolerated (§3.5, §10 Q1).

## 3. The design, as decided

### 3.1 The one line

- **[maintainer]** One text field. After 4–5 typed letters a list drops down, **most commonly used choice on top**.
  Each choice is a drug *with* its form and strength where those are known (capsule 500 mg, oral liquid
  250 mg/5 mL, tablet 1 g), and **combinations sit beside the single drug** (amoxicillin + clavulanic acid under
  "amox"). The maintainer chose this one-line form (mockup A) over a field per part (mockup B).
- **[maintainer]** The clinician then types dose and frequency, e.g. `500mg tds`. **Never prefilled.** Ideally the
  whole line, `amox 500mg tds`, is typed at once and completes to *amoxicillin · tablet/capsule · 500 mg · three
  times daily* in the right columns.
- **The typed text is what is signed [my call].** The parse into Drug / Form / Dose / Route / How often is
  shown on the line before Sign, as a reading of the text. The record keeps the typed directions (`sig`). The
  parsed `dose_amount`, `dose_unit`, route and frequency are filled where the parse is certain. A line that
  cannot be read stays as typed. It is never blocked, and the unread parts stay *unknown* (principle 4).
- **The parse vocabulary [my call]:** the abbreviations the Australian Commission on Safety and Quality in Health
  Care's *Recommendations for terminology, abbreviations and symbols* accepts (bd, tds, qid, mane, nocte, prn,
  stat, and routes po, IV, IM, SC, SL, PR, top, inh, neb …), numbers with units (mg, g, mcg, mL, units). It
  never guesses outside that list. A pure, table-driven, unit-tested function.
- **"Most commonly used" means this site's own charting history [my call].** A count of coded charted lines by
  (drug code, form/strength), read on the node. With no history, shorter names first (db/076's rule today).
- **Enter on the top choice is the fast path, so the top choice must be visible as the choice [my call].** The
  pick shows the full name of the highlighted choice (e.g. "amoxicillin + clavulanic acid" vs "amoxicillin"),
  and the parsed line shows the chosen drug before Sign. A single-drug name and its combination are one row
  apart; that is the reflex-pick risk (§7).

### 3.2 Started here vs already taking

- **[maintainer]** Paper does not distinguish these (a few digital ED charts flag it; discharge-summary software
  does). Cairn must.
- **[my call, maintainer did not object]** The medication pane offers **two entries instead of one Add: "Start
  medication" and "Already taking."** The clinician is always doing one or the other, so the press is the
  distinction, zero extra steps. The record gains an additive field with three values, `started-here`,
  `pre-existing` and `unknown` (an old list nobody can vouch for, principle 4). The press sets visible,
  changeable defaults. *Start*: started today, the information source is the clinician. *Already taking*: start
  date unknown, the patient as the source (`info_source` exists already: patient-reported | clinician-observed |
  external-record | unknown).

### 3.3 Names: generics shown, aliases funnel

- **[maintainer]** An **alias table lives in drugref**, not in Cairn. It is a very common problem. It funnels
  commonly used names to drugref's reference name: spellings (amoxycillin → amoxicillin, cephalexin ↔ cefalexin)
  **and brand names**, which Australians often use for combinations (Augmentin → amoxicillin + clavulanic acid,
  Bactrim → sulfamethoxazole + trimethoprim). Filed as drugref#231 (Cairn side #1065).
- **[maintainer]** The record **displays the correct generic name**. If the clinician typed something else, it
  goes **in brackets after it**: "amoxicillin + clavulanic acid (Augmentin)". The typed `term` is kept on the
  record as today; the display is generic-first.
- **A name that could mean two drugs shows both and never picks one [my call]** (drugref #227's own rule for a
  name two moieties claim). A wrong alias is silently a wrong drug, and the allergy check then checks the wrong
  thing.
- **Until drugref ships aliases** the search uses drugref's display names (db/076's `cairn_drug_search`). The alias
  search turns on by availability, as db/076's other doors do. With no alias table, the pick still works.
- **The medication pick is drugs only [my call, builder check]:** `cairn_drug_search` lists allergy groups
  first, and a group cannot be charted, so the medication line needs a drugs-only variant of the door.
- **Form and strength choices** come from drugref's PBS local tier (`drugref.local_product`, `form_strength`,
  `local_product_moiety`) **when a site has ingested PBS** (the operator ingests it; drugref may not redistribute
  it; on :5532 it is empty). Without it, the pick offers the drug only, and form/strength come from the typed line.

### 3.4 Combinations and route: two additive record fields

- **[my call, maintainer agreed]** A medication's coding today is **one** `SubstanceCoding` (ADR-0059). A
  combination (Augmentin) must be coded as **its list of components**, each a drugref moiety. The allergy check
  runs on every component, so Augmentin hits a penicillin allergy through its amoxicillin. Component-level, not
  a PBS product code, so it works at a site with no PBS data. Additive: an old event's single coding still
  reads.
- **[my call, maintainer: "good catch"]** The record has form, dose and directions but **no route**. ED needs it
  (ceftriaxone 1 g IV vs IM). Add `route` as an optional field the line fills from IV / IM / po / …; absent =
  unknown.
- **The component list reaches everywhere a single coding lives today [my call, builder check]:** the
  assertion, `clinical.medication.coded` and its correction (db/042, `CodingClaim`), and the ADR-0063 `safety`
  claim (one precise claim per component). Otherwise coding a line later, or a peer's signal, stays
  single-component.
- **Strength is a new optional field [my call, builder check]**, e.g. `500 mg` or `250 mg/5 mL`, filled from the
  pick or the line. No strength field exists today (`formulation` is a form token). Without it a liquid's dose
  volume cannot be read, and "most used by form/strength" has nothing to count.
- All of these are wire changes, forever (principle 11): the plan settles the exact shape before any code.

### 3.5 The allergy check at charting

The check is ADR-0098's (`cairn_drug_facts`, drugref owns the fold), run on the line being charted before Sign
(#1054), per component. Its result decides what the line offers:

| The press | A hit on… | What the line does |
|---|---|---|
| **Start medication** | an **allergy** (same drug or same group) | Loud warning on the line. A **Reason** field appears, and **Sign stays unavailable until a reason is typed** **[maintainer: medicolegally indefensible if omitted]**. Free text, no pick list **[my call]**. |
| **Start medication** | an adverse effect only | The muted ADR-0098 note. No reason asked. |
| **Already taking** | the **same drug** | Offers the **de-label** in the same act: "tolerates {drug} as of {today}", drug and date filled, signed with the line **[maintainer]**. The tolerated drug is coded (#1053, opens db/075's reserved `tolerated.coding` slot). |
| **Already taking** | a **group** only (a different drug sharing the allergen's drugref allergy group) | **Never strikes the allergy [maintainer].** At most offers a **comment on the allergy line**: "tolerated {drug} well on {date}". The allergy and its warning stay. |
| either | not checked (uncoded line, drugref absent) | The small "not checked" tag. No reason asked. |

- **The offer keys on the check's `match_kind` = same moiety, and nothing else [my call, builder check].** A hit
  with an empty or unknown kind, or a hit by term match (ADR-0098 decision 19, ruling T4-a words it "same drug"
  for the warning's sake), offers the comment only, never the de-label. A safe bias for a warning is an unsafe
  one for a de-label.
- **A de-label lifts every entry the check joined into that allergen [my call, builder check]** (ADR-0098
  decision 20 builds one allergen from several connected heads). Otherwise the next chart still alerts from the
  head left behind. One challenge event per head, in the same transaction. For a combination, `tolerated.coding`
  records the component that matched.
- **Charting and a taken de-label or comment are one transaction [my call, builder check]**: both go in or
  neither does, and the window says which.
- **Cephalosporin vs penicillin reads `no-match` today** (drugref has no such relation; #1055). So a cephalosporin
  charted against a penicillin allergy shows nothing. This design does not fix that; the risk is §7 item 6.
- **The reason is signed into the medication event**, with which allergy it was charted against **[my call]**.
- **The reason is a window rule, not a floor rule [my call]** (ADR-0021: soft policy in the UI). A peer or
  older node's event without one still syncs and reads.
- **The de-label and comment offers are optional.** Charting without taking them is one act, as today.
- **The group comment is a new allergy act**: it adds text to the line and never covers, strikes or weakens it.
  ADR-0097's overlays (correct, de-label, entered in error) have no such act. Its shape is the builder's (§8).

### 3.6 Stat doses: ordered, then given

- **[maintainer]** In this slice. `ceftriaxone 1g IV stat` is charted on the same line; *stat* reads as a
  one-off. The **order** is one signed event. The **giving** is a separate signed act by whoever gave it (usually
  a nurse, signing per act) and carries the time given.
- **[maintainer]** The list shows the time given, and **the audit trail must keep the acts in time order**:
  "ceftriaxone 1 g IV stat · ordered 12:34 Dr A · given 13:22 RN B". Until given: "· **not yet given**",
  visible until it is given or stopped (the existing Stop).
- Cairn has **no administration record today** (ADR-0081: no MAR). "Given" is the first administration event, a
  new event type with its own floor, twin, projection and shred hook, on the medication thread.
- **A given stat dose leaves the current list [my call, builder check].** Once given it is complete: it shows
  under the visit's given doses with its times, and no longer feeds the current-list check. An ungiven one stays
  current and visibly "not yet given".
- **Time:** the HLC (`t_recorded`) orders the two acts objectively. The time given is the asserted `t_effective`,
  stated by the giver (default now, editable to earlier).

## 4. Constraints and facts for the builder

- **Born-sealed** (ADR-0052) for every new event body. Follow `crates/cairn-node/src/medication/sealed_submit.rs`
  and the medication stream's floor, twin, projection, shred hook (db/037) and timing kind (db/044).
- **Wire shape is forever.** The started-here field, the component list, route, the override reason and the
  administration event are additive and settled in the plan.
- **The check reads custody bodies**, never the clear safety signal (ADR-0096 §4; #406/#401).
- **drugref is optional and co-resident**, read only through `db/076`'s definer doors. New drugref objects
  (aliases, PBS products) get doors there with availability checks, and `cairn_window` gains no drugref grant.
- **Pinned-count guards** move for a new event type (twin registry, db/tests/034, both `cairn_projection_apply`
  counts). See `developers/durable-rules.md` and the allergy blocks there.
- **Restore** must carry any new projection.
- The window is plain JS + Tauri (no npm). New code goes in its own files; `funnel.js` is at 499 lines (#1023).

## 5. Build order — *my call*, one PR each

1. **The line and the check at charting.** Start / Already taking; the one-line parse; route and components
   (wire); the pick over display names; ADR-0098's check on the line; the override reason; the same-drug de-label
   offer (#1053) and the group comment.
2. **Stat: ordered, then given.** The administration event; the giver's act in the window; the list's ordered /
   given / not-yet-given words.
3. **Aliases and PBS forms/strengths**, on as drugref supplies them (drugref#231; PBS ingest per site); the
   "(as typed)" bracket where the alias, not the clinician, chose the generic.

## 6. Paper-parity benchmark (§1.2) — for each plan to carry

- **Paper counterpart:** writing "Amoxycillin 500 mg tds PO" and signing on the drug chart or in the notes;
  "ceftriaxone 1 g IV stat" with the nurse's initials and time given in the stat column; striking an allergy and
  writing "tolerated amoxil 03/22"; "penicillin allergy — benefit outweighs risk" written beside a new order.
- **Steps:** chart a line: paper N = 1 (write and sign) → architecture M = 1 human act (one Sign; a taken
  de-label or comment adds its own allergy-stream event(s) to the same transaction, so events ≥ 2, acts = 1) → UI target K = 1 line + Sign. Override reason: N = 1 written note → M = 0
  extra events (it is in the event) → K = 1 field. Stat given: N = 1 (initial + time) → M = 1 → K = 1 press +
  Sign. Any `M > N` found while planning is an architecture defect to file.
- **Time + cognitive load:** budget ≤ 10 s to chart a common drug from the line (paper ≈ 8–15 s); ≤ 3 s to record
  a stat as given; reading the line before Sign needs no click to see the drug, dose, route and any allergy hit.
  Measured by slice 1's human RUNBOOK pass.

## 7. Risks named

1. **Reflex pick of a neighbour** (amoxicillin vs amoxicillin + clavulanic acid; a common drug on top). Mitigated
   by the parsed line naming the chosen drug before Sign. The risk returns if anyone hides it to save space.
2. **A parse that misreads a dose** (`1.5mg` read as `15mg`). The typed text is the record; the parse is shown and
   fills a structured field only when it is certain. A unit-tested table, never a heuristic.
3. **A wrong alias** silently changes the drug. Aliases are drugref's, ambiguous names show every candidate, and
   the line shows what it is coded as.
4. **A reason typed to get past the gate** (".", "ok"). The window cannot judge a reason. It keeps it, signed, for
   review; the gate makes the omission impossible, not the quality.
5. **A group hit treated as a same-drug hit** would de-label a true allergy. The offer must key on the check's
   `match_kind` (same moiety), never on "a hit".
6. **A cephalosporin against a penicillin allergy reads clean** (#1055: drugref has no such relation, and
   `no-match` is a verified negative to ADR-0098). The charting check inherits this. It needs fixing in drugref
   before a pilot.

## 8. What is left to the builder

The exact wire shapes (the started-here field's name, the component list, route tokens, the reason's place, the
administration event type); the group comment's event shape; the parse table's full contents within §3.1's
vocabulary; how "most used" is counted and when it is read; the pick's layout; the line's narrow-window form.

## 9. ADR owed

One ADR for charting: *"prescribe" means charted*; started-here vs pre-existing; components and route; the check
at charting and the override reason as a window rule; the same-drug-only de-label offer; the group comment. A
second (or the same) for the administration event when slice 2 lands.

## 10. Open for the maintainer (from the builder check)

1. **Does an amoxicillin challenge de-label a "penicillin" allergy?** The same-drug rule (§3.5) offers the de-label
   only on the same moiety. But an allergy recorded as "penicillin" and a patient tolerating amoxicillin is the
   standard oral-challenge de-label, and under the rule as written it would get the comment only.
2. **Combinations before drugref has them.** drugref's search holds single moieties; combinations exist only in
   the PBS tier, which most sites will not have. Slice 1 either lets the line take `amox + clav` and code each
   component, or leaves Augmentin uncoded ("not checked") until drugref#231 / PBS.
3. **A dose given before it was charted** (verbal order in resus: given 12:30, charted 12:34). Accept and flag the
   gap on the line (principle 4: clashes flagged, never auto-resolved), or refuse?
