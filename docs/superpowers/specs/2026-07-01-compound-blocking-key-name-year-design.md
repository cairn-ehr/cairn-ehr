# Design — Compound blocking key: name-token + birth-year (matcher B3)

**Date:** 2026-07-01 · **Component:** `cairn-matcher` (advisory, §9 fit-for-purpose tier) ·
**Status:** approved, ready for implementation plan

## Context

The §5.2 advisory matcher generates candidate pairs by **blocking** — only patient pairs
sharing some cheap key are scored, avoiding an O(n²) all-pairs comparison. Blocking lives
entirely in `matcher/src/cairn_matcher/pipeline/db.py` (`_GROUPS_SQL` +
`generate_candidate_pairs`); it is read-only Python-issued SQL, **not** an in-DB floor
(no `db/` file, no SCHEMA), because it is advisory and recall-oriented — the pure Python
scorer remains the source of truth for whether a generated pair is actually a match.

Today `_GROUPS_SQL` is a 3-pass `UNION ALL` disjunction, each pass emitting
`(pass_name, key, members)`:

1. **identifier** — patients sharing a `system:match_key` (excluding `system='unknown'`)
2. **dob** — patients sharing an *exact* DOB `value`
3. **name** — patients sharing a single lowercased whitespace-split name token

`generate_candidate_pairs(conn, *, max_block_size=100)` expands each group of size `k`
into its `C(k,2)` canonical `(low, high)` uuid pairs, **unless** the group exceeds
`max_block_size`, in which case the whole group is skipped and reported in
`skipped_blocks` (a block shared by hundreds of people is non-discriminating; the §5.13
hub duplicate-sweep is the declared backstop for what blocking drops).

**The problem this slice addresses:** a *single weak* key (a common surname token like
"smith", or a popular DOB) forms an over-broad block that exceeds the cap and is dropped
wholesale — taking every true-match pair inside it with it. The B3 eval harness already
measures this fallout (`pair_completeness`, `dropped_true_matches`, `dropped_pair_estimate`,
`skipped_blocks`), so the win is directly observable.

## Goal

Add a **compound blocking key** — name-token **+** birth-year — that combines two weak
signals into one more selective key, so an over-broad single-token block is partitioned
into per-birth-year sub-blocks that survive the cap and recover the true-match pairs that
would otherwise be dropped. As a bonus it catches a class of true matches the exact-DOB
pass structurally misses (see "Secondary recall gain").

This is the first of the measurement-driven B3 items unblocked by the eval harness
(the keystone merged in PR #83). It is intentionally minimal (YAGNI): one compound key,
proven and measured, before any further keys.

## Decisions (from brainstorming)

- **Additive, never replace.** The compound pass is a *new* `UNION ALL` branch alongside
  the existing three. Because `generate_candidate_pairs` deduplicates pairs by canonical
  uuid pair across all passes, the generated set is a **union** — so recall is **strictly
  non-decreasing** versus today. The only cost is a modest reduction-ratio dip when
  single-token blocks are already small (the compound pass then re-emits already-covered
  pairs, deduped away — no recall change, slightly more rows examined). Replacing the
  single-token pass was rejected: a true match where one record has no parseable DOB, or
  where birth-years differ by a typo, would then not be blocked together *at all* — a
  recall regression on exactly the messy records the matcher exists to catch.
- **Single compound key this slice.** Only `name-token + birth-year` (the key HANDOVER
  names). Additional compound keys (e.g. `dob + name-first-initial`,
  `name-token + sex-at-birth`) are deferred until this one is measured.
- **Mechanism proven by targeted small-cap DB tests**, not a volume fixture. The full
  synthetic-corruption / volume generator stays deferred (it is its own listed B3 item).

## Mechanism

A new CTE extracts a birth-year from the DOB projection, and a fourth pass joins it to the
existing `name_tokens` CTE:

```sql
WITH name_tokens AS (                          -- unchanged
    SELECT DISTINCT patient_id, token
    FROM patient_name, regexp_split_to_table(lower(value), '\s+') AS token
    WHERE token <> ''
),
birth_year AS (                                -- new
    SELECT patient_id, left(value, 4) AS year
    FROM patient_demographic
    WHERE field = 'dob' AND value ~ '^[0-9]{4}'   -- honest-degrade guard
)
SELECT 'identifier' ...                         -- unchanged
UNION ALL
SELECT 'dob' ...                                -- unchanged
UNION ALL
SELECT 'name' ...                               -- unchanged (single-token pass retained)
UNION ALL
SELECT 'name+year' AS pass_name,                -- new compound pass
       nt.token || '|' || byr.year AS key,
       array_agg(nt.patient_id) AS members
FROM name_tokens nt JOIN birth_year byr USING (patient_id)
GROUP BY nt.token, byr.year
HAVING count(DISTINCT nt.patient_id) >= 2
```

Each branch still returns the same `(pass_name, key, members)` row shape, so the uniform
oversized-block cap, the `skipped_blocks` reporting, and `_pairs_from_members` (the pure
canonical-pair expander) are **all untouched**. `name_tokens` is `DISTINCT (patient_id,
token)` and each patient has at most one `birth_year` row, so the join yields one row per
`(patient, token, year)` — `array_agg` produces no duplicate patient ids within a group.

### Honest degrade (culture-neutral, principle 4)

Birth-year is derived **only** when the stored DOB value begins with four digits
(`value ~ '^[0-9]{4}'`), taking `left(value, 4)`. This mirrors the pipeline adapter's
existing "non-ISO DOB → `None`" honest-degrade. There is **no date parsing**, no assumed
calendar or locale — just a leading-4-digit guard on the already-stored value. A record
with a null or non-ISO DOB simply does not appear in the `name+year` pass; it remains
covered by the single-token `name` pass, so its blocking coverage never regresses.

### Secondary recall gain

`left('1990', 4)` and `left('1990-05-12', 4)` both yield `'1990'`. So the `name+year`
pass groups together:

- **precision-mismatched** true matches — a year-precision DOB (`"1990"`) and a
  day-precision DOB (`"1990-05-12"`) for the same person — which the **exact-DOB** pass
  (`value = value`) never groups; and
- same-name, same-birth-year pairs whose **month/day differs** (a transcription error),
  again missed by exact-DOB.

These are genuine recall additions, not only block-shrinking.

## What does NOT change

- No `db/` floor file, **no SCHEMA bump** (advisory, read-only SQL issued by the matcher's
  Python; not the unbypassable in-DB floor).
- **No eval-harness change** — `blocking_eval.py` / `metrics.py` already measure the
  effect; they call the real `generate_candidate_pairs`.
- No change to `_pairs_from_members`, `generate_candidate_pairs`'s signature/return, the
  cap logic, `runner.py`, or `sweep.py`.
- No new dependency. No spec/ADR change (implements settled §5.2 / §5.13 / ADR-0014).
- `db.py` grows by ~8 SQL lines — stays well under the 500-line guideline.

## Testing (TDD — failing test first)

DB-gated matcher integration tests (require `CAIRN_TEST_PG`, PG18 + cairn_pgx; run with
`cd matcher && CAIRN_TEST_PG=… uv run --extra pipeline pytest`). Each red-first:

1. **Block-shrinking + rescue.** Seed a 3-member single name-token block ("smith")
   spanning two birth-years; run at `max_block_size=2`. Assert: the `name` pass block is
   oversized → appears in `skipped_blocks`; the `name+year` sub-blocks (size ≤ 2) survive
   → the same-name/same-year pair **is** generated (it would be dropped without the
   compound pass).
2. **Honest degrade — no recall regression.** A record with a null (and a non-ISO) DOB is
   absent from any `name+year` group but still grouped via the single-token `name` pass.
3. **Precision-mismatch rescue.** `"1990"` (year precision) and `"1990-05-12"` (day
   precision) under the same name token share a `name+year` block, though the exact-DOB
   pass does not group them.
4. **Additivity / regression.** Existing `generate_candidate_pairs` and
   `evaluate_blocking` tests stay green; on a dataset with no over-broad blocks the
   generated pair *set* is unchanged (compound pairs dedupe into the single-pass pairs).

The bundled `gold_v1.json` is intentionally tiny (a regression/tuning instrument, not a
statistical claim); these tests construct their own small seeded fixtures at a small
`max_block_size` to exercise the cap deterministically.

## Files touched

- `matcher/src/cairn_matcher/pipeline/db.py` — the `_GROUPS_SQL` string (new `birth_year`
  CTE + the `name+year` `UNION ALL` branch; in-comment rationale for the honest-degrade
  guard and the additive/recall-non-decreasing property).
- `matcher/tests/` — new/extended DB-gated integration tests (above).

## Known limitations / follow-ups (recorded, not lost)

- **The ISO-prefix guard (`'^[0-9]{4}'`) is an empirical bet.** It assumes a leading-4-digit
  year is the dominant stored DOB shape. Real-world adequacy across locales is unproven and
  is expected to be revisited once the harness runs against richer/real data — explicitly
  flagged by the user at design time. If a meaningful population stores non-leading-year
  DOBs, the guard silently excludes them from the compound pass (recall stays at the
  single-token-pass level for those — safe degrade, not a false group).
- **Quantitative before/after at volume** awaits the deferred **synthetic corruption /
  volume generator** (a separate B3 item). This slice proves the *mechanism* and is
  measurable on small seeded fixtures via the existing harness; it does not ship a
  volume benchmark.
- **Further compound keys** (`dob + name-first-initial`, `name-token + sex-at-birth`) are
  deferred until this key's metric impact is measured.

## House-rules check

- Pure/reusable: the only logic change is declarative SQL; the pure `_pairs_from_members`
  expander and the rest of the pipeline are reused verbatim.
- TDD: red-first integration tests drive the SQL change.
- Junior-legible inline docs: the new SQL branch and CTE carry comments explaining the
  honest-degrade guard, the additive (recall-non-decreasing) property, and the
  precision-mismatch rescue.
- ≤ 500 lines: `db.py` stays well under.
- No technical debt: the ISO-guard limitation is recorded above (and will be carried into
  HANDOVER), not silently absorbed.
