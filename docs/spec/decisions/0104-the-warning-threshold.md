# ADR-0104 — The medication warning threshold as administrator policy

- **Status:** Accepted (builds batch charting slice 3: which findings count as a warning when a medication is charted)
- **Date:** 2026-11-14 (built), recorded 2026-11-14
- **Spec version at acceptance:** 0.106
- **Issues:** PR [#1101](https://github.com/cairn-ehr/cairn-ehr/pull/1101). Design:
  `docs/superpowers/specs/2026-11-11-batch-charting-1072-design.md` §3.5; plan
  `docs/superpowers/plans/2026-11-14-batch-charting-slice-3-warning-threshold.md` (maintainer rulings Q1–Q3). Refs
  [#1102](https://github.com/cairn-ehr/cairn-ehr/issues/1102) (a restore returns every site-policy row to its default).
- **Builds on:** [ADR-0100](0100-charting-a-list-under-one-signature.md) (the default tick, the hand tick, "the
  warning threshold is fixed at allergy marks until slice 3") · [ADR-0099](0099-charting-a-medication.md) (the check
  at charting, the reason on a Start line, the general `warning_override`) ·
  [ADR-0098](0098-allergy-coding-and-the-medication-check.md) (marks, notes, "not checked") ·
  [ADR-0089](0089-the-quarantine-commit-and-the-asserted-grade.md) (`note_policy`, the console-only policy row this
  follows) · [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md) (soft policy in the window, hard policy
  anchored in the database)

## Context

ADR-0098's check answers a line being charted with marks (an allergy hit), notes (a previous adverse effect) or "not
checked". Until now only a mark was a WARNING: it withheld the default tick (ADR-0100 decision 8), asked a *Start*
line for a typed reason (ADR-0099 decision 17.5) and was named in the signed `warning_override`. A hospital drug
committee may decide that a previous adverse effect must be acknowledged too, as it is written in the reaction box of
a paper drug chart. The maintainer ruled (design §3.5) that a "warning" is general, never keyed on allergy by name,
and that the threshold is a policy changed only by an administrator, never a per-user preference.

## Decision

1. **The scale and its default.** Two levels today, highest first: `allergy-hit` > `adverse-effect`. The stored word
   is the LOWEST level that warns. No row (or NULL) is the default, `allergy-hit`: only an allergy hit warns, exactly
   as before this slice. Later checks (interactions, renal dosing) add their own levels on the same scale, additively.
2. **Lowerable only.** Nothing sits above `allergy-hit`, and the table's CHECK and the setter admit only the two
   words, so no setting switches allergy warnings off.
3. **Console-only.** `medication_warning_policy` (db/081) is one row nobody but the owner reads or writes.
   `cairn_medication_set_warning_threshold(text)` has no grant to any runtime role; `cairn-node
   medication-set-warning-threshold (--level <allergy-hit|adverse-effect> | --default)` refuses a database on another
   machine before connecting and prints the level read back. `cairn_medication_warning_threshold()` is the one read,
   a definer door granted to `cairn_window` only.
4. **The check keeps each adverse-effect note's record.** `LineCheck.adverse_against[i]` is the allergy record
   `notes[i]` came from, for every `i` below its length; a note past that names no record (the window's "the allergy
   list could not be read") and is never a warning. `merge` keeps the notes with a record ahead of those without, so
   a record never slides onto another note's words. `check_medications` never reads policy; it leaves
   `LineCheck.threshold` at the default.
5. **One rule for "warns".** `LineCheck::warned_against()` is the marks' records, then, at `adverse-effect`, the
   adverse-effect notes' records; `warns()` is that being non-empty. Every former "has an allergy mark" test on the
   draft path reads these instead: the default tick (`draft_tick::default_tick`), the hand tick's re-check
   (`hand_tick_holds`), the reason gate (`override_before_the_key`), the override's `against`
   (`med_chart::override_ids`, both Sign paths), and the records a list line draws as seen. An adverse-effect warning's
   override entry is `{source: "allergy", ref: <its record's event_id>}`, as a hit's is. No wire change.
6. **The draft path only (maintainer, Q2).** The window's `allergy_check::check_lines` (the line being charted and
   each list line) reads the threshold and stamps it on every check it returns. The charted list's
   `check_list` is not stamped, so a current line is drawn as ADR-0098 draws it whatever the threshold.
7. **How a warned adverse effect looks (maintainer, Q1).** Its words stay "Previous adverse effect: …". It is drawn as
   loud as a mark (`warning_notes`, class `allergy-prominent`), and with no allergy mark the tag reads **`Warning`**,
   never `Allergy`. When an allergy on the line could not be checked as well, the tag reads `Warning · not checked`.
8. **An unreadable threshold over-warns.** A failed read, no connection, or a word this build does not know reads as
   `adverse-effect` (logged): a read that cannot say what the site decided must never quietly drop a warning.
9. **A hand tick sees the threshold.** A tick made at the default and still standing when the administrator lowers it
   no longer holds at Sign if the line has an adverse effect: it never saw that warning (ADR-0100's rule, unchanged).

## Consequences

- A site that lowers the threshold asks clinicians for one more typed reason per *Start* line with a previous adverse
  effect, and pre-ticks fewer list lines. That is the committee's choice, made once at the console.
- **Not guaranteed — a restore drops it (maintainer, Q3).** Like `note_policy` and `vault_policy`, the row is custody
  state in no backup: after `restore` the threshold is the default again, silently, and a site that had lowered it
  loses its adverse-effect warnings until it is set again. Said in the verb's help, here and in RUNBOOK §35; the fix
  for all three rows is [#1102](https://github.com/cairn-ehr/cairn-ehr/issues/1102).
- The threshold is per node. Slice 5's autonomous terminal nodes will each need it set (or #1102's option 3, a policy
  stream that syncs, ADR-0024).
- The figures and the visual check are owed by a human (RUNBOOK §35).
