# Wire `repaired_record_ids` into eval lift reporting — #290

**Date:** 2026-07-25
**Tier:** advisory Python (§9 fit-for-purpose), offline eval tooling. Follow-on to #211 gap 4.
**Scope:** `matcher/` only. No spec/ADR/SCHEMA/wire/DB change.
**Paper-parity (§1.2):** not clinical-surface — offline eval tooling.

## Problem

#211 gap 4 added the *quantify seam*: the synthetic generator's `_repair` marks a clone
`repaired: True` when it injected a VERBATIM seed name to keep a corrupted pair blockable
(`eval/generator._repair`), `DatasetRecord.repaired` carries the flag through the loader, and
`eval/dataset.repaired_record_ids(ds)` enumerates the marked records. The effect is now
*measurable* — but no eval consumer *consumes* it.

So `eval/scorer_eval` (via `metrics`), `eval/crossval.kfold_lift`, and `eval/learner.learn_model`
still treat a repaired pair like any other. A repaired clone carries a verbatim EXACT name on
exactly the hardest true pair — the one every other blocking key already failed to recover — so it
grades EXACT, is trivially recovered, and **inflates held-out recall / F1** on precisely the pairs
that should be hard. Left invisible, the headline synthetic numbers read as an accuracy claim they
have not earned.

## Decision

**Report the count everywhere (approach b); never exclude, never alter the learned model.**

A repaired true-pair is a *genuine* match the matcher *should* recover — the generator injected the
name only because corruption destroyed the other keys, not because the pair is fake. Silently
dropping such pairs from the headline (approach a) would make recall look honest while *understating
coverage* and hiding that these are the hardest pairs. Reporting the count keeps every pair in the
metric and lets a reader discount — the disclose-don't-hide ethos `crossval.py` already states
("reporting train-set lift as generalization would be a precise untruth"). Learned weights are a PoC
model output and must not shift as a side effect, so the learner reports its repaired training count
but does **not** exclude those pairs.

Every added count is a pure additive field. On a real (unmarked) dataset every one is `0` — the
honest "nothing to discount."

## Design

### Shared pure primitive — `eval/dataset.py`

```
repaired_truth_pairs(ds) -> frozenset[tuple[str, str]]
```

The true-match pairs with at least one endpoint in `repaired_record_ids(ds)`. Reuses `truth_pairs`
and `repaired_record_ids`; the single home for "which true pairs did the repair ease." Every consumer
derives from it. Empty on a real dataset. Pure, no I/O.

### Consumer 1 — `eval/metrics.py` + `eval/scorer_eval.py`

- `PairOutcome` gains `repaired: bool = False`. Structural fact: "an endpoint of this pair is a
  synthetically-repaired record." The default keeps every existing construction and every real
  dataset valid.
- `scorer_eval.scorer_outcomes(ds)` computes `repaired_ids = repaired_record_ids(ds)` once and sets
  `repaired = low in repaired_ids or high in repaired_ids` on each outcome.
- `ScorerMetrics` gains `repaired_match_pairs: int` = the number of outcomes that are
  `is_match and repaired`, computed in `scorer_metrics`. (Only a true within-cluster pair is eased by
  the injection; the metric intersects `repaired` with `is_match`, so a repaired record's cross-cluster
  non-match pairs never count.)

Because `kfold_lift` pools `PairOutcome`s and calls `scorer_metrics`, the held-out repaired count flows
through the before/after blocks for free.

### Consumer 2 — `eval/crossval.py`

- `LiftReport` gains `held_out_repaired_pairs: int`. before and after measure the *identical* held-out
  pairs, so `before.repaired_match_pairs == after.repaired_match_pairs`; the field is set from that
  pooled value and reflects only measured (non-skipped) folds.

### Consumer 3 — `eval/learner.py` + `eval/model_io.py`

- `LearnMetadata` gains `train_repaired_pairs: int` = `len(repaired_truth_pairs(ds))` (in `learn_model`
  every true pair is a training pair). Reported, not excluded — weights untouched.
- `model_io._META_FIELDS` adds `train_repaired_pairs` so metadata round-trips through
  `write_model` / `read_model`. **Operational note:** this makes the field required in model JSON, so
  any hand-saved pre-#290 `learned.json` desk artifact must be regenerated (consistent with the
  module's fail-loud philosophy; none are committed).

### Reporting — `eval/report.py`

- `format_scorer`: add `  repaired match pairs (synthetically eased): N of M` (M = total true-match
  pairs = `match_auto + match_review + match_none`) plus a one-line note that recall/F1 above are
  optimistic by up to N pairs. `N = 0` on a real dataset.
- `format_lift`: surface `held_out_repaired_pairs` in the report.

## Tests (TDD, RED-first, all in the dependency-free `matcher` suite)

Wiring tests use hand-crafted datasets with explicit `repaired: True` markers — deterministic. (The
generator only repairs when *all* blocking keys are destroyed, so relying on it to produce a repaired
record would be flaky.)

- `dataset`: `repaired_truth_pairs` returns the eased pairs on a marked set; empty on an unmarked set.
- `metrics`: `PairOutcome.repaired` defaults False; `scorer_metrics` counts `repaired_match_pairs` — a
  repaired *non-match* is excluded, a repaired *match* is counted.
- `scorer_eval`: `evaluate_scorer(...).repaired_match_pairs == len(repaired_truth_pairs(ds))` on a
  marked set; `== 0` on the existing hand set (`test_eval_scorer_driver._DS`).
- `crossval`: `kfold_lift(...).held_out_repaired_pairs` equals the pooled measured repaired count and
  before/after agree.
- `learner`: `learn_model(ds).metadata.train_repaired_pairs == len(repaired_truth_pairs(ds))`;
  `model_io` write→read round-trips it.
- `report`: `format_scorer` / `format_lift` include the repaired line(s).

## Out of scope (YAGNI)

- No exclusion mode / `--exclude-repaired` CLI flag (approach a) — the decision is to report only.
- No change to the learner's weights or thresholds.
- No new event types, DB migrations, or spec/ADR edits.
