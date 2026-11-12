# GUI milestone — charting slice 1b: the same-drug de-label offer (design)

- **Date:** 2026-11-12
- **Status:** brainstormed with the maintainer 2026-11-12; builder-checked 2026-11-12 (one question, answered); approved by the maintainer 2026-11-12
- **Supersedes, for slice 1b only:** the charting design's §3.5 rows for *Already taking* and §3.5's bullets on the
  group comment and on one transaction for the line and a taken de-label
  ([`2026-11-10-charting-a-medication-design.md`](2026-11-10-charting-a-medication-design.md)). Everything else in
  §3.5 stands.
- **Builds on:** [ADR-0097](../../spec/decisions/0097-allergy-overlays-and-differing-accounts.md) (the de-label:
  a signed `clinical.allergy-challenge` overlay; the struck line stays visible with its byline and date) ·
  [ADR-0098](../../spec/decisions/0098-allergy-coding-and-the-medication-check.md) (the check) ·
  [ADR-0099](../../spec/decisions/0099-charting-a-medication.md) (the line, the check at charting) ·
  [ADR-0100](../../spec/decisions/0100-charting-a-list-under-one-signature.md) (the list and its one Sign).
- **Closes, when built:** #1054 (the check at charting: its last half), #1053 (code the tolerated drug).

Text marked **[maintainer]** was decided by the maintainer. Text marked **[my call]** is the designer's, open to
correction. Unmarked text restates a standing project rule or a fact about the code.

## 1. Why

**The real case [maintainer, 2026-11-12]:** a patient carries an allergy label; on taking the medication history
the clinician asks, confirms the patient tolerates the drug, and strikes the allergy with a note that the drug was
tolerated on a date — **early**, while still writing, because it changes every prescribing decision after it.

Two rulings shape the slice:
- **[maintainer]** Lifting an allergy is always the clinician's **deliberate act**, never offered for a related
  drug and never done automatically. The struck allergy stays visible as historical fact, with the act of striking
  it, its responsible clinician and its date (ADR-0097's de-label already does this).
- **[maintainer]** **A drug on the medication list does not attest that it was tolerated.** Charting a drug never
  writes anything to the allergy list.

Paper counterpart: strike the allergy line, write "tolerated amoxil 12/11/26" beside it, initial it.

## 2. Goals, non-goals, anti-goals

**Goals**
1. An unsigned list line whose drug is **exactly** the drug of a recorded allergy offers that allergy's de-label, as one
   deliberate press and its own signature, at once.
2. A de-label records the tolerated drug **coded** when it was picked (#1053), from the allergy list's de-label
   form as well as from the offer.

**Non-goals**
- **No offer for a related drug.** A hit through the allergy group (amoxicillin against "penicillin"), by name or by
  any match other than the identical code, offers nothing. Lifting such an allergy is the clinician's act on the
  allergy list **[maintainer, 2026-11-10 §10 Q1; reaffirmed 2026-11-12]**.
- **No group comment from the medication line [maintainer]**. A free-text comment on an allergy line, added on the
  allergy list when the clinician chooses, is a new allergy act with its own design: #1084, not built.
- **No flag yet** when a later allergy's code meets a coded tolerated drug (#1053's "what it is FOR"): #1085
  **[my call]**. This slice only records the code.

**Anti-goals**
- A de-label pre-ticked, pre-pressed, or riding along with the list's Sign.
- Any allergy record changed because a medication was charted.
- A de-label offered on a match the check made by name, by group or by drugref's fold.

## 3. The design, as decided

### 3.1 When the offer appears

- **[maintainer]** On an **unsigned line of the clinician's own list** (ADR-0100; either press: *Start medication*
  or *Already taking*, **[my call, maintainer agreed]**) whose drug is the **same drug** as a live allergy entry.
- **[maintainer, builder check]** **Never on a line already charted.** A current medication that hits an allergy
  keeps its warning, and acting on the allergy is up to the clinician, on the allergy list.
- **[my call]** "Same drug" is decided by **code equality only**: the line's part is coded `drugref-moiety` X and
  the allergy entry is coded `drugref-moiety` X. No drugref call, no name comparison, no group. ADR-0098's check
  words a name or drug-list match "same drug" for the warning's sake (decision 19); that bias is safe for a warning
  and unsafe for a de-label (charting design §7 item 5). The rule is a pure function over the two codings.
- **[my call]** Allergy entries only, never an adverse-effect entry and never a "none known" statement. Only a
  live (not struck, not withdrawn) head. For a combination, each coded part is compared; the offer names the part
  that matched.
- **[my call, builder check]** No offer for an allergy entry on a chart whose link is in doubt: the de-label door
  refuses such a target (`ON_A_DOUBTED_CHART`), so the control could only ever be refused. Slice 1a's lone-line
  editor (`med_chart`, unused by the window since ADR-0100) gets no offer.
- The offer is a visible control on the line, beside its warning: *De-label: tolerates {drug}*. It is never
  pressed for the clinician.

### 3.2 What pressing it does

- **[maintainer]** It strikes the allergy **straight away**, as its own act under its **own signature** (the
  signer strip; a held key costs no passphrase), before and independent of the list's Sign.
- **[my call]** Pressing opens the allergy list's existing **De-label** row for that allergy record, pre-filled:
  tolerated = the line's drug (its term and its coding), date = today, note empty. The clinician can change any of
  them, then signs there. So there is one de-label act and one form, whichever way it was reached; the offer is a
  shortcut to it, nothing else.
- **[my call]** One allergy record per de-label, as ADR-0097's act is today. When several live entries carry the
  same moiety code, the line offers one control per entry, each naming its record (its words and byline), so the
  clinician strikes each deliberately. A struck entry no longer warns; any other still does (over-warning is the
  safe error).
- After the de-label signs, the allergy list and the medication list are read again: every list line is re-checked
  against the corrected allergies, and ADR-0100's rules apply unchanged (a tick given against a warning that is gone
  still holds; a new warning still refuses it).

### 3.3 The tolerated drug, coded (#1053)

- The de-label row gains the drugs-only pick (`cairn_drug_medication_search`, slice 1a's), optional. Typed text with
  no pick stays uncoded, as today (picking must stay optional, M ≤ N).
- **Wire:** none new. `clinical.allergy-challenge/1` already reserves `tolerated.coding` (`{system, code, display}`
  or null). db/075's authoring door, which refuses a non-null coding today (#1053), admits a coding of system
  `drugref-moiety` whose shape passes `cairn_check_coding_object`; the remote door is unchanged. The projection
  already stores `tolerated_coding`.
- **[my call, builder check]** The window checks the picked coding in Rust before the key, as the allergy entry's
  `coding_before_the_key` does (system `drugref-moiety`, the code re-spelled canonically); the door checks shape only
  and never looks the drug up, as at the medication assertion's door.
- The twin and the struck line's words keep naming the **term** as tolerated.

## 4. Constraints and facts for the builder

- The de-label exists: `cairn-gui-tauri`'s `allergy_overlays.rs::allergy_challenge` (one `target`, `tolerated`,
  `date`, `text`, `credential`), db/075's challenge floor and projection. The offer reuses it; it adds an optional
  coding argument, never a second door.
- The check's `LineCheck` carries `marks`, `against` (the head per mark), `notes`, `unverified`; it does not say
  how a pair matched. The offer needs the heads matched by identical moiety code: compute them in a pure function
  from the allergy read and the line's codings, never by reading `marks`' words.
- The webview's answer is never trusted (ADR-0099, ADR-0100): the offer's records are derived in Rust; the de-label
  command re-checks its target before the key, as today.
- `med_list.js` is at 497 lines, `funnel.js` at 499: new webview code goes in a new file.
- No confirmation dialog anywhere.

## 5. Paper-parity benchmark (§1.2) — for the plan to carry

- **Paper counterpart:** strike the allergy line and write "tolerated amoxil {date}" beside it, initialled.
- **Steps:** paper N = 1 act (strike + note + initial) → architecture M = 1 signed de-label event → UI K = 1 press
  (the offer) + 1 Sign on the pre-filled row; from the allergy list directly, as today plus an optional pick.
- **Time + cognitive load:** ≤ 10 s from the press to the struck line, key held. The line's drug and the allergy it
  would strike are both readable before the press, without a click.

## 6. Risks named

1. **A de-label offered on a related drug** — prevented by the code-equality rule; pinned by tests on a group hit, a
   name hit and a drug-list hit, each offering nothing.
2. **A reflex press.** The offer is one press away from a de-label. Mitigation: the row opens pre-filled and still
   needs its own Sign; the struck allergy stays visible and can be withdrawn (ADR-0097's *Withdraw de-label*).
3. **Several entries for one drug** — one control each; none is struck by another's press.

## 7. Left to the builder

The offer's exact words and layout on the line; how the medication pane brings the allergy row into view; the
pick's placement in the de-label row; test names; the slice's ADR number and wording.

## 8. ADR owed

A new ADR-0101 (ADRs are immutable, so not a decision added to ADR-0097): the same-drug-only de-label offer, by code equality; the de-label as
its own deliberate signed act, never bundled with charting; charting never writes the allergy list; the tolerated
drug coded at the authoring door. Records that the charting design's group comment and shared transaction were
dropped [maintainer, 2026-11-12].
