# Design — a date of birth one keying error away is not a contradiction

- **Date:** 2026-10-11
- **Origin:** PR #749's review (R5b) — the maintainer's question "give me a real-life example" of a vetoed
  `review` row being withdrawn, and the rule they proposed after it (below).
- **Spec sections:** §5.2 (matching pipeline), §5.13 (locale-pluggable comparators)
  ([identity.md](../../spec/identity.md))
- **ADR:** one new ADR (numbered after PR #749's ADR-0078 lands — expected **0079**): the comparator
  contract gains an agreement level, which locale packs plug into ([ADR-0014](../../spec/decisions/0014-locale-pluggable-matcher-comparators.md)).
- **Builds on:** the advisory matcher (ADR-0014), the db/016 veto floor, R4's commit-time check (db/056),
  R5b's worklist and the `review`/`retracted` statuses (#743).
- **Depends on:** PR #749 merging first (it renames the matcher's `retract_pending_proposal` /
  `pending_proposal_pairs` / `pending_pairs_involving` to `awaiting_*`; this work touches the same files).
- **Revised in PR #758's review (2026-10-12), by the maintainer:** `NEAR` is priced **+2**, not 0.0
  (decision 2), and a DOB graded `DISAGREE` is capped at review as well as `NEAR` (decision 3). The
  `dob_near_miss` entry follows the scored level (decision 4), and the `name+monthday` key reads
  `parse_dob`'s grammar exactly (decision 5). Superseded text is kept where it explains a choice.

## Context

### The failure

Mary Smith, born **7 January 1950**, has an old chart holding her warfarin and her penicillin allergy, and
no Medicare number on it. She arrives by ambulance, confused; the ED clerk registers a new chart,
"Mary Smith, F, 07/01/1950". The matcher scores the pair 12 (name exact +5, DOB exact +6, sex +1) — an
auto-link candidate. Next morning a ward clerk "verifies" the new chart's DOB against a transfer letter
that carries a day/month swap, and overwrites it with **1 July 1950**. The DOB edit queues the chart
(db/056); the matcher re-scores: name +5, **DOB −4**, sex +1 = **2.0**, below the review floor (3.0), no
shared identifier to rescue it — so the proposal is **withdrawn**, and nobody ever saw it. The chart the
ward prescribes on has no warfarin and no penicillin allergy.

The root cause is one number: `compare_dob` grades *any* differing digit `DISAGREE`, weighted **−4**. A
pair that shares no identifier can never absorb that, whatever else agrees. Today (base weights at
full provenance; a lower provenance only scales every term down):

| Scenario | Score | Shown? |
|---|---|---|
| Married: "Mary Smith" → "Mary Jones", same DOB | −2 + 6 + 1 = 5 | yes |
| Renamed: "Bob Brown" → "Robert Brown", same DOB | −2 + 6 + 1 = 5 | yes |
| Same name, DOB one keying error off | 5 − 4 + 1 = **2** | **no** |
| Married *and* a DOB keying error | −2 − 4 + 1 = −5 | no |

### The maintainer's rule, and why it is not the one built

The maintainer asked that a shared first name **or** surname **and** a DOB within one year and a day be
presented as a possible match (marriage, a first-name change, one DOB number entered wrong). Measured on
the synthetic Australian name population (6.57 M names), with DOBs drawn from the ABS age pyramid —
every sampled person distinct, so **every flagged pair is a false alarm** (a lower bound: the name table
has no birth-cohort effect on first names):

| Charts | The rule as stated | + same sex | One keying error + one shared name | One keying error + same full name |
|---|---|---|---|---|
| 10 000 | 1 132 | 740 | 57 | 0 |
| 50 000 | 25 755 | 16 329 | 1 041 | 0 |
| 100 000 | 103 789 | 66 324 | 4 332 | 0 |

Per registration that is ~0.2 false proposals at 10 k charts and ~2 at 100 k (~300 a day for a busy ED; ~20
per registration at a 1 M-chart hub) — a tray no clerk can work, in which real duplicates are lost: a
paper-parity failure. The scenarios split instead: marriage and renaming **already surface** (table above);
the gap is the DOB keying error. The maintainer chose (2026-10-11) the targeted fix, **excluding** the
married-*and*-keying-error case, and ruled that a near-miss DOB **always goes to a human**.

Caveat on the last column: the sample holds only 126 same-full-name pairs per 100 000 (this name table
produces few namesakes; real "John Smith"s are far more common). Expected noise is roughly *namesake
pairs × ~0.2–0.3 %* (the chance two random DOBs sit one keying error apart) — low, not zero. The plan
re-measures it on the ABS-modelled population with diagnoses (the current dataset copy is not that one).

## Decisions

1. **A new agreement level, `NEAR`.** `compare_dob` returns `NEAR` when two **full-precision point
   dates** (year, month and day on both sides) differ by exactly one **keying error**:
   - exactly one digit differs (`YYYYMMDD`, any position — 1950→1960 and 07→08 alike);
   - two **adjacent** digits are swapped (day 12↔21, year 1958↔1985);
   - day and month are swapped, same year (07/01 ↔ 01/07);
   - same day and month, years one apart across a carry (1959 ↔ 1960, 1999 ↔ 2000 — more than one
     digit changes, so the one-digit rule misses it; 1950 ↔ 1951 is already "one digit differs").

   Anything else still grades `DISAGREE`. Coarser precisions (year-only, year-month) and §5.4 year
   ranges keep their current rules untouched — a keying error is only recognisable when both sides
   claim a full date.
2. **`NEAR` weighs +2** (revised from 0.0) — weak evidence *for*, far short of an exact DOB (+6): the
   digits fit one person and a typo, and a one-keystroke miss is rare between strangers. Same name +
   `NEAR` → 5 + 2 + 1 = 8 (review, capped); "Mary Smith"/"Mary Jane Smith" (names `PARTIAL`, priced 0) +
   `NEAR` → 0 + 2 + 1 = 3 (review at full provenance only — #762); married + `NEAR` → −2 + 2 + 1 = 1
   (not shown — decided). "Smith"/"Smyth" is *not* an edit-distance agreement at the default 0.90
   threshold (Jaro–Winkler 0.893), so it grades `DISAGREE`: −2 + 2 + 1 = 1, not shown (#762). (The first
   draft's "2.5 + 0 + 1 = 3.5" for this pair was wrong.)
3. **A DOB in doubt — `NEAR` or `DISAGREE` — forbids `AUTO_CANDIDATE`** — the band is capped at
   `REVIEW`, as any veto caps it. Someone must decide which DOB is right; that human step is also when
   the wrong DOB gets corrected. (A shared Medicare number could otherwise lift name + `NEAR` + identifier
   past the auto threshold. The first draft capped only `NEAR`, which left an unverified DOB that differs
   outright free to auto-link — identifier 4 + name 5 + sex 1 − DOB 2 at unknown provenance = 8 — since
   db/016 vetoes only a clash of two *verified* DOBs.)
4. **The proposal says why.** A proposal whose DOB was *scored* `NEAR` carries a `dob_near_miss`
   evidence entry naming the pattern ("one digit differs", "adjacent digits swapped", "day and month
   swapped", "years one apart", or `null` when a locale comparator graded `NEAR` on an error the default
   profile does not name), so the tray and banner can show it. It is derived from the scored level, never
   a second reading of the dates.
5. **A new blocking pass, `name+monthday`.** Name token + `MM-DD`, for `day`-precision point DOBs only.
   It closes the one blocking hole: a **year** keying error on a **common** name — `name+year` needs the
   same year, `dob`/`dob+first-initial` need the same date or year, and the bare `name` token block for
   "smith" exceeds the 100-chart cap at scale and is dropped. For real birthdays blocks are ~token
   frequency ÷ 365, so the cap rarely bites; *assigned* dates (01-01, 07-01, recorded when a birth date is
   unknown) heap and can exceed it (#760). Day- and month-keying errors keep the year and are already
   blocked by `name+year`.
6. **This is the default comparator profile's behaviour, not core policy.** Day/month swaps are a
   culture-dependent hazard (dd/mm vs mm/dd entry); a locale pack may grade differently. The level itself
   is part of the comparator contract (ADR-0014) — hence the ADR.

## Rejected

- **The maintainer's rule as stated** — measured above: ~2 false proposals per registration at 100 k
  charts, ~20 at a hub.
- **Reuse `PARTIAL` (+1.5) for a near miss.** Conflates a *consistent coarsening* (1950 vs 1950-03-12)
  with *conflicting digits*, and does not cap at review.
- **Keep −4 and add a banding rescue** ("a typo-adjacent DOB disagreement with agreeing names → review",
  shaped like the shared-identifier rescue). It works, but the evidence breakdown would still show the
  DOB as a −4 contradiction while the band says otherwise — two places encoding one fact.
- **Married *and* a DOB keying error** (the table's "one keying error + one shared name" column): ~0.08 false
  proposals per registration at 100 k charts. Declined by the maintainer; revisit with real adjudication
  data (§5.13's harness) if missed duplicates of that shape turn up.

## Design

### Comparator (`matcher/src/cairn_matcher/comparators.py`, `agreement.py`)

- `AgreementLevel` gains `NEAR` between `DISAGREE` and `PARTIAL` (*a clash that may be a typo* is weaker
  than *a consistent coarsening*, stronger than *a clash*). The enum is ordinal; inserting renumbers the
  levels above it, which is safe because levels persist **by name** (`banding.py`'s evidence and
  fingerprint, `eval/model_io.py`). A test checks what is written: evidence and fingerprint carry the
  name, and a weights table round-trips `"NEAR"`.
- A small pure helper, `keying_error_between(a: DateValue, b: DateValue) -> str | None`, returns the
  pattern's name or `None`; `compare_dob` calls it only in the point-vs-point, both-full-precision,
  any-part-differs branch, and only for dates that can exist (month 1–12, day 1–31, a four-digit year).
  Each pattern is unit-tested with its negatives (two digits differ; non-adjacent swap; day/month swap
  across different years; years two apart; a swap into an impossible date; New Year's Eve vs Day).
- Name comparison is unaffected: `NEAR` is emitted by `compare_dob` only.

### Scoring (`scoring.py`)

- `DEFAULT_WEIGHTS["dob"]` lists `NEAR: 2.0`. A learned (B3) table without the key scores it 0.0 —
  `score()` prices a level missing *within* a field's entry at 0.0 by design; a pre-0.2.0 learned table
  should be rejected when the pipeline first loads one (#765).

### Banding (`pipeline/banding.py`)

- The DOB graded `NEAR` or `DISAGREE` → never `AUTO_CANDIDATE` (checked beside the veto rule). A test
  pins monotonicity: with everything else fixed, a weaker DOB never bands higher.
- `_corroborated_positive` (the §5.4 unconfirmed-chart gate) counts `NEAR` as neither positive nor
  disagreeing — a typo-adjacent DOB is no corroboration, but it is no clash either. Pinned by a test.
- `build_payload` appends the `dob_near_miss` evidence entry, built from the scored DOB level.

### Blocking (`pipeline/blocking_sql.py`, `blocking.py`)

- `name+monthday` joins `ALL_PASSES` / `SYMMETRIC_PASSES`. The key is the token plus the value's
  zero-padded `MM-DD` for `precision = 'day'` only, read with exactly `parse_dob`'s day grammar
  (surrounding whitespace ignored, four ASCII year digits, a 1–2 digit month 1–12 and day 1–31,
  nothing after) — a
  value the scorer can grade joins, and a value of any other shape does not.
- Registered through `require_registered` like every pass; the per-chart targeted path
  (`targeted.py`) gains the same pass.

### Rollout

Changing a weight, a threshold or the comparator wiring changes the matcher's config fingerprint; a
change to a comparator's *code* or to a blocking pass does not, so it must bump the package version
(0.2.0 here; #765). Either way `matcher_version` changes, which re-queues **every** chart (db/056) — one full re-scan per node, the cost
#725 / #728 measure. It is also the moment the Mary Smith class of pair re-surfaces across the existing
population: the upgrade is itself the backfill.

### What does not change

- The db/016 veto floor: a *verified* DOB clash still vetoes; `NEAR` adds a cap, never removes one.
- The review floor (3.0) and the auto threshold (8.0).
- Retraction: a pair is still withdrawn when it bands below the floor — but a near-miss pair whose
  names agree exactly now scores ≥ 3 at any provenance, so the Context's story is never withdrawn.

## Testing

TDD throughout (house rule 2):

- **Pure** — `keying_error_between` per pattern and negatives; `compare_dob` returns `NEAR` only for
  full-precision point pairs (year-only / year-month / ranges unchanged); `score` prices `NEAR` at +2,
  and at 0.0 without the key; `band` caps `NEAR` at review even above the auto threshold and with a shared
  identifier; `_corroborated_positive` ignores `NEAR`; evidence carries `dob_near_miss`.
- **DB-gated** (`CAIRN_TEST_PG`) — `name+monthday` groups a common-name year typo that the capped `name`
  block drops; the Context's story end to end: auto-candidate → DOB overwritten with the day/month swap →
  `check_chart` → the row stays open at `review`, never `retracted`.
- **Measurement** — re-run the volume estimate (the throwaway script lives in the session scratchpad, not
  the repo) on the ABS-modelled dataset once available; record the per-registration false-proposal rate in
  the ADR.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** a records clerk pulling the card for "Mary Smith" and noticing the birthday on
  the new card is the old one with two digits swapped — "same woman, someone mistyped it".
- **Steps:** paper 1 human act (compare the two cards) → architecture-forced 1 (the clerk answers one
  tray entry) → UI bundling target 1. M = N; the architecture adds no act.
- **Time + cognitive load:** paper budget ≤ 30 s per suspected pair; the tray entry must show both DOBs
  and the named keying pattern so the comparison is read, not reconstructed. Measured by the RUNBOOK §12
  walk once a near-miss entry exists in the fixture.

## Open items for the plan

- The ADR's text (comparator contract gains `NEAR`; the default profile's pattern set; weight +2; the
  review cap on `NEAR` and `DISAGREE`) and a one-sentence §5.2/§5.13 addition.
- Whether the tray's sentence for `dob_near_miss` belongs in this slice or a follow-up (the evidence is
  recorded either way).
