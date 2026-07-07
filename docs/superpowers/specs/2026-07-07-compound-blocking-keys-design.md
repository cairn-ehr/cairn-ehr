# Compound blocking keys — `dob+first-initial` and `name+sex`

**Date:** 2026-07-07 · **Matcher slice:** 25 · **Tier:** advisory (§5.2/§5.13 fit-for-purpose)

## Problem

The §5.2 advisory matcher's candidate generation (`pipeline/db.py`, `pipeline/blocking.py`)
runs six blocking passes today: four SYMMETRIC group passes (`identifier`, exact-`dob`,
`name` token, `name+year`) and two ANCHORED birth-year-range passes (`dob-range`,
`dob-range+sex`). Blocking is recall-oriented and additive: pairs are deduped by canonical
uuid pair across passes, so a pass can only ever RAISE recall.

Two record-linkage-standard compound keys are not yet covered, and each closes a real gap:

1. **A shared name TOKEN is required to group two charts by name** (`name`, `name+year`).
   A misspelling, transposition, or diacritic difference ("Jon"↔"John", "Müller"↔"Muller")
   yields no shared token, so a true duplicate born the same year is never even grouped.
2. **A common name token can blow past the block-size cap** and be dropped wholesale. This
   is acute in cultures with heavily unisex given names, where a single token is shared by a
   large cohort — precisely the population where a `name` block is oversized and skipped.

## Goal

Add two **additive, symmetric** compound blocking passes:

- **`dob+first-initial`** — birth-year + first initial of a name token. A first-initial
  RELAXATION of the name requirement: catches true matches that share a first letter and a
  birth year but NO full name token. Genuinely new recall over the whole current pass set.
- **`name+sex`** — name token + normalized sex. A subset of the `name` block when uncapped;
  its value is the OVERSIZED-block rescue: it splits a common unisex-token `name` block that
  exceeded the cap into per-sex sub-blocks that fit. Also the rescue that still works in the
  §5.4 John-Doe population, where DOB is often a range or absent (so `name+year` cannot fire)
  but an observed administrative-sex is present.

**Non-goals / no change to:** the in-DB veto floor (`db/016`), any `db/` migration, the
`match_proposal` schema, any event type, any ADR, the spec. This is advisory eval/matcher
code only — the same footprint as slices 21–23.

## The additive-only invariant (why this is safe)

Every pass's output is UNIONed into one canonical-pair set and deduped. A new pass can only
add pairs the Python scorer then evaluates and (if unconvincing) rejects; it can never
suppress a pair another pass found, and never auto-links anything (banding + the veto floor
own that). So erring toward more grouping is safe by construction — the standing rationale
for `name+year` and `dob-range+sex`, and for these two.

## Pass semantics

Both new passes are SYMMETRIC (every within-group pair, `C(s,2)`), live in `_GROUPS_SQL`,
and reuse existing CTE logic verbatim — no new normalization rule is introduced.

### `dob+first-initial`

- **Key:** `initial || '|' || year` (e.g. `j|1990`).
- **`year`:** the existing `birth_year` CTE — the FIRST 4-consecutive-digit run of the stored
  `dob` value (`substring(value FROM '[0-9]{4}')`, guarded by `value ~ '[0-9]{4}'`), with
  `year-range` precision rows EXCLUDED. Culture-neutral, parses no date, assumes no calendar
  (principle 4) — identical to `name+year`'s year source.
- **`initial`:** the first character of each name token from the existing `name_tokens` CTE
  (already `lower(normalize(value, NFC))`, whitespace-split, callsign/placeholder-excluded),
  via `substring(token FROM 1 FOR 1)`. `substring ... FROM 1 FOR 1` is CHARACTER-wise in
  PostgreSQL, so it returns the first code point after NFC, not the first byte. One
  (patient, initial, year) row per token the chart carries.
- **Grouping:** `GROUP BY initial, year HAVING count(DISTINCT patient_id) >= 2`.
- **Why it adds recall:** it requires only a shared first letter + birth year, so two charts
  with the same birth year and same initial but DIFFERENT name tokens group — which no
  existing pass does. This is the pass that catches `corrupt_name` transpose/diacritic clones.

### `name+sex`

- **Key:** `token || '|' || sex` (e.g. `smith|female`).
- **`token`:** the existing `name_tokens` CTE.
- **`sex`:** the existing `blocking_sex` normalization — the UNION of a chart's `sex-at-birth`
  and `administrative-sex` values, uncertainty-sentinels excluded (bound from
  `adapter.VALUE_SENTINELS_PARAM`, the same set the Python scorer treats as absent-value),
  trimmed and lowered. Recall-first union: a trans patient whose administrative-sex matches
  an observation still groups even though sex-at-birth differs.
- **Grouping:** `GROUP BY token, sex HAVING count(DISTINCT patient_id) >= 2`.
- **Why it adds recall:** a `name+sex` block is a strict SUBSET of the `name` block, so with
  the cap OFF it adds zero pairs. Its contribution is the CAPPED case: when a common
  unisex-token `name` block exceeds `max_block_size` and is skipped wholesale,
  `name+sex` splits it on sex (~2–3 values) into sub-blocks that fit and are generated.

## SQL restructure (extract shared CTE fragments)

The `blocking_sex` normalization currently lives ONLY inside `_RANGE_GROUPS_SQL`. `name+sex`
is symmetric and belongs in `_GROUPS_SQL`, so both statements now need it. Rather than
duplicate a load-bearing, sentinel-bound normalization (the module was bitten once by a
hand-mirrored sex literal lagging the adapter — see the `blocking_sex` comment), extract the
shared CTEs into composable module-level SQL-fragment constants:

- `_NAME_TOKENS_CTE` — the `name_tokens` CTE (binds the placeholder-use exclusion `%s`).
- `_BIRTH_YEAR_CTE` — the point-DOB `birth_year` CTE (no bind param).
- `_BLOCKING_SEX_CTE` — the `blocking_sex` CTE (binds the value-sentinel exclusion `%s`).

`_GROUPS_SQL` is composed from all three plus its six arms (identifier, dob, name,
`name+year`, `dob+first-initial`, `name+sex`) and now binds TWO parameters in a fixed,
documented order: `(_PLACEHOLDER_USES_PARAM, VALUE_SENTINELS_PARAM)` — placeholder-uses
first (it appears first, in `_NAME_TOKENS_CTE`), value-sentinels second (in
`_BLOCKING_SEX_CTE`). `_RANGE_GROUPS_SQL` is composed from `birth_window` +
`_BLOCKING_SEX_CTE`, binding the single value-sentinel param as before. One definition per
CTE; the bind order is asserted by the DB-gated tests actually running each statement.

## Pure registry & code (`pipeline/blocking.py`)

- `ALL_PASSES` becomes the eight names, in execution order:
  `("identifier", "dob", "name", "name+year", "dob+first-initial", "name+sex", "dob-range", "dob-range+sex")`.
- Both new passes join `SYMMETRIC_PASSES` (derived as `ALL_PASSES - ANCHORED_PASSES`, so it
  updates automatically; `ANCHORED_PASSES` is unchanged).
- `dropped_pair_estimate`, `require_registered`, `pairs_from_anchor`, `canonical_pair`,
  `resolve_enabled_passes`: **unchanged** — both new passes take the existing symmetric
  `C(s,2)` branch and are validated against `SYMMETRIC_PASSES` like every other group pass.

## Eval mirror (`eval/generator.py`)

The synthetic-eval recoverability model (`shares_blocking_key`) must represent the passes it
can (it models the UNCAPPED world — the block-size cap is deliberately not simulated;
`evaluate_blocking` reports skips separately).

- New pure helper `_first_initials(record) -> set[str]` — the first character of each of the
  record's name tokens (mirrors `name_tokens` + `substring(...,1,1)`), NFC-lowercased.
- `shares_blocking_key` gains ONE clause: the records share a first initial AND a shared
  POINT birth-year (reuse the `_birth_window` point branch / `_first_year`; `year-range` rows
  are excluded, faithful to the SQL's `birth_year` CTE). Ordering/short-circuit does not
  matter (any True wins).
- `name+sex` adds NO clause — it is subsumed by the existing shared-name-token check in the
  uncapped model (a `name+sex` pair is always a `name` pair), documented exactly as
  `name+year` already is ("subsumed by the name-token check").

## Testing (TDD, red first)

**Pure — `tests/test_blocking_passes.py`:**
- `ALL_PASSES` equals the eight names (replaces the "six known names" assertion).
- The shape-set partition still holds (`SYMMETRIC ∪ ANCHORED == ALL`, disjoint;
  `ANCHORED == {dob-range, dob-range+sex}`), which now also asserts both new names landed in
  `SYMMETRIC_PASSES`.
- Existing anchored-pair / `dropped_pair_estimate` / `require_registered` tests unchanged.

**Pure eval — `tests/test_eval_*` (generator/truth):**
- `shares_blocking_key`: same-initial + same-point-year + no-shared-token → True (the new
  clause fires); different initial, same year → False; same initial but one side a `year-range`
  DOB → the new clause does NOT fire (still may be True via a range/name clause, so test on a
  record pair that shares nothing else).
- `_first_initials` returns the set of token initials; empty for a nameless record.

**DB-gated — `tests/test_candidate_generation.py` (+ siblings):**
- `dob+first-initial` rescue: two charts, same birth year, same first initial, DIFFERENT name
  tokens. With `enabled_passes={"dob","name","name+year"}` the pair is ABSENT; adding
  `"dob+first-initial"` makes it PRESENT. Proves genuinely new recall.
- `name+sex` targeted rescue (the capped case the aggregate metric can't show): insert a
  unisex token shared by `> max_block_size` charts of mixed sex. Run
  `generate_candidate_pairs` with a small `max_block_size`; assert `name` appears in
  `skipped_blocks` (oversized, dropped wholesale) while `name+sex` yields the same-sex
  sub-block pairs. Proves the rescue directly.
- Regression: with ALL passes enabled, the pair set is a SUPERSET of the pre-slice set on the
  same fixture (additive-only holds end to end).

## Honest limits (recorded, not engineered away)

- **`name+sex` is invisible to the uncapped blocking-recall metric.** It adds no pairs in the
  model `evaluate_blocking`/`shares_blocking_key` represent; its value is proven only by the
  targeted capped DB test, not by the aggregate number. This is deliberate: modelling the cap
  in the eval harness is a larger, riskier change, out of scope for this slice.
- **`first-initial` is code-point 1 after NFC.** A grapheme cluster whose combining mark did
  not precompose collapses to its base letter. Acceptable — the pass is advisory and
  recall-only; a mis-taken initial only ever feeds the scorer a few extra pairs it rejects.
- **`dob+first-initial` blocks can be large** (~1/26 of a birth-year cohort). Bounded by the
  same `max_block_size` cap + `skipped_blocks` reporting + the §5.13 hub duplicate-sweep
  backstop as every other pass.
- **Lift is measured on synthetic data only.** Real-world magnitudes await the large
  hand-crafted gold set (the deferred slice-24 follow-on); this slice ships the mechanism and
  a synthetic demonstration of `dob+first-initial`'s lift.

## Files touched

- `matcher/src/cairn_matcher/pipeline/blocking.py` — `ALL_PASSES` (+2 names).
- `matcher/src/cairn_matcher/pipeline/db.py` — extract `_NAME_TOKENS_CTE`/`_BIRTH_YEAR_CTE`/
  `_BLOCKING_SEX_CTE`; recompose `_GROUPS_SQL` (+2 arms, +1 bind param) and `_RANGE_GROUPS_SQL`.
- `matcher/src/cairn_matcher/eval/generator.py` — `_first_initials` + one `shares_blocking_key`
  clause.
- `matcher/tests/test_blocking_passes.py`, `matcher/tests/test_candidate_generation.py`,
  the pure eval tests — as above.
