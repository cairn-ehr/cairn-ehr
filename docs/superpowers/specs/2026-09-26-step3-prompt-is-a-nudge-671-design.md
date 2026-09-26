# Design — the step-3 prompt is a nudge, not a completeness claim (#671)

- **Date:** 2026-09-26
- **Issue:** [#671](https://github.com/cairn-ehr/cairn-ehr/issues/671)
- **ADR:** [ADR-0075](../../spec/decisions/0075-the-step-3-prompt-is-a-nudge-not-a-completeness-claim.md)
- **Spec sections:** §5.3 / §5.8 (search-before-create, [identity.md](../../spec/identity.md))
- **Builds on:** [ADR-0061](../../spec/decisions/0061-registration-is-an-act-that-carries-its-search.md),
  [ADR-0060](../../spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md),
  the funnel design `2026-09-20-registration-search-funnel-ui-design.md` (its *Risks* bullet on the
  bounded prompt is the condition this answers)

## Context

Slice 2c measured the step-3 prompt over 50,000 real names
(`cairn-gui/cairn-gui-tauri/results/2026-09-23-funnel-prompt-truncation.md`):

- `db/046` is a disjunction (any name token OR exact DOB), so a full-name-plus-DOB search returns a
  median of ~100 candidates. The prompt shows `PROMPT_CAP` = 5, and `bound_for_prompt` sets
  `incomplete` whenever it cuts — so **92% of registrations sign `incomplete: true`**, and the flag
  says nothing.
- Ranking by passes matched puts an **exactly-typed** duplicate first 500/500, but a duplicate typed
  with a **wrong DOB** is shown only 100/500: it matches the name pass alone, which counts once
  however many name tokens matched, so it ties with ~100 namesakes and falls to chart-age order.
- A duplicate with a **typo in a name token** ("Smyth" for "Smith") loses that token's match and can
  be found only through its other keys; with every token misspelt it is not found at all, and no
  prompt logic can show it. *(Corrected 2026-09-26 by measurement: a one-token surname typo is still
  FOUND through the given name and the DOB — see the result file.)*

## The maintainer's framing (decided in the brainstorm)

Duplicates are common in practice, mostly from typos in hard-to-spell names. We cannot make the
person at the desk — clerk, nurse, doctor — wade through a long list when they only want to open or
create a chart. People do the right thing most of the time; what matters is that a mistake has no
serious consequence. Cairn's answer is **accept that duplicates happen and make repairing them easy
and safe** — which is exactly what principle 2 (*never merge, always link*) exists for: a duplicate
is repaired by an auditable, reversible `link`, with no data loss.

The residual hazard is the **window** between the duplicate's creation and its link (an allergy on
chart A, a drug charted on duplicate B). So the safety measure is **how fast a duplicate is found**,
not whether the clerk looked. That work — a commit-time local duplicate check, a worklist, a link
gesture — is the next design thread, filed as issues here, not built.

## Decisions (recorded in ADR-0075)

1. **The prompt is a best-effort nudge.** It shows the closest few charts; it never claims to show
   every candidate, and truncation is its normal state, not a defect. The funnel design's *"if it is
   routinely incomplete, the cap is wrong"* condition is retired.
2. **`search.incomplete` means what ADR-0061 said: the SEARCH was partial** (the node could not read
   some candidate it found). `bound_for_prompt` stops OR-ing display truncation into it. No wire
   change: db/045 already requires a boolean. Test fixtures signed before this date may carry
   `true` for truncation; no clinical data exists (pre-clinical), and the ADR states the date.
3. **Truncation stays visible on screen, not in the signature.** `PromptList` carries `withheld`
   (count not shown) for one quiet line — *"the 5 closest of 103 shown · type more to narrow"*. No
   signed count: the size of a disjunctive search is noise to a later reader.
4. **Ranking gets better for free.** It only reorders; it never adds or removes a candidate.

## Ranking

A pure function in `cairn-patient-search` (`rank_candidates`), ordering by:

1. **Passes matched**, descending (today's key).
2. **Name tokens matched**, descending — how many DISTINCT query tokens equal a token of any of
   the candidate's RETAINED names (`patient_name`, repudiated values included — the same set
   `db/046` searches, deliberately, #349), both sides tokenised by `SearchQuery::new`'s rule and
   compared lowercased and NFC-normalised (normalisation done by Postgres, as `db/046` does it).
3. **DOB near-miss**, true first — both the query DOB and the candidate's are full ISO
   `YYYY-MM-DD`, they differ, and the candidate's is the query's with day and month swapped, or the
   year ±1, or the year's last two digits transposed (1967↔1976). Partial-precision dates never
   count. An EXACT DOB is already rewarded by key 1 and is not a near-miss.
4. **Chart age** (UUIDv7 id), ascending — today's tie-break.

`search_patients` ranks after its per-candidate reads (it already reads DOBs), with one new read of
the candidates' retained names. **Stated limit:** keys 2 and 3 are computed in Rust and may drift
from `db/046`'s SQL normalisation. Drift can only worsen the order, never lose a candidate, so it is
stated in the doc comment rather than pinned by a cross-language twin.

## Measurement (success criteria)

`scripts/measure_prompt_truncation.py`'s Python `rank()` twin is extended to the new keys and
pinned against the Rust unit examples (self-test). Runs over the same 50,000 real names:

| Arm | Before | Target |
|---|---|---|
| exact name + DOB | 500/500 | stays 500/500 |
| name + wrong DOB (`--perturb dob`) | 100/500 | close to 500/500 |
| surname typo (`--perturb name`, new) | not measured | **recorded, not optimised** — plus `both`, `dob-any` and `both-any` arms added while measuring, to find where the prompt stops helping |

## Testing (TDD)

- Pure unit tests in `cairn-patient-search`, one per key, each with a tie only that key can break;
  the DOB near-miss predicate over swap / year ±1 / transposition / partial dates / exact match.
- `bound_for_prompt`: truncation no longer sets `incomplete`; a node-partial list still does, with
  its reason; `withheld` is the exact count not shown.
- DB-gated (`patient_search_ranking.rs`): a same-name, wrong-DOB existing chart ranks first among
  more namesakes than the cap; a two-token match outranks a one-token match.

## Out of scope

The repair path (commit-time duplicate check, worklist, link gesture) — filed as issues. Any change
to `db/046` or `db/045`. `PROMPT_CAP` stays 5.

## Paper-parity

Paper counterpart: the clerk glancing at the card index before writing a new card. The prompt adds
no step (ranking is invisible) and removes a false signal; the §1.2 budget is the plan's section.
