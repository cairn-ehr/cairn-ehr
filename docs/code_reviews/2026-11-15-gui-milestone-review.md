# Whole-milestone review: the runnable clinical surface (2026-11-15)

The GUI milestone (enrol → sign in → find/register → demographics → note → allergy → charted medication → sign off
→ log out) was built in about twenty slices between 2026-10-31 and 11-15, and each slice was reviewed on its own PR.
This review looks only at the **seams between slices**: defects that no single slice's review could see. It covers
`main` at `e61820e5`, spec v0.109, `SCHEMA_GENERATION` 82. HANDOVER's standing gate recorded that this surface
had never been reviewed as a whole.

The review made five parallel read-only passes, each tracing one question through every layer (window JS → Tauri
command → DB door):

1. **Who the author is** across the session.
2. **Which chart a write lands on** across every pane.
3. **The allergy ↔ medication check** from end to end.
4. **The note and draft lifecycle** against sign-off and log-out.
5. **The floor as a whole** against a raw-SQL `cairn_window` client.

Every Critical and Important finding below was re-checked against the code before filing. A pass's claim that did
not survive that check is not listed. Each finding was checked against the issue list for duplicates.

Severity follows the 2026-07-15 review. **Critical** means a wrong author, a wrong patient, or silent corruption is
possible. **Important** means a real defect, or a decision owed. **Minor** means a latent problem or an edge case.

## Verdict

The **write path is sound where it matters most.**

- **The right chart:** every Rust write door resolves the chart through `displayed_patient` /
  `on_screen_and_writable` + `check_displayed_set` at entry, and `patient_id` lives inside the signed body. No
  path was found by which a write lands on the wrong patient.
- **The allergy check:** every path that charts a drug re-runs it on a fresh read at Sign. That covers list lines,
  restored lines, stat orders, re-starts, synonyms and combinations.
- **The floor:** a raw `cairn_window` client cannot get a wrong-author event past it (enrolment, revocation,
  attestation token, authorship bound to the signer, born-sealed, registration-first). Every SECURITY DEFINER in
  db/061–082 pins its `search_path` and is revoked from PUBLIC. The window has no direct write on any draft,
  vault or policy table.
- **Typed words:** note text and medication lines are flushed before every switch and re-lock.

The defects cluster in **three seams**:

- **(A) Who is acting.** The held key, the who's-here claim and the Lock button were each built in a different
  slice, and nothing ties them together.
- **(B) Leaving a chart while an act is in flight.** Each pane solved this its own way, so there is no single
  in-flight set.
- **(C) Rules stated for one stream, silent for another.** Co-sign covers notes only. The draft and quarantine
  model differs between notes, medications and allergies.

## Critical

### C1 — A held key signs for someone other than the person who's here names (#1128)

This applies only on a terminal launched with `--key-hold-seconds` above 0.

- Only a who's-here pick forgets the held key. `signer_for_act` and `keep_for_next_act` never read the claim.
- On a key-hold terminal the strip keeps the last typed ID, so `decide` answers `UseHeld`.

Dr B signs one act at Dr A's workstation. Every act Dr A presses for the rest of the hold window is then signed by
Dr B's key: her allergy records, chart lines and note Signs (ADR-0088 decision 15: "attested by Dr B").

The floor cannot catch this, because the key is valid. It contradicts HANDOVER's "never a wrong author" note on
key-hold terminals. **Fix (small):** hold a key only when the claim names nobody or the same person, compared by
key.

## Important

| # | Finding | Issue | Fix size |
|---|---|---|---|
| I1 | The vault doors (incl. `cairn_vault_release`, which returns a human's seed) and three supervision doors are granted to `cairn_agent`, the advisory role that the matcher's login inherits. An agent login can list staff and hints, take the terminal hash offline, and hold the whole staff at the back-off. db/061's "window's runtime role" comment predates db/062. | #1129 | small + follow-on ADR |
| I2 | "Find another patient" (`closeChart`) does not await `signsSettled()`. The main-pane acts (sign-off, cease, withhold, release) join no in-flight set. Patient A's answer is then said on B, and an "unknown" W sends the review to B, or A's answer is dropped and said nowhere. This is the wrong fix that durable rule R13 names. | #1130 | small (+ #709) |
| I3 | A mark's words come from the allergen's worst account, but its id is the first *hitting* account's. A hand tick for "Amoxil → rash" still holds after an uncoded "Amoxil → anaphylaxis" arrives in the same allergen group. | #1131 | small |
| I4 | A student's part that the quarantine commits unsigned owes no co-sign (`writer_signature` is `None`). A colleague's sign-off, even a fellow student's, takes it off every menu. The supervisor is never told. | #1132 | small–medium, ADR-0094 amendment |
| I5 | An unsigned medication list is visible only to its writer, never timer-committed, and missing from every menu when no writer is named. A nurse's off-shift list is invisible to the doctor. | #1133 | small, ruling |
| I6 | The path ends in "log out", but no gesture logs out. Lock forgets the key and keeps the claim, the chart and the editor, so the next person's typing is saved as the previous person's part. | #1134 | small, ruling |
| I7 | A supervisee's medication charting needs no co-sign, and no ADR says so. Allergies made the same choice openly (ADR-0096 decision 22). The rule that allergies get no draft is recorded only as a design "[my call]", and a re-lock silently wipes a half-typed allergy form. | #1135 | decision |
| I8 | The CLI `medication-assert` charts with no allergy check, warning or override, and nothing says the CLI is not a clinical surface. | #1136 | small (state it) or medium |
| I9 | Opening a person-menu row during `closeChart`'s save wait swaps the backend chart. A's queued saves are then refused silently (text lost), or the window and the node disagree on the open chart. | #1137 | small |

The consequence of #1109 is also larger than filed (comment added). A given dose stops its line, stopped lines are
never checked, so "given amoxicillin after a penicillin allergy was recorded" is shown nowhere.

## Minor and process

The minor findings are gathered in **#1138**:

1. A held key is not re-checked after a re-key.
2. The prefilled ID counts a passer-by's wrong guess against the person named.
3. A hold release resumes a marked drug with no reason.
4. With allergies "not yet asked", a draft line looks clean.
5. An uncoded drug allergy is only ever "not checked" on the write path.
6. Policy rows change with no audit trail.
7. The person menu copies db/067's `due`/`stuck` predicates.
8. `cairn_med_draft_gate` is exposed as a door.
9. #1077 understates the list path's race.

`med_chart` is a registered command that nothing invokes, and it is weaker than the list's Sign: it has no R1
seen-set binding (**#1139**).

**Process.** Two cross-stream rules live only in design docs: medication co-sign ("if wanted") and allergy drafts
("[my call]"). ADR-0096 decision 22 shows the right form, a one-line ADR decision, so the next slice cannot treat
the silence as settled. No source guard pins that every chart-leaving path awaits in-flight acts (#1130). The
sign-in plans carry no §1.2 benchmark for "end of stint" (#1134).

## Deliberately not findings

These rules live above the floor **by ADR**, and were checked as such. They are not to be moved into the floor
without a new ADR:

- requiring a `warning_override`, and the warning threshold (ADR-0099, ADR-0104; durable rule "a WINDOW rule, never
  a floor rule");
- co-sign judged at read time (ADR-0092/0094);
- a draft part's writer as a claim, not authentication (ADR-0088);
- the node-signed timer note (ADR-0089);
- registration signed by the node (ADR-0061 §4).

The draft-trust gaps #985, #987, #1082, #1094 and #1111 stand as filed.
