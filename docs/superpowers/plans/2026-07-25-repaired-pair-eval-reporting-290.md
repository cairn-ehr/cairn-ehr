# Repaired-Pair Eval Reporting (#290) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consume the #211.4 `repaired_record_ids` seam by reporting, in every eval consumer, how many measured/trained true-match pairs were synthetically eased by a verbatim-name repair — so a reader discounts the optimistic recall/F1.

**Architecture:** One shared pure primitive (`repaired_truth_pairs`) in `eval/dataset.py`; additive-only count fields on `PairOutcome`/`ScorerMetrics` (measurement, flows through k-fold pooling for free), `LiftReport` (held-out), and `LearnMetadata` (training, round-tripped through `model_io`); rendered in `report.py`. No pairs dropped, no learned weights changed.

**Tech Stack:** Python 3.12, pure stdlib + the existing `cairn_matcher` package; `uv` for the env; `pytest` + `ruff`. The whole plan lives in `matcher/` (advisory §9 tier).

## Global Constraints

- **Tier:** advisory Python (§9 fit-for-purpose). `matcher/` only — **no** spec/ADR/SCHEMA/wire/DB change.
- **AGPL-3.0**; zero new runtime dependencies (the matcher core is dependency-free).
- **TDD**, RED before GREEN. Pure functions, explicit inputs/outputs.
- **Paper-parity (§1.2):** not clinical-surface — offline eval tooling. (No plan section owed; noted here per house rule 7.)
- **Run tests from `matcher/`:** pure suite is `cd matcher && uv run pytest` (never venv/pip). Lint: `cd matcher && uv run ruff check`.
- Every new count field is `int`; on a real (unmarked) dataset it is `0` — the honest "nothing to discount."

---

### Task 1: `repaired_truth_pairs` primitive

**Files:**
- Modify: `matcher/src/cairn_matcher/eval/dataset.py` (add function after `repaired_record_ids`)
- Test: `matcher/tests/test_eval_dataset.py`

**Interfaces:**
- Consumes: existing `truth_pairs(ds)`, `repaired_record_ids(ds)`, `canonical_label_pair(a, b)` (same module).
- Produces: `repaired_truth_pairs(ds: LabelledDataset) -> frozenset[tuple[str, str]]` — the true-match pairs with ≥1 endpoint in `repaired_record_ids(ds)`. Consumed by Tasks 5 and (indirectly, for cross-checks) the tests of Tasks 3–4.

- [ ] **Step 1: Write the failing tests** (append to `test_eval_dataset.py`, after the existing `test_repaired_record_ids_enumerates_only_marked_records`)

```python
def test_repaired_truth_pairs_selects_only_true_pairs_touching_a_repaired_record():
    ds = load_dataset({
        "entities": [
            {"entity_id": "e1", "records": [
                {"record_id": "s1"}, {"record_id": "c1", "repaired": True}]},
            {"entity_id": "e2", "records": [
                {"record_id": "s2"}, {"record_id": "c2"}]},
        ],
    })
    # e1's within-cluster pair is eased (c1 repaired); e2's is not; no cross-cluster
    # (non-match) pair is ever included even though c1 appears in some of them.
    assert _dataset.repaired_truth_pairs(ds) == frozenset({("c1", "s1")})


def test_repaired_truth_pairs_empty_on_an_unmarked_dataset():
    ds = load_dataset({
        "entities": [{"entity_id": "e1", "records": [
            {"record_id": "r1"}, {"record_id": "r2"}]}],
    })
    assert _dataset.repaired_truth_pairs(ds) == frozenset()
```

Note: `("c1", "s1")` is the canonical order (`"c1" < "s1"`), matching `canonical_label_pair`. `_dataset` is already imported at the bottom of this test file.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd matcher && uv run pytest tests/test_eval_dataset.py -k repaired_truth_pairs -v`
Expected: FAIL — `AttributeError: module ... has no attribute 'repaired_truth_pairs'`.

- [ ] **Step 3: Implement the primitive** (add after `repaired_record_ids` in `dataset.py`)

```python
def repaired_truth_pairs(ds: LabelledDataset) -> frozenset[tuple[str, str]]:
    """The true-match pairs the generator made artificially easy via a verbatim-name repair.

    A true pair (both records in one cluster) is 'eased' when at least one endpoint was
    REPAIRED — generator._repair injected an exact seed name (#211 gap 4) so the pair grades
    EXACT and is trivially recovered. This is the subset of truth_pairs an eval consumer
    reports so a reader can discount the optimistic recall/F1 it produces. Cross-cluster
    (non-match) pairs are never included even when they touch a repaired record: the injection
    only eases the within-cluster true pair. Empty on a real (unmarked) dataset — the honest
    'nothing to discount'. Pure: no I/O.
    """
    repaired = repaired_record_ids(ds)
    if not repaired:
        return frozenset()
    return frozenset(
        pair for pair in truth_pairs(ds) if pair[0] in repaired or pair[1] in repaired
    )
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd matcher && uv run pytest tests/test_eval_dataset.py -k repaired_truth_pairs -v`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/eval/dataset.py matcher/tests/test_eval_dataset.py
git commit -m "feat(#290): repaired_truth_pairs — the eased-true-pair subset primitive"
```

---

### Task 2: `PairOutcome.repaired` + `ScorerMetrics.repaired_match_pairs`

**Files:**
- Modify: `matcher/src/cairn_matcher/eval/metrics.py` (`PairOutcome`, `ScorerMetrics`, `scorer_metrics`)
- Test: `matcher/tests/test_eval_metrics.py`

**Interfaces:**
- Produces: `PairOutcome(..., repaired: bool = False)`; `ScorerMetrics(..., repaired_match_pairs: int)`. `scorer_metrics` sets `repaired_match_pairs = sum(1 for o in outcomes if o.is_match and o.repaired)`.
- Consumed by: Task 3 (`scorer_outcomes` sets `repaired`), Task 4 (pooled metrics), Task 6 (report).

- [ ] **Step 1: Write the failing tests** (append to `test_eval_metrics.py`)

```python
def test_pair_outcome_repaired_defaults_false():
    assert PairOutcome(is_match=True, score_total=1.0, band=None).repaired is False


def test_scorer_metrics_counts_only_repaired_true_matches():
    outcomes = [
        PairOutcome(is_match=True, score_total=9.0, band=Band.AUTO_CANDIDATE, repaired=True),
        PairOutcome(is_match=True, score_total=8.0, band=Band.AUTO_CANDIDATE),  # match, unmarked
        PairOutcome(is_match=False, score_total=1.0, band=None, repaired=True),  # non-match: excluded
    ]
    assert scorer_metrics(outcomes).repaired_match_pairs == 1


def test_scorer_metrics_repaired_zero_when_nothing_marked():
    assert scorer_metrics([PairOutcome(is_match=True, score_total=8.0, band=None)]).repaired_match_pairs == 0
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd matcher && uv run pytest tests/test_eval_metrics.py -k repaired -v`
Expected: FAIL — `TypeError: __init__() got an unexpected keyword argument 'repaired'` (first) / `AttributeError: ... 'repaired_match_pairs'`.

- [ ] **Step 3: Implement — add the field to `PairOutcome`**

Replace the `PairOutcome` dataclass body:

```python
@dataclass(frozen=True)
class PairOutcome:
    """One evaluated pair: whether it is truly a match, its score, and its band.

    repaired is True when at least one endpoint is a synthetically-REPAIRED record
    (generator._repair injected a verbatim seed name, #211 gap 4); set by
    scorer_eval.scorer_outcomes. Only a true within-cluster pair is eased by that injection,
    so scorer_metrics reports the is_match AND repaired subset. Default False keeps every real
    dataset and hand-built outcome honest (nothing repaired).
    """

    is_match: bool
    score_total: float
    band: Band | None
    repaired: bool = False
```

- [ ] **Step 4: Implement — add the count to `ScorerMetrics` and compute it**

Add the field to the `ScorerMetrics` dataclass, after `pair_count`:

```python
    pair_count: int
    repaired_match_pairs: int
```

Update the metrics docstring's field list is not required; in `scorer_metrics`, before the `return`, compute the count and pass it:

```python
    repaired_match_pairs = sum(1 for o in outcomes if o.is_match and o.repaired)

    return ScorerMetrics(
        confusion=confusion,
        strict=strict,
        lenient=lenient,
        auto_false_link_rate=_ratio(confusion.nonmatch_auto, total_auto),
        missed_match_rate=_ratio(confusion.match_none, total_true),
        match_scores=_score_stats(match_scores),
        nonmatch_scores=_score_stats(nonmatch_scores),
        pair_count=len(outcomes),
        repaired_match_pairs=repaired_match_pairs,
    )
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd matcher && uv run pytest tests/test_eval_metrics.py -v`
Expected: PASS (all, including the 3 new). Confirms no other `ScorerMetrics` construction broke.

- [ ] **Step 6: Commit**

```bash
git add matcher/src/cairn_matcher/eval/metrics.py matcher/tests/test_eval_metrics.py
git commit -m "feat(#290): PairOutcome.repaired + ScorerMetrics.repaired_match_pairs"
```

---

### Task 3: `scorer_outcomes` sets `repaired`

**Files:**
- Modify: `matcher/src/cairn_matcher/eval/scorer_eval.py` (`scorer_outcomes` + the dataset import)
- Test: `matcher/tests/test_eval_scorer_driver.py`

**Interfaces:**
- Consumes: `repaired_record_ids` (Task's new import), `PairOutcome.repaired` (Task 2), `ScorerMetrics.repaired_match_pairs` (Task 2), `repaired_truth_pairs` (Task 1, in the test only).
- Produces: `evaluate_scorer(ds).repaired_match_pairs` == count of true pairs touching a repaired record.

- [ ] **Step 1: Write the failing tests** (append to `test_eval_scorer_driver.py`; add `repaired_truth_pairs` to the dataset import at the top of that file)

Top-of-file import becomes:
```python
from cairn_matcher.eval.dataset import load_dataset, repaired_truth_pairs
```

New tests:
```python
_REPAIRED_DS = load_dataset({
    "name": "repaired-driver",
    "entities": [
        {"entity_id": "p", "records": [
            {"record_id": "p-1",
             "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 70},
             "names": [{"value": "Ana Silva"}]},
            {"record_id": "p-2",
             "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 70},
             "names": [{"value": "Ana Silva"}], "repaired": True},
        ]},
        {"entity_id": "q", "records": [
            {"record_id": "q-1",
             "dob": {"value": "1970-01-01", "precision": "day", "provenance_rank": 70},
             "names": [{"value": "Wei Nguyen"}]},
        ]},
    ],
})


def test_evaluate_scorer_reports_repaired_match_pairs():
    m = evaluate_scorer(_REPAIRED_DS)
    # exactly the one within-cluster true pair (p-1,p-2), whose p-2 is repaired
    assert m.repaired_match_pairs == len(repaired_truth_pairs(_REPAIRED_DS)) == 1


def test_evaluate_scorer_reports_zero_repaired_on_the_unmarked_hand_set():
    assert evaluate_scorer(_DS).repaired_match_pairs == 0
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd matcher && uv run pytest tests/test_eval_scorer_driver.py -k repaired -v`
Expected: FAIL — `assert 0 == 1` (the outcome's `repaired` is still default False, so nothing counts).

- [ ] **Step 3: Implement — set `repaired` in `scorer_outcomes`**

Change the dataset import in `scorer_eval.py` to add `repaired_record_ids`:
```python
from cairn_matcher.eval.dataset import (
    LabelledDataset,
    all_pairs,
    record_to_candidate,
    repaired_record_ids,
    truth_pairs,
)
```

In `scorer_outcomes`, compute the repaired set once and pass the flag per pair:
```python
    candidates = {r.record_id: record_to_candidate(r) for r in ds.all_records()}
    truth = truth_pairs(ds)
    repaired_ids = repaired_record_ids(ds)

    outcomes: list[PairOutcome] = []
    for low, high in all_pairs(ds):
        comparisons = field_comparisons(candidates[low], candidates[high], config)
        match_score = score(comparisons, weights)
        outcomes.append(
            PairOutcome(
                is_match=(low, high) in truth,
                score_total=match_score.total,
                band=band(match_score, (), thresholds),
                repaired=low in repaired_ids or high in repaired_ids,
            )
        )
    return outcomes
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd matcher && uv run pytest tests/test_eval_scorer_driver.py -v`
Expected: PASS (all, including the 2 new).

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/eval/scorer_eval.py matcher/tests/test_eval_scorer_driver.py
git commit -m "feat(#290): scorer_outcomes flags repaired-touched pairs"
```

---

### Task 4: `LiftReport.held_out_repaired_pairs`

**Files:**
- Modify: `matcher/src/cairn_matcher/eval/crossval.py` (`LiftReport`, `kfold_lift`)
- Test: `matcher/tests/test_eval_crossval.py`

**Interfaces:**
- Consumes: `ScorerMetrics.repaired_match_pairs` (Task 2) on the pooled before/after outcomes.
- Produces: `LiftReport(..., held_out_repaired_pairs: int)`. Consumed by Task 6 (`format_lift`).

- [ ] **Step 1: Write the failing test** (append to `test_eval_crossval.py`)

```python
def test_kfold_lift_reports_held_out_repaired_pairs():
    # Six 2-record clusters; each clone marked repaired -> every held-out true pair is eased.
    ents = tuple(
        EntityCluster(entity_id=f"e{i}", records=(
            DatasetRecord(record_id=f"e{i}-a", names=({"value": f"Name{i} Fam{i}"},)),
            DatasetRecord(record_id=f"e{i}-b", names=({"value": f"Name{i} Fam{i}"},),
                          repaired=True),
        ))
        for i in range(6)
    )
    report = kfold_lift(LabelledDataset(name="repaired", entities=ents), folds=3)
    # before/after measure the SAME held-out pairs -> identical repaired counts
    assert report.before.repaired_match_pairs == report.after.repaired_match_pairs
    assert report.held_out_repaired_pairs == report.after.repaired_match_pairs
    # no fold skipped (4-cluster training has match + non-match pairs) -> all 6 true pairs
    # are measured and every one is repair-eased
    assert report.held_out_repaired_pairs == 6
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd matcher && uv run pytest tests/test_eval_crossval.py -k held_out_repaired -v`
Expected: FAIL — `AttributeError: 'LiftReport' object has no attribute 'held_out_repaired_pairs'`.

- [ ] **Step 3: Implement — add the field and set it**

Add the field to the `LiftReport` dataclass (after `after`) and extend its docstring's final sentence:

```python
    before: ScorerMetrics
    after: ScorerMetrics
    held_out_repaired_pairs: int
```

Append to the `LiftReport` docstring:
> `held_out_repaired_pairs` counts, among the measured held-out true pairs, those a synthetic repair made artificially easy (#211 gap 4 / #290) — 0 on real data; before and after measure the identical pairs, so it equals either's `repaired_match_pairs`.

In `kfold_lift`, name the two metric bundles and set the field from `after`:

```python
    before_metrics = scorer_metrics(before)
    after_metrics = scorer_metrics(after)
    return LiftReport(
        folds=folds,
        skipped_folds=skipped,
        before=before_metrics,
        after=after_metrics,
        held_out_repaired_pairs=after_metrics.repaired_match_pairs,
    )
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd matcher && uv run pytest tests/test_eval_crossval.py -v`
Expected: PASS (all, including the new one).

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/eval/crossval.py matcher/tests/test_eval_crossval.py
git commit -m "feat(#290): LiftReport.held_out_repaired_pairs from pooled held-out outcomes"
```

---

### Task 5: `LearnMetadata.train_repaired_pairs` + model_io round-trip

**Files:**
- Modify: `matcher/src/cairn_matcher/eval/learner.py` (`LearnMetadata`, `learn_model`, dataset import)
- Modify: `matcher/src/cairn_matcher/eval/model_io.py` (`_META_FIELDS`)
- Test: `matcher/tests/test_eval_learn_model.py`, `matcher/tests/test_eval_model_io.py`

**Interfaces:**
- Consumes: `repaired_truth_pairs` (Task 1).
- Produces: `LearnMetadata(..., train_repaired_pairs: int)` (a required field, inserted before `review_auto_collided`); round-trips via `model_io`.

- [ ] **Step 1: Write the failing tests**

Append to `test_eval_learn_model.py` (its top already imports `learn_model`, `load_bundled_gold`; add `load_dataset` + `repaired_truth_pairs`):
```python
from cairn_matcher.eval.dataset import load_dataset, repaired_truth_pairs  # add at top


def test_learn_model_reports_train_repaired_pairs():
    ds = load_dataset({
        "entities": [
            {"entity_id": "p", "records": [
                {"record_id": "p-1", "names": [{"value": "Ana Silva"}],
                 "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 70}},
                {"record_id": "p-2", "names": [{"value": "Ana Silva"}], "repaired": True,
                 "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 70}},
            ]},
            {"entity_id": "q", "records": [
                {"record_id": "q-1", "names": [{"value": "Wei Nguyen"}],
                 "dob": {"value": "1970-01-01", "precision": "day", "provenance_rank": 70}},
                {"record_id": "q-2", "names": [{"value": "Wei Nguyen"}],
                 "dob": {"value": "1970-01-01", "precision": "day", "provenance_rank": 70}},
            ]},
        ],
    })
    model = learn_model(ds)
    assert model.metadata.train_repaired_pairs == len(repaired_truth_pairs(ds)) == 1


def test_learn_model_train_repaired_zero_on_gold():
    assert learn_model(load_bundled_gold()).metadata.train_repaired_pairs == 0
```

Append to `test_eval_model_io.py`:
```python
def test_round_trip_preserves_train_repaired_pairs(tmp_path):
    # learn_model on gold gives train_repaired_pairs == 0; force a non-zero via a dict edit to
    # prove the field survives to_json/from_json and file round-trip, not just the 0 default.
    model = learn_model(load_bundled_gold())
    obj = model_to_json(model)
    obj["metadata"]["train_repaired_pairs"] = 3
    restored = model_from_json(obj)
    assert restored.metadata.train_repaired_pairs == 3
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd matcher && uv run pytest tests/test_eval_learn_model.py tests/test_eval_model_io.py -k "train_repaired or repaired" -v`
Expected: FAIL — `TypeError: __init__() got an unexpected keyword argument 'train_repaired_pairs'` (from the model_io test's `from_json` once `_META_FIELDS` includes it) and `AttributeError` on `.train_repaired_pairs` (learner tests).

- [ ] **Step 3: Implement — `learner.py`**

Add `repaired_truth_pairs` to the dataset import:
```python
from cairn_matcher.eval.dataset import (
    LabelledDataset,
    all_pairs,
    record_to_candidate,
    repaired_truth_pairs,
    truth_pairs,
)
```

Add the field to `LearnMetadata` (before `review_auto_collided`) and note it in the docstring:
```python
    train_pairs: int
    train_matches: int
    train_repaired_pairs: int
    review_auto_collided: bool
```
(docstring line: "... the knobs + training-set size + repaired-pair count (#290) + collision flag.")

In `learn_model`, set it from the primitive:
```python
    metadata = LearnMetadata(
        alpha=alpha,
        recall_target=recall_target,
        margin=margin,
        train_pairs=len(labelled),
        train_matches=sum(1 for is_m, _ in labelled if is_m),
        train_repaired_pairs=len(repaired_truth_pairs(ds)),
        review_auto_collided=collided,
    )
```

- [ ] **Step 4: Implement — `model_io.py`**

Add the field to `_META_FIELDS` (mirror the dataclass order):
```python
_META_FIELDS = (
    "alpha", "recall_target", "margin", "train_pairs", "train_matches",
    "train_repaired_pairs", "review_auto_collided",
)
```

- [ ] **Step 5: Fix the six existing model_io bad-fixtures** (integrity, not new behavior)

Each of these tests builds a `metadata` dict to isolate a *different* failure. With `train_repaired_pairs` now required, add the key so metadata construction succeeds and the intended failure (bad weights/thresholds) is what raises. In `test_eval_model_io.py`, in every `metadata={...}` block inside `test_unknown_agreement_level_rejected`, `test_missing_top_level_key_rejected` (this one has no metadata block — skip it), `test_non_numeric_weight_value_rejected`, `test_non_mapping_weights_rejected`, `test_non_numeric_threshold_value_rejected`, and `test_inverted_thresholds_rejected`, change:
```python
            "train_pairs": 1, "train_matches": 1, "review_auto_collided": False,
```
to:
```python
            "train_pairs": 1, "train_matches": 1, "train_repaired_pairs": 0,
            "review_auto_collided": False,
```
(`test_missing_top_level_key_rejected` passes only `{"weights": {}}` — leave it.)

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cd matcher && uv run pytest tests/test_eval_learn_model.py tests/test_eval_model_io.py -v`
Expected: PASS (all, including the new round-trip and the unchanged-intent bad-fixtures). `test_round_trip_reconstructs_a_model...` and `test_file_round_trip` already assert `restored.metadata == model.metadata`, so they also now cover the new field via gold (value 0).

- [ ] **Step 7: Commit**

```bash
git add matcher/src/cairn_matcher/eval/learner.py matcher/src/cairn_matcher/eval/model_io.py \
        matcher/tests/test_eval_learn_model.py matcher/tests/test_eval_model_io.py
git commit -m "feat(#290): LearnMetadata.train_repaired_pairs + model_io round-trip"
```

---

### Task 6: Render the counts in `report.py`

**Files:**
- Modify: `matcher/src/cairn_matcher/eval/report.py` (`format_scorer`), `matcher/src/cairn_matcher/eval/crossval.py` (`format_lift`)
- Test: `matcher/tests/test_eval_report.py`, `matcher/tests/test_eval_crossval.py`

**Interfaces:**
- Consumes: `ScorerMetrics.repaired_match_pairs` (Task 2), `LiftReport.held_out_repaired_pairs` (Task 4).
- Produces: a "repaired match pairs" line in the scorer report; a held-out-repaired figure in the lift report.

- [ ] **Step 1: Write the failing tests**

Append to `test_eval_report.py` (top already imports `format_scorer`, `evaluate_scorer`; add `load_dataset`):
```python
from cairn_matcher.eval.dataset import load_dataset  # add at top


def test_scorer_report_shows_repaired_match_pair_count():
    ds = load_dataset({
        "entities": [{"entity_id": "p", "records": [
            {"record_id": "p-1", "names": [{"value": "Ana Silva"}],
             "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 70}},
            {"record_id": "p-2", "names": [{"value": "Ana Silva"}], "repaired": True,
             "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 70}},
        ]}],
    })
    text = format_scorer(evaluate_scorer(ds))
    assert "repaired match pairs" in text
    assert "1 of 1" in text  # 1 repaired of 1 true pair


def test_scorer_report_shows_zero_repaired_on_unmarked_data():
    text = format_scorer(evaluate_scorer(load_bundled_gold()))
    assert "repaired match pairs" in text
    assert "0 of 3" in text  # gold has 3 true pairs, none repaired
```

Append to `test_eval_crossval.py`:
```python
def test_format_lift_shows_held_out_repaired_pairs():
    text = format_lift(kfold_lift(_synthetic(6), folds=3), dataset_name="synthetic")
    assert "repaired" in text.lower()
    assert "0" in text  # _synthetic clusters carry no repaired marker
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd matcher && uv run pytest tests/test_eval_report.py tests/test_eval_crossval.py -k "repaired" -v`
Expected: FAIL — the substrings are absent from the rendered reports.

- [ ] **Step 3: Implement — `format_scorer`**

In `report.py`, inside `format_scorer`, build the repaired line from the confusion true-total and insert it just before `_CAVEAT` in the `lines` list:
```python
    true_total = c.match_auto + c.match_review + c.match_none
    repaired_line = (
        f"  repaired match pairs (synthetically eased): "
        f"{metrics.repaired_match_pairs} of {true_total}"
    )
    if metrics.repaired_match_pairs:
        repaired_line += (
            f"  — recall/f1 above optimistic by up to "
            f"{metrics.repaired_match_pairs} true pair(s)"
        )
```
Then add `repaired_line,` to the `lines` list immediately before `_CAVEAT`.

- [ ] **Step 4: Implement — `format_lift`**

In `crossval.py` `format_lift`, add the held-out figure to the report. Change the title line list entry to append a held-out-repaired note after the AFTER block (before the trailing NOTE):
```python
            format_scorer(report.after),
            f"held-out repaired pairs (synthetically eased): {report.held_out_repaired_pairs}",
            (
                "NOTE: PoC — advisory, not shipped weights; ..."
            ),
```
(Keep the existing NOTE text verbatim; only insert the new line before it.)

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd matcher && uv run pytest tests/test_eval_report.py tests/test_eval_crossval.py -v`
Expected: PASS (all, including the 3 new). The existing `test_scorer_report_mentions_key_metrics_and_the_caveat` still passes (caveat unchanged, only a line added above it).

- [ ] **Step 6: Commit**

```bash
git add matcher/src/cairn_matcher/eval/report.py matcher/src/cairn_matcher/eval/crossval.py \
        matcher/tests/test_eval_report.py matcher/tests/test_eval_crossval.py
git commit -m "feat(#290): render repaired-pair counts in the scorer + lift reports"
```

---

### Task 7: Full-suite + lint verification

**Files:** none (verification only).

- [ ] **Step 1: Run the full pure matcher suite**

Run: `cd matcher && uv run pytest`
Expected: PASS, 0 failed. Baseline was 395; +14 new tests here (2+3+2+1+3+3) → ~409 (exact number is whatever the run reports — the gate is 0 failed, not a fixed count).

- [ ] **Step 2: Run ruff**

Run: `cd matcher && uv run ruff check`
Expected: `All checks passed!` (the CI matcher.yml ruleset: I/UP/B/E5 at line-length=100).

- [ ] **Step 3: Manual CLI smoke (optional, non-gating)**

Run: `cd matcher && uv run python -m cairn_matcher.eval.learn --folds 5`
Expected: the lift report prints and now includes the "held-out repaired pairs" line and, inside each scorer block, the "repaired match pairs" line (both `0` on the bundled gold set).

- [ ] **Step 4: No commit** — this task only verifies. (HANDOVER/ROADMAP updates happen at session close, outside this plan.)

---

## Notes for the implementer

- **Pure tier:** every file here is import-light; do not add DB or psycopg imports. The `matcher` pure suite must stay runnable with just `uv run pytest`.
- **Additive discipline:** all new fields default-or-required-but-single-construction; the only field with a default is `PairOutcome.repaired` (so hand-built outcomes and real datasets stay valid). `ScorerMetrics.repaired_match_pairs`, `LiftReport.held_out_repaired_pairs`, and `LearnMetadata.train_repaired_pairs` are required — each has exactly one production construction site, updated in its task.
- **Why report, not exclude:** a repaired true pair is a *real* match the matcher should recover; dropping it understates coverage. See the design doc `docs/superpowers/specs/2026-07-25-repaired-pair-eval-reporting-290.md` for the full rationale.
