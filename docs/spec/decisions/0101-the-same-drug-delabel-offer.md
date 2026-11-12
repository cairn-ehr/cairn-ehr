# ADR-0101 — The same-drug de-label offer

- **Status:** Accepted (builds charting slice 1b; closes the second half of
  [#1054](https://github.com/cairn-ehr/cairn-ehr/issues/1054) and [#1053](https://github.com/cairn-ehr/cairn-ehr/issues/1053))
- **Date:** 2026-11-12 (built), recorded 2026-11-12
- **Spec version at acceptance:** 0.103
- **Issues:** PR [#1086](https://github.com/cairn-ehr/cairn-ehr/pull/1086) (the design, the plan and the build).
  Design: `docs/superpowers/specs/2026-11-12-charting-slice-1b-same-drug-delabel-offer-design.md` (approved by the
  maintainer 2026-11-12). Filed by this slice: [#1084](https://github.com/cairn-ehr/cairn-ehr/issues/1084) (an additive
  comment on an allergy line, a new act), [#1085](https://github.com/cairn-ehr/cairn-ehr/issues/1085) (flag a live
  allergy whose drug matches a coded tolerance).
- **Builds on:** [ADR-0097](0097-allergy-overlays-and-differing-accounts.md) (the de-label: a signed
  `clinical.allergy-challenge` overlay; the struck line stays visible) ·
  [ADR-0098](0098-allergy-coding-and-the-medication-check.md) (the allergy check) ·
  [ADR-0099](0099-charting-a-medication.md) (the line, the check at charting) ·
  [ADR-0100](0100-charting-a-list-under-one-signature.md) (the list and its one Sign) ·
  [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md) (soft policy in the window)

## Context

A patient carries an allergy label. While taking the medication history the clinician asks, hears that the patient
tolerates the drug, and wants the label struck, early, because it changes every prescribing decision after it. On
paper: strike the allergy line, write "tolerated amoxil 12/11/26" beside it, initial it.

Two maintainer rulings (2026-11-12) shape the slice. Lifting an allergy is always the clinician's deliberate act, never
offered for a related drug and never done automatically. And a drug on the medication list does not attest that it was
tolerated: charting a drug writes nothing to the allergy list.

The charting design (§3.5) had planned, for *Already taking*, a group comment on the allergy and one transaction
covering the line and a taken de-label. The maintainer dropped both for this slice (2026-11-12). The comment is #1084.

## Decision

### The rule and the offer

1. **Charting never writes the allergy list.** The list's Sign submits medication events only. The only way
   an allergy is de-labelled is the existing `allergy_challenge` command, signed on its own.
2. **The offer is decided by code equality only.** `same_drug_heads` returns the live allergy heads whose coding has
   system `drugref-moiety` and a code that parses as the same `Uuid` as the line part's moiety code. It looks at no
   name, no group and no drugref answer. The allergy check may word a name or drug-list match "same drug" for a warning
   (ADR-0098); that bias is safe for a warning, since a false warning costs a glance, and unsafe for a de-label, which
   lifts an allergy.
3. **Only an allergy, only live, only a drug.** A head is considered only when its allergen kind is `drug` and its kind
   is `allergy` (an adverse-effect entry, a "none known" statement and a kind added by a newer peer are not offered).
   `r.live` already leaves out de-labelled and withdrawn heads. A head on a chart whose link is in doubt is excluded,
   because the window's de-label command refuses such a target.
4. **Unsigned list lines only.** The offers are derived in `judge` for the clinician's own list lines (a line from
   either *Start medication* or *Already taking*), only when the allergies were read over the chart set on screen and
   the line's parts are signable. A line already charted gets none, and slice 1a's lone-line editor gets none: a
   charted medication that hits an allergy keeps its warning, and acting on the allergy is done on the allergy list.
   The window draws an offer's button disabled exactly when the line's other controls are: while another terminal holds
   the list, or while the list's Sign is in flight.
5. **One control per allergy record.** `offers_for` takes each part coded `drugref-moiety` and, for each live head of
   the same drug, makes one offer, deduplicated by target. A combination offers only for the part that matches. Several
   live entries for one drug give several controls, one per record (worded by term and byline only, so two alike may
   read alike; a press brings its own record's line into view); none is struck by another's press. The words come from
   `offer_words` ("De-label the {allergy} allergy: tolerates {drug} (recorded by {signer})", the tail omitted when the
   signer is unknown). Rust words and derives them; the webview draws one button per offer.

### What pressing it does

6. **A press signs nothing.** `pressDelabelOffer` finds the allergy pane's drawn live line for the offer's target and
   opens that line's existing De-label row (`openDelabel` with a prefill): *Tolerated* is the line's drug, its coding is
   held, the note is empty, and the date is today on an *Already taking* line but empty (not known) on a *Start
   medication* line, whose drug the patient has not yet had in this episode (a date is a claim, and "tolerated today" is
   true only of a drug already being taken). The allergy pane's state is read first: while it may not write (a press in
   flight, no list drawn for the chart on screen) a press says why at the allergy pane and opens nothing, never that the
   allergy is gone, which would be a claim about a list nobody read. With a list drawn but no such line in it (the list
   moved on) it says the allergy is no longer on the list, at the allergy pane and beside the pressed line, once (the
   line's next draw takes the note down), and opens nothing. While its line is being edited the line draws no offer:
   like its marks, an offer is about the saved words. There is one de-label act and one form, whichever way it is
   reached.
7. **The de-label signs at once, as its own act.** The clinician signs the row with the signer strip as for any
   de-label: under its own signature, before and independent of the list's Sign (a held key costs no passphrase). It is
   never ticked, pre-pressed or bundled with the list's Sign, and shares no transaction with a line. After an overlay
   signs, the allergy list and the medication list are read again, so every list line is re-checked against the
   corrected allergies (ADR-0100's rules apply unchanged).

### The tolerated drug, coded (#1053)

8. **A de-label may code the tolerated drug.** `AllergyChallenge.tolerated_coding` is an optional `SubstanceCoding`;
   the payload carries `{system, code, display}` or null. The wire is unchanged: `clinical.allergy-challenge/1`
   already reserved `tolerated.coding`, and the projection already stored `tolerated_coding`. The twin and the struck
   line's words still name the term as tolerated.
9. **The authoring door admits only a drug.** `db/075`'s `cairn_check_allergy_challenge` admits a non-null
   `tolerated.coding` only with system `drugref-moiety` (the shape check by `cairn_check_coding_object` runs at both
   doors). The system rule is below the remote-apply return, so the remote door is unchanged and a newer peer's finer
   coding never wedges sync. A tolerance is of a drug, never of a group.
10. **The window checks the pick before the key.** `tolerated_coding_before_the_key` refuses a coding whose system is
    not `drugref-moiety` (`NOT_FROM_THE_LIST`) or whose display is blank or whose code does not read as a `Uuid`
    (`BAD_CODE`), and re-spells the code canonically. In `allergy_challenge_impl` the order of refusals is: the chart on
    screen and writable (`on_screen_and_writable`), what was tolerated is named, the coding, the date, the target, the
    key. The door checks shape and system only and never looks the drug up.
11. **The row's pick is optional and drops on any edit.** The De-label row gains a drugs-only pick (`med_drug_search`,
    slice 1a's). Typed text with no pick stays uncoded. Any edit of the *Tolerated* field drops the held coding, and
    while one is held the row says "Coded as: {drug} (drug)". The offer's prefill and a pick on the allergy list use
    the same row, so a de-label from the allergy list can be coded too.
12. **No new file, no generation bump.** The `db/075` change is in place. A connect replays the schema when its content
    hash changes (ADR-0080 decisions 1-2), so an existing database receives the edit on its next connect.
    `SCHEMA_GENERATION` stays 78: it moves only when a new `db/` file lands and gates the projection heal (ADR-0057
    decision 4), and this slice changes no projection.

## What is not guaranteed

- **No conflict flag** when a live allergy's code meets a coded tolerance, or an allergy recorded after a tolerance
  ([#1085](https://github.com/cairn-ehr/cairn-ehr/issues/1085)). This slice only records the code.
- **No comment on an allergy line** from the medication list. A comment is a new allergy act with its own design
  ([#1084](https://github.com/cairn-ehr/cairn-ehr/issues/1084)).
- **No offer on a charted line**, and none on a match made by name, by group or by drugref's fold: lifting such an
  allergy is the clinician's act on the allergy list.
- **No offer for an uncoded line or an uncoded allergy.** A line typed without a pick, or an allergy recorded as text,
  has no code to equal.
- **The offer is soft policy** (ADR-0021). It lives in the window; a client of the same doors can de-label any live
  allergy, as before, and can code the tolerated drug.
- **The DB-gated window tests run locally only** (#905); the real window's WebKit, the real Tauri IPC, VoiceOver and
  every stopwatch figure are owed by a human (RUNBOOK §32).

## Consequences

- Paper *N* = 1 act (strike, note, initial) → architecture *M* = 1 signed de-label event → UI *K* = 1 press (the offer)
  plus 1 Sign on the pre-filled row. From the allergy list directly, *K* is as before plus an optional pick (M ≤ N).
  Budget: at most 10 s from the press to the struck line, key held; the line's drug and the allergy it would strike are
  both readable before the press, without a click.
- A reflex press is one press from a de-label, but the row opens pre-filled and still needs its own Sign, and a
  de-label can be withdrawn (ADR-0097's *Withdraw de-label*).
- The medication list and the allergy list stay independent records; the code is the only link between them.
