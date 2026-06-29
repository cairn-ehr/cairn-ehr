# Matcher eval harness (§5.2 matcher, piece B3 keystone) — design

**Date:** 2026-06-30 · **Status:** approved, ready for plan · **Spec home:** §5.2/§5.13 (advisory matcher),
ADR-0014. **No new ADR, no spec bump** — this is a measurement substrate for the advisory matcher; it
ships no clinical floor and makes no link decision.

## Why

Two of the most valuable remaining B3 items are explicitly **measurement-driven** and are guesswork without
a way to measure:

- **Compound blocking keys** (token+birth-year, to shrink oversized blocks) — you cannot justify a tighter
  blocking key without measuring whether it keeps **pair-completeness** (blocking recall) high while
  improving the **reduction ratio**.
- **Weight-learning** (the Fellegi–Sunter `m/u` weights and the conservative thresholds are shipped
  *illustrative* defaults) — you cannot tune weights without measuring precision/recall/F1 and the
  dangerous **auto-false-link rate** against labelled ground truth.

This slice builds the keystone both depend on: a labelled-dataset format and a harness that measures both
the **scorer/banding** decision and the **blocking** candidate generation. It is advisory and low-stakes
(a defect yields a wrong *metric a human reads*, never record corruption — the §9 fit-for-purpose tier).

## Scope (decided)

- **Full split**: a pure dataset/metrics/scorer-eval core that always runs (zero runtime deps), **plus** an
  optional DB-gated blocking-eval layer under the existing `pipeline` extra (`CAIRN_TEST_PG`), calling the
  **real** `db.generate_candidate_pairs`. No parallel pure-Python blocking mirror — the SQL is the source of
  truth and a second implementation would drift.
- **Gold fixture + loader**: one hand-authored, checked-in entity-cluster dataset + the JSON format and
  loader. Deterministic — doubles as a regression gate. A synthetic corruption generator is a **deferred**
  follow-on that emits the same format (out of scope here).
- **Thin CLI runner**: `python -m cairn_matcher.eval [dataset.json]` prints the report (scorer metrics
  always; blocking metrics when `CAIRN_TEST_PG` is set).

## Key alignment: one record shape, two consumers

A dataset record mirrors the **projection-row shape** the matcher already operates over, so both consumers
derive from a single shape with **zero parallel construction logic**:

- **Pure scorer eval** maps a record → `CandidateRecord` by delegating to the **existing pure adapter**
  (`pipeline/adapter.py::candidate_from_rows`, with `parse_dob`/`build_names`/`build_identifiers`). The
  scorer eval therefore measures the *real* adapter+scorer path, not a hand-built record.
- **DB blocking eval** maps a record → seeded `patient_*` rows, exactly as `tests/conftest.py::seed_patient`
  does today.

## Components & module layout

A new `cairn_matcher/eval/` sub-package beside `pipeline/`, mirroring the pure-core / optional-DB split.
Every module small and single-purpose (house rule #4: keep files well under 500 lines).

| Module | Purity | Purpose |
|---|---|---|
| `eval/__init__.py` | pure | package exports |
| `eval/dataset.py` | pure | value types (`DatasetRecord`, `EntityCluster`, `LabelledDataset`); `load_dataset(obj)`; `record_to_candidate(rec)` (delegates to `candidate_from_rows`); `truth_pairs(ds)` ground-truth derivation; pair counts |
| `eval/metrics.py` | pure | `Confusion` (truth × band) + `ScorerMetrics`; pure functions computing precision/recall/F1, auto-false-link rate, missed-match rate, score separation |
| `eval/scorer_eval.py` | pure | `evaluate_scorer(ds, *, weights, thresholds, config)` → `ScorerMetrics`; iterates pairs, builds (cached) `CandidateRecord` per record, `field_comparisons`→`score`→`band(..., vetoes=[])`, collects predictions |
| `eval/blocking_eval.py` | `pipeline` extra, DB-gated | `evaluate_blocking(conn, ds, *, max_block_size)` → `BlockingMetrics`; seeds `patient_*` (label→`uuid5`), calls real `generate_candidate_pairs`, reverse-maps uuid pairs → labels, computes metrics |
| `eval/report.py` | pure (formatting) | render `ScorerMetrics`/`BlockingMetrics` to a plain-text report string |
| `eval/__main__.py` | CLI | argparse; load dataset (bundled gold by default); run scorer eval always; run blocking eval iff `CAIRN_TEST_PG`; print report |
| `eval/fixtures/gold_v1.json` | data | hand-authored entity-cluster gold set |

## Dataset format

Ground truth as **entity clusters** — records grouped by the real person. Within-cluster unordered pairs are
true matches; cross-cluster pairs are true non-matches. This avoids enumerating O(n²) pair labels by hand.

```json
{
  "name": "gold_v1",
  "description": "...",
  "entities": [
    {
      "entity_id": "alex",
      "records": [
        {
          "record_id": "alex-1",
          "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 70},
          "sex_at_birth": {"value": "female", "provenance_rank": 70},
          "names": [{"value": "Alex Nguyen", "provenance_rank": 30}],
          "identifiers": [{"system": "au-medicare", "match_key": "12345", "value": "12345"}]
        }
      ]
    }
  ]
}
```

Field rules (all optional except `record_id`; absence is a safe, gradeable absence per principle 4):

- `record_id` — a unique **readable label** (string). The blocking layer derives a stable `uuid5` for
  seeding and reverse-maps generated uuid pairs back to labels. Must be globally unique across the dataset.
- `dob` — `{value: ISO string, precision: "year"|"month"|"day", provenance_rank: int}`. Maps to a
  `patient_demographic` dob row (`facets = {"precision": ...}`) and to `candidate_from_rows`' `dob_row`.
- `sex_at_birth` — `{value, provenance_rank}`. Maps to a `patient_demographic` sex-at-birth row / `sex_row`.
- `names` — list of `{value: display string, provenance_rank}`. Maps to `patient_name` rows / `name_rows`.
- `identifiers` — list of `{system, match_key, value}`. Maps to `patient_identifier` rows / `identifier_rows`.

The gold fixture deliberately spans **non-Western name shapes** (mononyms, patronymics, multi-token
given/family, transliteration variants), typos/edit-distance variants, missing fields, and provenance
differences — keeping the eval honest and anti-cultural-capture (ADR-0014).

## Metrics

### Ground truth (pure)
From entity clusters: `true_match_pairs` = all within-cluster unordered pairs; `total_pairs` = C(N, 2);
`true_match_count` = Σ C(cluster_size, 2); `true_nonmatch_count` = total − true. Pairs are identified by the
canonical (lower, higher) ordering of the two `record_id` labels (string order in label space; the blocking
layer maps to uuid order separately and reverse-maps back).

### Scorer metrics (pure)
For each unordered pair over the universe: build `CandidateRecord` for each record (cached per record),
`field_comparisons(a, b, config)` → `score(.., weights)` → `band(score, vetoes=[], thresholds)`. Two
operating points:

- **strict**: positive iff `AUTO_CANDIDATE`
- **lenient**: positive iff `AUTO_CANDIDATE` or `REVIEW`

`Confusion` is the 2×3 table (truth ∈ {match, nonmatch} × band ∈ {auto, review, none}). `ScorerMetrics`
derives:

- precision / recall / F1 at strict and lenient operating points
- **auto_false_link_rate** = nonmatch pairs banded `auto` / all pairs banded `auto` (the dangerous metric;
  must be ~0 on a sane config)
- **missed_match_rate** = match pairs banded `none` / all true-match pairs
- per-class **score separation**: min/median/max score for matches vs non-matches (shows the overlap a
  weight change must reduce)

**Zero-denominator convention** (explicit, to remove ambiguity): precision = 0.0 when nothing is predicted
positive; recall = 0.0 when there are no true matches; F1 = 0.0 when precision + recall == 0; the rates
(auto_false_link, missed_match) = 0.0 when their denominator is 0. No NaNs are returned.

### Blocking metrics (DB-gated)
Seed all records, call `generate_candidate_pairs(conn, max_block_size=...)` → `(uuid_pairs, skipped)`,
reverse-map to label pairs. `BlockingMetrics`:

- **pair_completeness** (blocking recall) = |generated ∩ true_match_pairs| / |true_match_pairs|
- **reduction_ratio** = 1 − |generated| / total_pairs
- **block size**: count + sizes of skipped oversized blocks; **dropped_pair_estimate** = Σ C(size, 2) over
  skipped blocks (folds in the separately-deferred B3 telemetry item)
- **dropped_true_matches**: the explicit list of true-match pairs blocking did **not** generate — the
  actionable output for compound-key work (these can never be matched downstream)

## CLI

`python -m cairn_matcher.eval [dataset.json]`:

- no path → run the bundled `eval/fixtures/gold_v1.json`
- always runs scorer eval (pure) and prints the scorer report
- if `CAIRN_TEST_PG` is set, also opens a connection, runs blocking eval, prints the blocking report
- exit non-zero on a malformed dataset; the report explicitly states that fixture metrics are a
  regression/tuning instrument, not a statistical accuracy claim (see caveats)

## Testing (TDD, failing test first)

Pure tests (run everywhere via `uv run pytest`):

- `test_eval_dataset.py` — load/round-trip; `record_to_candidate` agrees with a directly-built record;
  `truth_pairs` derivation (within-cluster only; counts; canonical ordering); malformed input raises.
- `test_eval_metrics.py` — precision/recall/F1/rates computed correctly on hand-built confusions, incl.
  the zero-denominator convention below.
- `test_eval_scorer.py` — over the gold fixture: a strong same-person pair → `auto`; a typo/edit-distance
  same-person pair → `review`; a different-person pair → `none`; invariant **auto_false_link_rate == 0** on
  the gold set; metrics are internally consistent.

DB-gated tests (`CAIRN_TEST_PG`, skipped cleanly without it — the existing conftest pattern):

- `test_eval_blocking.py` — over the gold fixture: **pair_completeness == 1.0** (every true match survives
  blocking) and **reduction_ratio > 0**; `dropped_true_matches == []` on the gold set; a constructed
  oversized block is reported with a non-zero `dropped_pair_estimate`.

## Dependencies & house rules

- **No new runtime dependency.** Pure core: stdlib only (`json`, `uuid`, `statistics`, `dataclasses`).
  Blocking layer: `psycopg`, already the optional `pipeline` extra (LGPL → AGPL-compatible). The pure core
  never imports psycopg.
- AGPL-3.0; TDD; junior-legible inline docs; pure functions in small modules (house rules #1–#4).
- Reuses (does not fork) `candidate_from_rows`, `generate_candidate_pairs`, `field_comparisons`, `score`,
  `band` — the eval measures the real path.

## Honest caveats (flagged, not hidden)

1. **Pure scorer eval runs without vetoes** (`band(.., vetoes=[])`) — it measures scorer+threshold quality in
   isolation; the real banded outcome can cap a high score at `review` on a DB veto. Pure metrics are thus
   *slightly optimistic* vs end-to-end. A veto-aware DB mode is a later, additive extension.
2. **O(N²)** over all pairs — fine for the small gold set; will not scale. That is *why* blocking eval
   exists. An "end-to-end" mode (scorer over only blocking-generated pairs) is a noted future extension, not
   built here.
3. **The gold fixture is small and hand-authored** — its metrics are a **regression gate and tuning
   instrument, not a statistical accuracy claim**. The report says so. Statistically meaningful volume comes
   from the deferred synthetic generator.

## Out of scope (deferred, recorded so it is not lost)

- Synthetic corruption generator (volume + recall curves) — a follow-on emitting the same dataset format.
- Compound blocking keys themselves (this harness *measures* them; it does not change the SQL).
- Weight-learning itself (this harness *measures* a weight set; the learning loop is a separate slice).
- Veto-aware / end-to-end scorer mode; locale comparator packs; `compare_address`; full §7.5 matcher actor
  registration; hub-tier aggressive duplicate-sweep + proposal retraction.
