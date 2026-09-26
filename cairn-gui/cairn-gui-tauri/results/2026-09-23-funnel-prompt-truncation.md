# The funnel's step-3 prompt: how often it truncates, and whether the duplicate survives — 2026-09-23

> **Superseded in part by [2026-09-26-funnel-prompt-ranking.md](2026-09-26-funnel-prompt-ranking.md)**
> (ADR-0075, #671): truncation is now the prompt's normal state, and the ranking gained ADR-0075
> decision 5's keys (an identifier match, a callsign typed whole, name tokens matched, a DOB
> near-miss, an exact-token tie-break). The figures below remain the 2c baseline. They were drawn
> from the name pool's first 50,000 rows, which over-represent common surnames about fourfold;
> a representative re-run is [#685](https://github.com/cairn-ehr/cairn-ehr/issues/685).

**Why this was measured.** The registration window shows at most `PROMPT_CAP` (5) candidates in
its step-3 prompt, and the new chart's birth act attests exactly those rows as displayed
(ADR-0061). The funnel design assumed a full-name-plus-DOB search "returns few candidates by
construction" and set its own revisit condition: *if the prompt is routinely incomplete, the cap
is wrong.* HANDOVER asked slice 2c to answer that with evidence. Planning 2c found that `db/046`
is a disjunction and that `search_patients` ordered candidates by chart age, so the five shown
were the five oldest; 2c now ranks by passes matched. This run measures both.

## Rig

| | |
|---|---|
| Host / CPU | MacBook, Apple M3 Max |
| PostgreSQL | 18.1 (Postgres.app), database `cairn_test` |
| Rig | `scripts/measure_prompt_truncation.py` (self-test passes) |
| Population | 50,000 (spec §8.1), rows straight into `patient_name` + `patient_demographic` |
| Dates of birth | uniform 1930–2025, day ≤ 28, seeded (`--seed 20260923`) |
| Searches | 500 sampled patients, each searched by its own full name + DOB |

**The question per search:** a clerk is registering someone who is ALREADY on file. Is that
existing chart among the five the prompt shows?

## Result — real names

`--name-pool ~/src/SyntheticHealthData/synthetic_demographics.sqlite3` (table `names`; 6.57M real
Australian names, a long multicultural tail). ⚠️ That copy is **not** the current version of the
population generator, whose complete ABS-modelled output is on an offline archive; the name
distribution is what matters here, and this copy's is realistic.

| Measure | Value |
|---|---|
| Candidates per search | median **103.5**, p90 **363**, max **968** |
| Prompt truncated (> 5) | **460 / 500 (92%)** |
| Existing chart in the five shown — chart-age order (before 2c) | **100 / 500 (20%)**, median position 35.5 |
| Existing chart in the five shown — ranked by passes matched (2c) | **500 / 500**, median position 1 |
| Searches with > 5 candidates matching ≥ 2 passes | **0** |

## Result — the duplicate typed with a WRONG date of birth (`--perturb dob`)

Same population and pool; each sampled patient is searched with its own name but a mis-typed date
of birth (day and month swapped when that is a different valid date, otherwise the year off by
one — `perturb_dob`). This is the harder case the funnel exists for, added after the whole-branch
review pointed out that the arm above only ever measures EXACT duplicates.

| Measure | Value |
|---|---|
| Candidates per search | median 103.5, p90 365, max 968 |
| Prompt truncated (> 5) | 457 / 500 |
| Existing chart in the five shown — chart-age order | **100 / 500 (20%)**, median position 36 |
| Existing chart in the five shown — ranked by passes matched | **100 / 500 (20%)**, median position 36 |

**Ranking does nothing for this case**, and that is structural: with the date wrong the existing
chart matches only the NAME pass, and the name pass counts ONCE however many name tokens matched,
so it ties with every chart sharing any one token and falls back to chart-age order.

## Result — synthetic common names (worst case)

No pool: names drawn with a 1/rank skew from ~40 common given names × ~50 common surnames, so
shared tokens are far more frequent than in reality.

| Measure | Value |
|---|---|
| Candidates per search | median 7,576, p90 15,879, max 20,547 |
| Prompt truncated (> 5) | 500 / 500 |
| Existing chart in the five shown — chart-age order | 2 / 500, median position 2,944.5 |
| Existing chart in the five shown — ranked | 500 / 500, median position 1 |
| Searches with > 5 candidates matching ≥ 2 passes | 0 |

## What it means

1. **The ranking was necessary, and it works for EXACT duplicates only.** In chart-age order the
   prompt hid an exactly-typed existing duplicate **four times in five**; ranked, it was first in
   every search. **A duplicate typed with a wrong date of birth is still hidden four times in
   five, ranked or not** (the second table). Ranking within the name pass by how many name tokens
   matched is the obvious next lever, and it is #671's to decide.
2. **The cap truncates routinely: the design's revisit condition has fired.** 92% of
   registrations would sign `incomplete: true`, so the flag no longer distinguishes a prompt that
   could have shown the duplicate from one that could not. Filed as
   [#671](https://github.com/cairn-ehr/cairn-ehr/issues/671), a design/ADR question because it
   touches a signed body. `PROMPT_CAP` is unchanged here, deliberately.
3. **For #671, a lead and a warning.** No search had more than five candidates matching two or
   more passes. But a prompt that showed only those and withheld the single-pass remainder "by
   rule" would withhold **exactly the wrong-DOB duplicate**. That rule is not safe as stated; any
   design must be measured against the `--perturb dob` arm.

## Reproduce

```bash
uv run --no-project python scripts/measure_prompt_truncation.py --self-test
uv run --no-project python scripts/measure_prompt_truncation.py --dbname cairn_test --rows 50000 \
    --samples 500 --name-pool ~/src/SyntheticHealthData/synthetic_demographics.sqlite3
uv run --no-project python scripts/measure_prompt_truncation.py --dbname cairn_test --rows 50000 \
    --samples 500 --perturb dob --name-pool ~/src/SyntheticHealthData/synthetic_demographics.sqlite3
uv run --no-project python scripts/measure_prompt_truncation.py --dbname cairn_test --rows 50000 \
    --samples 500    # synthetic worst case
```

The rig deletes its own rows (`asserted_origin = 'measure-prompt'`) on exit and refuses to
report if the population changed mid-run, e.g. because a `cargo test` suite TRUNCATEd the table.
