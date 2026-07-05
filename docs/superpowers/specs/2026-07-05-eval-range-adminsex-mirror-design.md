# Design — B3 eval mirror: generator range-DOB emission + administrative-sex representation

**Date:** 2026-07-05 · **Scope:** advisory Python only (`matcher/`) — no `db/` floor change, no SCHEMA
bump, no event-type change, no ADR/spec edit. Implements the deferred item recorded in HANDOVER after
§5.4 slices B–D: *"generator range-DOB emission + range-aware eval mirror, which must also mirror
`administrative_sex`."*

## 1. Problem

The shipped matcher moved twice since the B3 eval harness was built, and the harness did not follow:

1. **Year-range DOBs** (§5.4 slice B/C): clinician-estimated age is carried as a birth-year window
   (`value "<yyyy>/<yyyy>"`, precision `"year-range"`), blocked by two ANCHORED passes
   (`dob-range`, `dob-range+sex` in `_RANGE_GROUPS_SQL`) and scored by range-aware positive-only
   `compare_dob`. The eval **generator never emits a range DOB** and the generator's recoverability
   predicate `shares_blocking_key` **deliberately does not mirror the range passes** (recorded safe
   deferral: it only under-claims, so `_repair` over-repairs via a name token).
2. **Composite sex** (§5.4 slice D): scoring reads *both* `sex-at-birth` and `administrative-sex`
   facets into a composite `SexValue` (positive-only union fallback). The eval `DatasetRecord`
   **cannot represent `administrative_sex` at all**, so the pure scorer eval can never exercise the
   fallback path.

Consequence: **B3 weight-learning is blocked.** Sweeping `evaluate_scorer` weights today would train
on a field set the shipped matcher no longer has (no composite-sex pairs, no range-DOB pairs), and
blocking A/B measurements over the range passes have no synthetic data to measure.

One live defect found during design, fixed by this slice: `shares_blocking_key`'s exact-DOB branch
compares raw `value` strings with no precision guard. Two identical *year-range* values would claim
an exact-DOB block — but `_GROUPS_SQL`'s exact-`dob` arm **excludes** `year-range` rows
(`IS DISTINCT FROM 'year-range'`, the PR #131 A/B-purity fix). An over-claiming mirror is the
dangerous direction: `_repair` would skip a pair the SQL cannot recover, silently breaking the
volume set's recoverable-by-construction guarantee.

## 2. Non-goals

- Weight-learning itself (the next slice; this one unblocks it).
- Gold-set (`gold_v1.json`) additions — the hand-curated culture-plural set stays as is; synthetic
  volume data carries the new fields.
- Variable cluster size / hard negatives / unrecoverable fraction (recorded earlier deferrals).
- Any change to `pipeline/db.py` SQL, comparators, orchestrator, or scoring — the production path
  is the fixed target the eval mirrors.
- CLI knob plumbing for the new operator probability (GenSpec is constructible in code; the
  `generate` CLI keeps its current flags — additive follow-up if a sweep needs it).

## 3. Design

Four small parts, all in `matcher/`; every part additive.

### 3.1 `DatasetRecord.administrative_sex` (dataset.py, blocking_eval.py)

- New optional field `administrative_sex: Mapping | None = None`, same
  `{"value": str, "provenance_rank": int}` shape as `sex_at_birth`; `_record_from` passes it
  through. Docstring documents the dob `"year-range"` precision + `"<min>/<max>"` value shape
  (the loader already accepts it — dob is an opaque mapping).
- `record_to_candidate` builds an `admin_sex_row` and passes it to `candidate_from_rows`
  (parameter already exists since slice D) — the composite-sex fallback then rides the REAL
  adapter/orchestrator path with zero parallel logic.
- `seed_dataset` inserts an `administrative-sex` row into `patient_demographic` when present
  (same INSERT shape as `sex-at-birth`). Year-range dobs need **no seeding change**: the dob INSERT
  already writes `value` + `facets.precision` verbatim, which is all `_RANGE_GROUPS_SQL` reads.

### 3.2 Generator: the estimated-age operator (generator.py)

New corruption operator `corrupt_dob_estimate(record, rng)` — "this clone is the §5.4
estimated-age registration of the same person":

- Extract the birth year as the **first 4-digit run** of the current dob value (mirroring the SQL
  point-branch discipline); no run → no-op (safe degrade, like every operator).
- Replace the dob with an inclusive window containing that year:
  `tol = rng.randint(2, 5)` → `value f"{year-tol:04d}/{year+tol:04d}"`, `precision "year-range"`,
  `provenance_rank 30` (clinician-observed, per slice B).
- Move sex to the observed facet: drop `sex_at_birth`, set
  `administrative_sex = {"value": <seed sab value if present, else rng male/female>,
  "provenance_rank": 30}` — a clinician observes apparent sex; they cannot know the birth fact
  (slice B's reasoning). This manufactures exactly the sab-vs-admin pairs the composite-sex
  fallback needs for training.
- New `GenSpec` knob `p_dob_estimate: float = 0.15`, appended **last** in `_OPERATORS` so it
  replaces whatever dob the earlier operators left (an estimated-age record supersedes format/typo
  corruption wholesale; a typo'd year windows around the typo — honest corruption, the window may
  or may not still overlap).
- Determinism note: adding an operator changes RNG consumption, so the same `GenSpec.seed`
  produces a different byte stream than before this slice. "Deterministic given a seed" is a
  reproducibility contract, not a cross-version stability contract (the dataset `name` embeds no
  version); recorded here deliberately.

### 3.3 Range-aware `shares_blocking_key` (generator.py)

New pure helper `_birth_window(record) -> (y_min, y_max, is_range) | None`, mirroring
`_RANGE_GROUPS_SQL`'s `birth_window` CTE exactly:

- range rows: precision `"year-range"` AND value matches `^\d{4}/\d{4}$` AND `min <= max`
  → `(min, max, True)`; malformed/inverted → **excluded** (mirrors the SQL's safe degrade).
- point rows: precision ≠ `"year-range"`, first 4-digit run `y` → `(y, y, False)`; no run → excluded.

`shares_blocking_key` gains the range branch: windows overlap AND at least one side `is_range`
→ True (the ANCHORED semantics — point↔point windows never key, exactly as `window_overlap`
requires `a.is_range`). The `dob-range+sex` pass is a subset of `dob-range`'s pair set (same
`window_overlap`, intersected with shared sex), so the recoverability predicate needs only the
plain overlap branch — the +sex rescue only matters under cap-skips, which the predicate has
never modelled (same stance as the symmetric passes).

Fix the exact-DOB branch: value equality only counts when **neither side declares precision
`"year-range"`** (mirrors `IS DISTINCT FROM 'year-range'`; kills the over-claim in §1).

### 3.4 Guards: drift canary + DB-gated recoverability proof

- `test_eval_generator_sync.py` extends the `_MIRRORED_PASSES` table to `_RANGE_GROUPS_SQL`
  fragments (each entry now names its SQL constant): the `year-range` precision filter, the
  `^([0-9]{4})/` extraction, the overlap join predicate, the `WHERE a.is_range` anchor guard, and
  the exact-arm exclusion fragment `IS DISTINCT FROM 'year-range'` in `_GROUPS_SQL` (the fix in
  §3.3 leans on it).
- DB-gated volume test: a second run with the estimate + name operators forced high
  (`p_dob_estimate=0.9, p_name=0.9`) must still measure `pair_completeness == 1.0` under a large
  cap — the end-to-end proof that the mirror never over-claims what the real SQL recovers (the
  new range branch is load-bearing there: with names corrupted and `_repair` standing down, only
  the `dob-range` pass can carry those pairs).

## 4. Testing (TDD, red-first)

Pure (no DB):
- `_birth_window`: range parse, malformed/inverted exclusion, point first-4-digit-run, no-run → None.
- `corrupt_dob_estimate`: window contains the seed year; sex moved sab→admin (both the copy and
  the random-draw arm, rng-forced); no-op without a 4-digit run; purity (input never mutated).
- `shares_blocking_key`: range↔point overlap True; range↔point disjoint False; range↔range overlap
  True; point↔point same-year False via the range branch (still True via exact/name if those keys
  exist); **identical malformed year-range values → False** (the §1 over-claim fix);
  `_repair` does NOT append a name when the window overlap already carries the pair.
- `DatasetRecord`: `administrative_sex` loads, round-trips, and `record_to_candidate` maps it to
  `CandidateRecord.administrative_sex` (composite fallback then grades via the real orchestrator —
  asserted with one `field_comparisons` smoke over an sab-vs-admin pair).
- Generator end-to-end: a high-`p_dob_estimate` dataset round-trips `load_dataset`; estimate clones
  carry `year-range` dob + `administrative_sex` and no `sex_at_birth`.
- Drift canary extensions (§3.4).

DB-gated (CAIRN_TEST_PG):
- `seed_dataset` writes the `administrative-sex` row; a seeded range↔point pair with shared sex is
  generated by the real `dob-range+sex` pass (asserting the seeding is visible to the sex CTE).
- The forced-high volume run of §3.4.

Suites green (pure + DB-gated + ruff) before commit, per house rules.

## 5. Honest limits (recorded)

- The mirror still ignores the block-size cap (as it always has for symmetric passes): a
  cap-skipped block can drop a "recoverable" pair on a small `max_block_size`; the volume tests
  run under a large cap and `evaluate_blocking` reports skips honestly.
- `blocking_sex`'s Unicode-whitespace approximation (SQL btrim set ⊂ Python strip) is inherited,
  not widened, by seeding admin-sex rows.
- The generator's estimate operator windows around a possibly-typo'd year when `p_dob_typo` fired
  first — the pair may then be honestly unrecoverable by the range key alone and `_repair` restores
  a name token (the safe direction, unchanged).
- Same-seed outputs differ from pre-slice outputs (§3.2 determinism note).
