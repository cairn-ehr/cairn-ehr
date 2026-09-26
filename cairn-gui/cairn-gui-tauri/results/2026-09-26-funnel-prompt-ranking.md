# The step-3 prompt after ADR-0075: is the duplicate among the five shown? — 2026-09-26

**Why this was measured.** [ADR-0075](../../../docs/spec/decisions/0075-the-step-3-prompt-is-a-nudge-not-a-completeness-claim.md)
(#671) made the step-3 prompt a best-effort nudge: it shows the five closest charts, and being cut
is its normal state. That leaves the ORDER as the one lever that helps the person at the desk
without asking them for anything. `search_patients` now ranks by passes matched, then an
**identifier match**, then a **§5.4 callsign typed whole**, then **name tokens matched** (exactly or
as a typed prefix of at least 3 bytes, as `db/046` matches them), then a **DOB near-miss** (day/month
swapped, year ±1, the year's last two digits transposed, or the same date written differently),
then **exactly-matched tokens**, then chart age (`cairn_patient_search::rank`). The rig seeds no
callsigns, so the callsign key (added in the review's third round) is false throughout and cannot
move any figure below. The identifier key, the prefix rule and the exact-token tie-break
came from the PR #678 review; the tables below report the order as first reviewed and the final
one side by side. The 2026-09-23 run
([2026-09-23-funnel-prompt-truncation.md](2026-09-23-funnel-prompt-truncation.md)) found the
2c ranking helped an exactly-typed duplicate only. This run measures the new ranking against every
realistic way the duplicate can be mistyped, and against a control that the near-miss key cannot
help.

## Rig

| | |
|---|---|
| Host / CPU | MacBook, Apple M3 Max |
| PostgreSQL | 18.1 (Postgres.app), database `cairn_test` |
| Rig | `scripts/measure_prompt_truncation.py` (self-test passes; Python twins of `rank_candidates`, `tokens_matched`, `is_dob_near_miss` pinned to the Rust unit tests' examples) |
| Population | 50,000 (spec §8.1), rows straight into `patient_name` + `patient_demographic` + one MRN each in `patient_identifier` |
| Dates of birth | uniform 1930–2025, day ≤ 28, seeded (`--seed 20260923`) |
| Searches | 500 sampled patients, each searched for as a duplicate registration |
| Name pool | `~/src/SyntheticHealthData/synthetic_demographics.sqlite3` (real Australian names; ⚠️ not the current generator version — the name distribution is what matters and this copy's is realistic). ⚠️ **Drawn from the pool's FIRST 50,000 rows**, which hold ~4× the table's share of its commonest surnames (Smith 0.72% vs 0.18%); see the limit below |

**The question per search:** someone ALREADY on file is being registered again. Is their existing
chart among the five the prompt shows?

> [!WARNING]
> **Stated limit: these real-name figures come from an unrepresentative draw.** `pool_names` took
> the pool's first 50,000 rows, which over-represent common surnames about fourfold (review of PR
> #678). More namesakes means bigger candidate sets and more competition for the five, so the
> figures are likely *conservative* — but they are not the figures for the population described.
> The rig now draws every k-th row across the table (`pool_names(..., spread=True)`) and reports
> each arm over only the searches it actually perturbed (`perturbed_only`); the re-run, and the
> correction of every figure quoted from this file, is
> [#685](https://github.com/cairn-ehr/cairn-ehr/issues/685).

## Result — real names

"2c" is slice 2c's order (passes matched, then chart age); "as reviewed" is ADR-0075's order as PR
#678 first proposed it (passes, exact name tokens, DOB near-miss, chart age); "**final**" adds the
review's three changes.

| How the duplicate was typed (`--perturb`) | In the candidate set | Among the five — chart age | — 2c | — as reviewed | — **final** | Median position, final |
|---|---|---|---|---|---|---|
| exactly (`none`) | 500 | 100 | 500 | 500 | **500** | 1 |
| DOB slip — day/month swapped or year off by one (`dob`) | 500 | 100 | 100 | 500 | **500** | 1 |
| DOB simply wrong — any other date (`dob-any`, the near-miss key's control) | 500 | 97 | 97 | 500 | **500** | 1 |
| surname typo, DOB right (`name`) | 500 | 157 | 500 | 500 | **500** | 1 |
| surname typo AND DOB slip (`both`) | 500 | 156 | 156 | 500 | **500** | 1 |
| surname typo AND DOB simply wrong (`both-any`) | 500 | 155 | 155 | 204 | **203** | 10 |
| first name cut to a prefix AND DOB simply wrong (`short-any`)¹ | 500 | 63 | 63 | 286 | **499** | 1 |
| the chart's MRN, ANOTHER chart's name, DOB simply wrong (`ident`) | 500 | 100 | 100 | 48 | **500** | 1 |

¹ 38 of the 500 sampled first names were under four characters and could not be cut; the rig sent
those names as-is and reports the count (`unperturbed`) rather than passing them off as shortened.
The surname-typo arms (`name`, `both`, `both-any`) likewise report **5 of 500** surnames under three
characters ("Li", "Wu") that had no interior letter to change. Before the review they were counted
as typo'd searches without saying so; the bias is small (1%) but it is now stated.

Candidates per search: median ~104 (typo arms ~60, `short-any` ~163 — a 3-letter prefix is broad),
and the prompt is cut on ~80–96% of searches — the state ADR-0075 accepts as normal.

## Result — synthetic common names (worst case)

No pool: names drawn with a 1/rank skew from ~40 given names × ~50 surnames, so exact full-name
twins are everywhere (median 7,576 candidates per search, max 20,547).

| `--perturb` | Among the five — chart age | — 2c | — as reviewed | — **final** | Median position, final |
|---|---|---|---|---|---|
| `none` | 2 | 500 | 500 | **500** | 1 |
| `dob-any` | 2 | 2 | 72 | **72** | 42 |
| `ident` | 2 | 78 | 77 | **500** | 1 |

## What it means

0. **The review's two gaps were real, and the `ident` arm shows the first was a regression.** A
   chart found only by its MRN was shown in **48/500** searches under the order as reviewed —
   *fewer* than plain chart age (100/500), because every one-token namesake now strictly outranked
   it. With the identifier key it is first in all 500, in the twin-heavy synthetic population too
   (77 → 500). A shortened first name with a wrong DOB went 286 → **499**. Measuring the prefix
   rule on its own (before the exact-token tie-break) cost `both-any` 204 → 183, because every
   prefix-only candidate joined the duplicate's tie; the tie-break restores it to 203. The one
   search still lost is a chart matching two tokens, one by prefix, which legitimately outranks a
   one-token duplicate.
1. **The name-tokens key is what does the work.** With the name typed right, the duplicate is in
   the five in every search whatever happened to the date of birth (`dob`, `dob-any`): a real
   two-token name rarely has many exact twins. The DOB near-miss key only matters once a name
   token is lost too (`both`: 156 → 500).
2. **A typo in the surname is still FOUND** — through the given name and, when right, the DOB
   pass. ADR-0075's first draft said a name typo "never enters the candidate set"; measured, that
   is false for a one-token typo, and the ADR was corrected before merge. What cannot be found is a
   name with every token misspelt; this rig does not model it.
3. **Where the prompt stops helping — the repair path's territory:**
   - a surname typo together with a date of birth that is simply wrong (`both-any`): **203/500**
     (41%) — the chart is left with one given-name token and nothing to break the tie;
   - a population with many exact full-name twins and a wrong DOB (synthetic `dob-any`):
     **72/500** — no order can pick the right John Smith without the date.
   These are the cases ADR-0075 decision 2 hands to the §5.2 matcher's commit-time check and the
   link-repair path.
4. **Caveat, stated:** the `dob` arm's slips are exactly the ones the near-miss key rewards, so it
   grades the rule on its own test. The `dob-any` and `both-any` arms are the controls, and the
   conclusions above lean on them.

**Re-run after the review's remaining fixes** (the same date written differently counts as a DOB
near-miss; a punctuated word is skipped only when its own parts are query tokens, so Turkish `İ`,
Thai, Devanagari and initials like `J-P` count; the rig reports unperturbable names): all eleven
arms reproduced the "final" figures above exactly. The real-name pool is Latin-script and zero-padded,
so the rig does not exercise those two fixes; the unit tests
`a_word_whose_parts_do_not_stand_for_it_counts_whole` and
`the_same_date_written_differently_is_a_near_miss` do.

**Earlier re-run, after the final review's fix** (`tokens_matched` then counted only plain tokens —
a rule since replaced by `is_represented_by_its_parts`, see the re-run above — so a hyphenated word
counted once per part): all eight arms reproduced the figures of the first run
exactly. The
real-name pool is almost entirely unpunctuated, so this rig does not exercise that fix — the unit
test `a_hyphenated_given_name_does_not_outweigh_a_matched_surname` does.

## Latency of the two new reads

`search_patients` gained two reads: every retained name of the candidates, and the query's
tokens, both normalised by Postgres. Timed in isolation on a 50,000-row `patient_name` (seeded and
rolled back in one transaction) over **968 candidate ids — the largest set any REAL-NAME search
above returned**: 2.3–4.7 ms (warm to cold), and 0.2–0.5 ms for the token normalisation. The
synthetic worst case (up to 20,547 candidates) was NOT timed; the read is linear in the candidate
count, so expect it to be roughly 20× that, a figure owed rather than measured. The review's changes
add no read: one `bool_or` in the existing candidate query, and a second in-memory token count. Against the
5 s find budget this is nothing; even an order of magnitude slower on a Pi it stays well under the
~860 ms `db/046` floor #637 is about. (`measure_patient_search.py` times `cairn_search_candidates`
only and cannot see these reads, so it was not the instrument.)

## Reproduce

```bash
uv run --no-project python scripts/measure_prompt_truncation.py --self-test
for arm in none dob dob-any name both both-any short-any ident; do
  uv run --no-project python scripts/measure_prompt_truncation.py --dbname cairn_test --rows 50000 \
      --samples 500 --perturb $arm --name-pool ~/src/SyntheticHealthData/synthetic_demographics.sqlite3
done
for arm in none dob-any ident; do   # synthetic worst case
  uv run --no-project python scripts/measure_prompt_truncation.py --dbname cairn_test --rows 50000 \
      --samples 500 --perturb $arm
done
```

Do not run while a `cargo test` suite is using `cairn_test`: the rig refuses to report if the
population changes mid-run, and — since a suite leaves its last fixtures behind — if any candidate
is a chart it did not seed. Clear them first (`DELETE FROM patient_name / patient_demographic /
patient_identifier WHERE asserted_origin <> 'measure-prompt'`, on a test database only).
