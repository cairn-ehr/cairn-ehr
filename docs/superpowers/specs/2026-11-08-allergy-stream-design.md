# GUI milestone — the allergy list (design)

- **Date:** 2026-11-08
- **Status:** brainstormed with the maintainer 2026-11-08; builder-checked; awaiting approval
- **Milestone step:** "record an allergy" (HANDOVER ⇒ NEXT, the 2026-10-31 GUI milestone)
- **Builds on:** [data-model §3.3](../../spec/data-model.md#33-mutable-non-demographic-state) (allergies: *union,
  never auto-delete; removal requires an explicit reconciliation event*), [ADR-0052](../../spec/decisions/0052-born-sealed-clinical-bodies.md)
  (born-sealed bodies), [ADR-0053](../../spec/decisions/0053-per-write-human-authorship.md) (per-write human
  authorship), [ADR-0059](../../spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md) (drugref
  coding, honest degrade), [ADR-0063](../../spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md)
  (the clear safety signal), [ADR-0076](../../spec/decisions/0076-duplicate-repair-a-linked-chart-reads-as-one-and-a-human-judgement-outranks-a-machine.md)
  (a linked record reads as one).

Text marked **[maintainer]** was decided by the maintainer in the brainstorm. Text marked **[my call]** is the
designer's, open to correction. Unmarked text restates a standing project rule.

## 1. Why

**[maintainer]** The allergy failure with the most clinical consequence is not a missing allergy. It is a
**false or inflated** one: a minor adverse effect (loose bowel motions on metformin) recorded as "allergy", which
then stops an inexperienced junior doctor from prescribing a drug the patient needs and would very likely
benefit from. So the list must make two things legible at 3 a.m.: what is dangerous, and what is *not*.

Today Cairn has no allergy stream at all. The only record of an adverse reaction is a medication's free-text
cessation reason (db/060; `cairn-medication-view` row.rs says so), shown only on hover (#871).

## 2. Goals, non-goals, anti-goals

**Goals**
1. Any clinician — nurses included — records a drug or non-drug allergy, or an adverse effect, in one short
   form and one Sign, with no confirmation by anyone else **[maintainer]**.
2. Each entry's wording tells the reader its kind, its source, and what happened, in the words a clinician
   would write on paper (§3.3).
3. "No known allergies" and "NKDA" are recorded statements, distinct from "not yet asked" **[maintainer]**.
4. Entries are corrected, de-labelled or withdrawn by overlay, never edited or deleted.
5. Every current medication is checked against the drug allergies, and the check says plainly when it could
   not be made **[maintainer: "should warn"]**.

**Non-goals (this design)**
- The warning *at the moment of charting* a medication: the window cannot chart a medication yet; that work
  comes next **[maintainer: allergies first, then charting]** and reuses §3.7's matcher.
- Drug-allergy groups and cross-reactivity knowledge in Cairn. **[maintainer]** drugref owns them: classes and
  drugs linked to MeSH terms, hand-curated lists seeded from open sources. Cairn keeps no list of its own.
- Non-drug allergens feeding any decision support (food ↔ vaccine excipients and the like): later, from
  drugref, on the same record shape.
- The essential replicated tier (ADR-0016) — not built anywhere yet; allergy events sync like every clinical
  event.

**Anti-goals — a failure even if it "works"**
- A default of *Allergy* on the kind choice (it recreates the metformin problem) **[maintainer, via the
  mockup]**.
- An empty list that reads as "none".
- Silence that reads as "checked, safe" when the check could not be made.
- A confirmation dialog anywhere (principle 3).
- A reason that is visible only on hover (#871's lesson).

## 3. The design, as decided

### 3.1 One record shape, two lists

**[maintainer]** One entry shape for every allergen, with a field saying whether it is a **drug** or **other**
(latex, peanut, bee sting, Elastoplast). Drug entries feed the medication check; other entries never do. They
are shown as two lists. Contrast media is a drug.

### 3.2 An entry's fields (the form)

**[maintainer: approved in a mockup, 2026-11-08]** One short form under the
header strip, five fields, then **Sign** with the held key:

| Field | Values | Default |
|---|---|---|
| **Substance** | a drugref drug, a drugref drug group, or the text as typed (uncoded) | — |
| **What kind** | *Allergy* · *Adverse effect* | **none — must be chosen** |
| **What happened** | short list: *anaphylaxis · angioedema · urticaria · rash · GI upset · other · not known*, plus optional free text **[maintainer]** | — |
| **When** | a full date, a month and year, a year, or *not known* | *not known* |
| **Source** | *patient stated* · *seen / documented* · *another record* | *patient stated* |

- "Not known" is a recorded answer for *what happened* and *when* (principle 4), never a blank.
- An intolerance is an *adverse effect* **[my call, unchallenged]**: two kinds, not three.
- Substance coding reuses ADR-0059's shape — `{system, code, display}` beside a mandatory `term` (the text the
  clinician saw) — with the coding optional, so "that blue antibiotic" is recordable and coded later by overlay.
  A drug group needs a coding system of its own beside the moiety one **[my call]**.
- Without drugref the substance field is free text only, and everything else still works.

### 3.3 How entries read (the clinician's words)

**[maintainer]** wording, from the brainstorm:

| Entry | Reads |
|---|---|
| allergy, seen/documented, group + agent, anaphylaxis | **Allergy: penicillin (amoxicillin) → anaphylaxis** · 2014 |
| allergy, patient stated, reaction not known | **Patient stated allergy: penicillin. Effect not known.** |
| adverse effect, with a year | Previous adverse effect (2019): metformin → loose bowel motions |
| de-labelled (§3.5) | *Historically reported: penicillin allergy. Challenged: tolerated amoxycillin 03/2022* |

**[my call]** The free text follows the picked reaction ("→ rash: itchy, trunk only"). "Seen / documented"
carries no prefix; "another record" reads "Allergy (from another record): …".

### 3.4 The header strip and "none known"

**[maintainer: option A of the placement mockup]** The list lives in the **patient identity header**, as one
terse, always-visible line — the paper folder's red sticker. Screen space is at a premium; A is terse but safe.

- The line shows the allergies, drug first, then "Other: …", then a quiet count of adverse effects and historical
  entries; **show all** expands the full list and the form in place (no dialog).
- Allergies are visually prominent; adverse effects are not; historical entries are muted **[my call]**.

**"None known" is a statement, not an absence [maintainer]**, and the clinician-facing words are derived from
the one shape:

| Recorded | Shown |
|---|---|
| "asked about all allergies: none" | **No known allergies** (asked 08/11/2026) — NKDA implied |
| "asked about drug allergies: none" and nothing on others | **NKDA** (asked …) · other allergies: **not yet asked** |
| NKDA + a latex entry | **NKDA** · Other: latex → urticaria |
| nothing | **Allergies not yet asked** — never blank |

- **[maintainer]** Any recorded allergy simply overrides "no known allergies" / NKDA in its list — no flag. The
  older statement stays readable in the expanded list ("previously recorded: no known allergies, date").
- **[my call]** The two statements are two buttons on the strip when its list is empty (and in the expanded
  list), each one Sign with the held key.

### 3.5 Changing an entry later — overlay only

**[maintainer: de-labelling wording; the rest my call, unchallenged]** Three acts on an existing line, each a
new signed event referring to the entry; nothing is edited or deleted:

1. **Correct** (a better reaction, a date, coding it): the line shows the newer account; the older is in its
   history.
2. **De-label — "challenged: tolerated <drug> <date>"** **[maintainer]**: the line stays shown as *Historically
   reported: … Challenged: …*, and is **not relevant to warnings or decision support any more**.
3. **Entered in error** (wrong patient, slip): leaves the strip and every check; stays in the history, like a
   struck-through line on paper.

A "none known" statement is also withdrawable as entered in error.

### 3.6 Differing accounts

**[maintainer]** Accounts that disagree about the **same allergen** are flagged: e.g. *penicillin → rash* and
*penicillin → anaphylaxis*. **[my call]** Both stay shown, the more severe first, marked *differing accounts*;
warnings always use the more severe. An *allergy* and an *adverse effect* on the same allergen also differ. A
reaction of *not known* never differs from anything. The flag asks for a closer look; it never says which is
wrong, and nothing is auto-resolved. Two entries on different allergens never differ.

### 3.7 Checking the medication list

**[maintainer: warn; drugref provides the groups]** **[my call: the rules below]**. A pure matcher takes the live
drug-allergy entries and the current medication lines and returns, per line, zero or more marks:

| Case | Result on the medication line |
|---|---|
| same substance (same code; or, uncoded, same normalised term) | **mark** — "Allergy recorded: amoxicillin → anaphylaxis" |
| allergen is a group, drugref says the drug is a member | **mark** naming the group — "amoxicillin is a penicillin; allergy recorded: anaphylaxis" |
| entry is an adverse effect on that substance | a **quiet note**, never a warning mark |
| entry is historical (de-labelled) or entered in error | nothing |

The mark is **on the line, always visible** — never hover-only (#871).

**Honest degrade.** Membership drugref *has* is a real hit; membership it *lacks* proves nothing (measured
2026-11-08 in `drugref_profiles`: MED-RT's *Penicillin-class Antibacterial* lists *amoxicillin anhydrous* but not
*amoxicillin*, and no form of ampicillin, flucloxacillin or cloxacillin). So:

- For each **group** allergen whose membership is not declared complete by drugref (or when drugref is absent,
  or the allergen is uncoded), the strip carries **one line per allergen**: *"Penicillin group: medications not
  fully checked against it"*. One line per allergen, not one per medication, so ten current drugs do not make
  ten marks.
- An uncoded medication line cannot be group-checked; it says so on that line.
- Drugref must therefore expose a **completeness statement per group** (its curated lists will). Until it does,
  every group reads "not fully checked" while real hits still mark.
- **[maintainer]** Build this **now on drugref's current MED-RT classes**, not waiting for the curated lists.
  The slice writes down the narrow read-only contract Cairn needs (§4) so the drugref side can implement it;
  until drugref implements it, Cairn's read door adapts the existing tables (`class_membership`,
  `substance_class`, `class_subtree`) and reports every group as not declared complete.

### 3.8 Signing and authorship

Each act is one signed, sealed event under the held key, through the one `EventSigner` seam (ADR-0053,
ADR-0083), like a medication act. **[my call]** No drafts and no quarantine: the form is too short to be worth
one. **[maintainer]** No co-sign: a student's or supervisee's entry counts and reads exactly as anyone else's,
with no "— not co-signed" marker (ADR-0094's note rule does not extend to allergies). The signed byline shows on expansion with the unconfirmed marker rules slice 3 built.

### 3.9 A linked record

A linked record reads as one (ADR-0076): the strip shows the union of every member chart's entries, each naming
its chart in the expanded list as medication rows do. "None known" on one member and an allergy on another
resolves by §3.4's override; §3.6 compares across members.

## 4. Constraints and facts for the builder

- **Born-sealed.** Every body is sealed under a per-event DEK (ADR-0052), with the twin, floor at both doors,
  shred hook (db/037), timing kind (db/044) and projection the note and medication streams carry. Follow
  `db/064` + `crates/cairn-event/src/note.rs` as the most recent full example of a stream, and
  `crates/cairn-node/src/medication/sealed_submit.rs::seal_sign_submit` for the sealed submit path.
- **Every read in this design is a custody read.** The strip, the differing-accounts flag and the medication
  check all read the decrypted bodies, with overlays applied, through the stream's own projection. None of them
  reads the clear safety signal.
- **No clear safety signal in this design [my call, after the builder check].** db/049's signal has no
  supersession (#406: "a ceased drug warns forever"; #401: a correction emits no signal; ADR-0063 leaves thread
  rollup open). So an allergy signal emitted today would keep warning after *entered in error* and *challenged:
  tolerated* — the metformin problem, made permanent on the wire. ADR-0063 decision 5 also puts class and severity
  in `safety_class_map`, which ships empty on purpose (principle 9), and decision 7 says an uncoded event emits
  nothing. Allergy events therefore emit no `safety` field until #406/#401 settle supersession for medications
  and allergies alike; the stream ADR states this, and the gap is §7 risk 1. Not emitting is additive-safe: a
  later event type version may emit.
- **Reaction severity order** for §3.6's "more severe first" and §3.7's "the more severe": anaphylaxis >
  angioedema > urticaria > rash > other > GI upset; *not known* sorts with anaphylaxis for warning purposes (assume
  the worst, db/049's rule) but never triggers §3.6. Clinical display order, not policy **[my call]**.
- **Union merge** (data-model §3.3): sync never removes an entry; only §3.5's events change what is shown.
- **Drugref is optional and co-resident** (data-model §3.16, ADR-0059): it lives in the deployment's Postgres or
  is absent, and never rides the wire. Cairn reads it through a **narrow contract it names** (substance search
  by name; group membership; per-group completeness), behind a Cairn read door, so the window's `cairn_window`
  login gains no drugref grants and a drugref schema change touches one place. Tests use a stub schema that
  implements the contract; the local `drugref_profiles` database (:5532) is for a manual run.
- **What drugref has today** (2026-11-08): 19 438 moieties; MED-RT/MeSH class memberships (`class_membership`,
  `substance_class`, `class_subtree`); no allergy groups, no completeness statement, no HTTP API (its slice 6).
- **Wire shape is forever.** Event types and body fields are additive-only once merged (principle 11); settle
  the overlay shape (one event type with a status, or one per act) in the plan, not mid-build.
- **Pinned-count guards** move for a new event type: `twin_registry.rs`, `db/tests/034`, both
  `cairn_projection_apply` counts, and the per-file guards the memory list names — see
  `developers/durable-rules.md`.
- **Restore** must carry the new projection (the clinical-plane restore tests name the pattern).
- The window is plain JS + Tauri (no npm); `funnel.js` sits at 499 lines (#1023) — the strip is its own file.

## 5. Build order — *my call*, one PR each

1. **The stream and the strip.** Entry + "none known" events (db/074), floor, twin, projection, shred (no clear
   safety signal, §4); `cairn_node::allergy` (sign; read over a linked set); a pure display crate (§3.3 wording, §3.4
   states); the header strip, the form (free-text substance), the two "none known" buttons.
2. **Overlays and differing accounts.** Correct, de-label, entered in error (§3.5); §3.6's flag.
3. **drugref and the medication check.** The contract + read door + stub; drugref pick in the form; §3.7's
   matcher and its marks on the medication list; the per-allergen "not fully checked" line.

## 6. Paper-parity benchmark (§1.2) — for each plan to carry

- **Paper counterpart:** writing "Penicillin — anaphylaxis 2014" in the drug chart's allergy box (and the
  sticker); "NKDA" written in the box; striking a line and writing "tolerated amoxil 03/22".
- **Steps:** record an entry — paper N = 1 (write the line) → architecture M = 1 (one signed event) → UI target
  K = 1 form + Sign. "None known": N = 1 → M = 1 → K = 1 press. De-label: N = 1 → M = 1 → K = 1 action + Sign.
  Any `M > N` found while planning is an architecture defect to file.
- **Time + cognitive load:** budget ≤ 15 s for a coded entry, ≤ 10 s for free text, ≤ 3 s for "none known"
  (paper ≈ 5–10 s); reading the strip needs no click to learn whether a dangerous allergy exists. Measured by
  the slice that first exposes the window (slice 1).

## 7. Risks named

1. **A reader without custody sees no allergies.** With no clear signal (§4), a node or role that cannot
   decrypt an allergy body learns nothing from it — the gap ADR-0006's safety projection exists to close. Today
   that reader is rare (custody is normally held); it stays open until #406/#401 give the signal supersession,
   and the allergy ADR says so. The tempting fix — emit the signal now — is the worse error (§4).
2. **Group checks look complete when they are not.** Mitigated by §3.7's per-allergen line; the risk returns if
   anyone "simplifies" it to a per-line mark that only appears on hits.
3. **Over-warning noise.** Adverse effects must stay quiet notes, or juniors learn to ignore the mark.
4. **Concurrent edits across a partition** produce two corrections of one entry: both stay; the newer by HLC
   reads as current, and §3.6 flags them if they differ.

## 8. What is left to the builder

The overlay event shape; the coding system name for a drug group; the strip's exact styling; the matcher's term
normalisation; how the strip collapses on a narrow window; the order of entries within a list.

## 9. ADR owed

One ADR for the allergy stream (the shape, the none-known statements, the overlays, the safety severity map,
the drugref contract). The milestone's fourth decision — *"prescribe" means charted* — is still in no ADR and
can ride the charting slice's ADR.
