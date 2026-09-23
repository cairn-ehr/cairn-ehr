# The funnel's step-3 prompt: how often it truncates, and whether the duplicate survives — 2026-09-23

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

1. **The ranking was necessary, and it works.** In chart-age order the prompt would have hidden
   an existing duplicate **four times in five** with real names. Ranked, the duplicate was first
   in every search.
2. **The cap truncates routinely: the design's revisit condition has fired.** 92% of
   registrations would sign `incomplete: true`, so the flag no longer distinguishes a prompt that
   could have shown the duplicate from one that could not. Filed as
   [#671](https://github.com/cairn-ehr/cairn-ehr/issues/671), a design/ADR question because it
   touches a signed body. `PROMPT_CAP` is unchanged here, deliberately.
3. **The lead for #671:** no search had more than five candidates matching two or more passes.
   A prompt that shows every strong candidate and states the single-pass remainder as withheld
   *by rule* would be complete in the sense that matters.

## Reproduce

```bash
uv run --no-project python scripts/measure_prompt_truncation.py --self-test
uv run --no-project python scripts/measure_prompt_truncation.py --dbname cairn_test --rows 50000 \
    --samples 500 --name-pool ~/src/SyntheticHealthData/synthetic_demographics.sqlite3
uv run --no-project python scripts/measure_prompt_truncation.py --dbname cairn_test --rows 50000 \
    --samples 500    # synthetic worst case
```

The rig deletes its own rows (`asserted_origin = 'measure-prompt'`) on exit and refuses to
report if the population changed mid-run, e.g. because a `cargo test` suite TRUNCATEd the table.
