# ADR-0079 — A date of birth one keying error away is not a contradiction

- **Status:** Accepted
- **Date:** 2026-10-11
- **Spec version at acceptance:** 0.81
- **Issues:** none — a maintainer decision of 2026-10-11, raised in PR #749's review
- **Relates to:** [ADR-0014](0014-locale-pluggable-matcher-comparators.md) (the comparator contract this
  extends) · [ADR-0078](0078-a-machine-unlink-does-not-settle-a-possible-duplicate.md) (the worklist the
  recovered pairs reach)
- **Design:** `docs/superpowers/specs/2026-10-11-dob-near-miss-design.md`

## Context

The matcher's DOB comparator graded *any* differing digit `DISAGREE`, weighted −4. A pair with no shared
identifier cannot absorb that: two charts of "Mary Smith, F" whose DOBs differ by one keying error score
name +5, DOB −4, sex +1 = 2, below the review floor (3.0). So when a clerk overwrote one chart's DOB with
a day/month swap (7 January → 1 July 1950) while "verifying" it against a letter, the matcher's re-check
withdrew an auto-link candidate that nobody had seen; the chart a ward prescribes on then carried neither
the warfarin nor the penicillin allergy on the other.

The maintainer first asked that a shared first name **or** surname with a DOB within a year and a day be
presented as a possible match. Measured on the synthetic Australian name population with an ABS age
pyramid, that rule produces about 0.2 false proposals per registration at 10 000 charts and about 2 at
100 000. It was declined in favour of the targeted rule below, once the scenarios were scored: a changed
surname or first name with an exact DOB already scores 5 and reaches review; the gap is the keying error.

## Decision

1. **A new agreement level, `NEAR`,** between `DISAGREE` and `PARTIAL`. Levels are persisted by name, so
   inserting it renumbers nothing stored.
2. **`compare_dob` grades `NEAR`** when two **full-precision point dates** (year, month and day on both
   sides) differ by exactly one keying error, named — first match wins — as "one digit differs", "adjacent
   digits swapped" (read in `YYYYMMDD` order, so a swap across the year/month boundary counts and one across a
   day-first entry's day/month boundary does not), "day and month swapped" (same year), or "years one apart"
   (same day and month). Every
   other difference is still `DISAGREE`; year-only and year-month precisions and §5.4 year ranges are
   unchanged.
3. **The default weights price `NEAR` at 0.0** — honest uncertainty (principle 4): the digits fit one
   person and a typo as well as two people. A weights table without the key prices it at 0.0 too; a learned
   table may price it otherwise, which decision 4's cap makes safe.
4. **`NEAR` forbids `AUTO_CANDIDATE`**, as a veto does: a human decides which DOB is right, which is also
   when the wrong one gets corrected. For the §5.4 unconfirmed-chart gate it is neither corroboration nor
   a clash.
5. **The proposal says why:** an evidence entry `{"kind": "dob_near_miss", "pattern": <the name above>}`.
6. **A blocking pass, `name+monthday`** — a name token with the `MM-DD` of a day-precision ISO DOB — so a
   year keying error on a common name is generated at all: the capped `name` block drops such names at
   scale, and every other DOB-keyed pass needs the same year.
7. **This is the default comparator profile's behaviour, not core policy.** Day/month swaps are a
   culture-dependent hazard (day-first vs month-first entry); a locale pack may grade differently
   ([ADR-0014](0014-locale-pluggable-matcher-comparators.md)).

## Consequences

- Measured on the matcher's synthetic evaluation set (`eval.generate --entities 2000 --seed 7`, whose
  generator injects DOB typos), against the previous matcher:
  - **Scoring:** of 2 000 true duplicates, unproposed fell from 351 to 276 (`missed_match_rate` 0.175 →
    0.138), all 75 recovered into review; auto-links unchanged at 594, and false auto-links unchanged at 1
    (`auto_false_link_rate` 0.002). False review proposals rose by 173, against 81 362 already produced
    when every pair is scored without blocking (7 996 000 non-matching pairs).
  - **Blocking:** true duplicates never generated fell from 175 to 136 (`pair_completeness` 0.912 →
    0.932), for 1.3% more candidate pairs (82 331 → 83 369) and no new oversized block (455 either way).
- The matcher's package version moves to 0.2.0, so `matcher_version` changes and every node re-queues
  every chart once for a re-check (db/056's `'config'` notice). That re-scan is also the backfill: pairs
  like the one in the Context return to the worklist.
- A changed surname *and* a DOB keying error together still score below the floor (−2 + 0 + 1). Declined
  by the maintainer for its noise (~0.08 false proposals per registration at 100 000 charts); revisit
  with real adjudication data if missed duplicates of that shape appear.
- The false-proposal estimate was taken on a name population that produces few namesakes; it is owed a
  re-measure on the ABS-modelled population with diagnoses (#760).
- The proposal records the pattern; the worklist and banner do not yet show it. That sentence, and its
  timed walk, are a follow-up (#759).
- No wire change, no schema change, no new event type: this is the advisory matcher only.

## Rejected

- **The maintainer's first rule** (shared first or surname, DOB within a year and a day): about 2 false
  proposals per registration at 100 000 charts, about 20 at a million-chart hub.
- **Reusing `PARTIAL` (+1.5):** it conflates a consistent coarsening (1950 vs 1950-03-12) with conflicting
  digits, shows a typo as positive evidence, and does not cap at review.
- **Keeping −4 and adding a banding rescue:** the evidence would still show the DOB as a −4
  contradiction while the band said otherwise — two places encoding one fact.
- **Also surfacing the changed-surname-and-typo case:** declined by the maintainer (above).
