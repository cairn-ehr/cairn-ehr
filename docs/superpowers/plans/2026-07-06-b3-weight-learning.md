# B3 Weight-Learning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a pure, supervised Fellegi–Sunter learner that turns a labelled matcher dataset into a `(Weights, Thresholds)` model, plus an honest k-fold held-out before/after measurement and a CLI — advisory eval-tier only.

**Architecture:** Closed-form F-S m/u estimation by counting agreement levels across labelled match/non-match pairs (same math as `scoring.score`, run backwards from ground truth), then safety-first threshold derivation coupled to the learned weights. Measurement splits on whole entity clusters (never pairs) and reports metrics only on held-out folds. All new code is pure and lives under `matcher/src/cairn_matcher/eval/`.

**Tech Stack:** Python 3 (dependency-free, stdlib only — `math`, `argparse`, `json`), `uv` for the test/run harness (never venv/pip). Reuses the existing eval harness (`eval/dataset.py`, `eval/scorer_eval.py`, `eval/metrics.py`, `eval/report.py`) and the production scoring path (`orchestrator.field_comparisons`, `scoring.score`, `pipeline/banding`).

## Global Constraints

- **License:** AGPL-3.0; no new third-party dependency (this slice adds none — stdlib only). Copied verbatim from house rules #1.
- **TDD:** failing test first, then minimal code (house rule #2).
- **Purity:** pure functions in focused modules; no I/O in the core (`learner.py`, `crossval.py`, `model_io.py` are filesystem-free; only the CLI `learn.py` touches disk). House rule #1/#4.
- **File size:** keep each file < 500 lines (house rule #4). All new files here are well under.
- **Inline docs:** every non-trivial function carries a docstring legible to a junior contributor — *why* it exists and *how* it fits (house rule #3).
- **Loud on malformed input:** raise, never silently default (house rule #5).
- **Determinism:** no RNG, no wall-clock. k-fold splitting stripes by *sorted* `entity_id`.
- **No production-path change:** `scoring.py`, `banding.py`, `orchestrator.py`, comparators, `pipeline/`, and every `db/` file are untouched. The only edit to an existing file is an additive, behavior-preserving refactor of `eval/scorer_eval.py` (Task 4).
- **Run tests from `matcher/`:** pure suite `uv run pytest`; a single test `uv run pytest tests/<file>::<test> -v`.

---

### Task 1: Weight estimation core (`estimate_weights` + `labelled_comparisons`)

**Files:**
- Create: `matcher/src/cairn_matcher/eval/learner.py`
- Test: `matcher/tests/test_eval_learn.py`

**Interfaces:**
- Consumes: `eval/dataset.py` (`LabelledDataset`, `all_pairs`, `record_to_candidate`, `truth_pairs`), `orchestrator` (`DEFAULT_CONFIG`, `ComparatorConfig`, `field_comparisons`), `records.FieldComparison`, `scoring` (`FieldWeights`, `Weights`), `agreement.AgreementLevel`.
- Produces:
  - `LabelledPair = tuple[bool, list[FieldComparison]]`
  - `labelled_comparisons(ds: LabelledDataset, config: ComparatorConfig = DEFAULT_CONFIG) -> list[LabelledPair]`
  - `estimate_weights(labelled: Sequence[LabelledPair], *, alpha: float = 0.5) -> Weights`

- [ ] **Step 1: Write the failing test**

Create `matcher/tests/test_eval_learn.py`:

```python
"""Tests for the supervised F-S weight estimator (eval/learner.py)."""

import math

import pytest

from cairn_matcher.agreement import AgreementLevel
from cairn_matcher.eval.learner import estimate_weights, labelled_comparisons
from cairn_matcher.eval.loader import load_bundled_gold
from cairn_matcher.records import FieldComparison


def _pair(is_match, *fields):
    """(is_match, [FieldComparison...]); each field is (name, level, rank=0)."""
    comps = [FieldComparison(f, lvl, rank) for f, lvl, rank in
             ((name, lvl, 0) for name, lvl in fields)]
    return (is_match, comps)


def test_level_concentrated_in_matches_earns_positive_weight():
    # dob EXACT appears only among matches -> m>u -> positive log2(m/u).
    labelled = [
        _pair(True, ("dob", AgreementLevel.EXACT)),
        _pair(True, ("dob", AgreementLevel.EXACT)),
        _pair(False, ("dob", AgreementLevel.DISAGREE)),
        _pair(False, ("dob", AgreementLevel.DISAGREE)),
    ]
    w = estimate_weights(labelled, alpha=0.5)
    assert w.per_field["dob"].weight_for(AgreementLevel.EXACT) > 0.0
    assert w.per_field["dob"].weight_for(AgreementLevel.DISAGREE) < 0.0


def test_zero_count_cell_is_bounded_not_infinite():
    # EXACT never occurs among non-matches: smoothing must keep the weight finite.
    labelled = [
        _pair(True, ("dob", AgreementLevel.EXACT)),
        _pair(False, ("dob", AgreementLevel.DISAGREE)),
    ]
    w = estimate_weights(labelled, alpha=0.5)
    weight = w.per_field["dob"].weight_for(AgreementLevel.EXACT)
    assert math.isfinite(weight)


def test_insufficient_data_never_learns_a_weight():
    labelled = [
        _pair(True, ("dob", AgreementLevel.INSUFFICIENT_DATA),
                    ("name", AgreementLevel.EXACT)),
        _pair(False, ("name", AgreementLevel.DISAGREE)),
    ]
    w = estimate_weights(labelled, alpha=0.5)
    # dob was only ever INSUFFICIENT_DATA -> an entry exists but prices no level.
    assert AgreementLevel.INSUFFICIENT_DATA not in w.per_field["dob"].weights
    assert w.per_field["dob"].weights == {}


def test_every_seen_field_gets_an_entry_even_if_never_comparable():
    # A field only ever INSUFFICIENT_DATA still gets an (empty) entry so score() never
    # raises for it on held-out data; an empty entry scores 0.0 at every level.
    labelled = [_pair(True, ("dob", AgreementLevel.INSUFFICIENT_DATA))]
    w = estimate_weights(labelled, alpha=0.5)
    assert "dob" in w.per_field
    assert w.per_field["dob"].weight_for(AgreementLevel.EXACT) == 0.0


def test_alpha_must_be_positive():
    with pytest.raises(ValueError):
        estimate_weights([_pair(True, ("dob", AgreementLevel.EXACT))], alpha=0.0)


def test_empty_labelled_raises():
    with pytest.raises(ValueError):
        estimate_weights([])


def test_labelled_comparisons_reuses_real_path_on_gold():
    ds = load_bundled_gold()
    labelled = labelled_comparisons(ds)
    # gold: 10 records -> C(10,2)=45 pairs; exactly 3 true-match pairs (3 two-record clusters).
    assert len(labelled) == 45
    assert sum(1 for is_m, _ in labelled if is_m) == 3
    # every entry is (bool, list[FieldComparison])
    for is_m, comps in labelled:
        assert isinstance(is_m, bool)
        assert all(isinstance(c, FieldComparison) for c in comps)
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd matcher && uv run pytest tests/test_eval_learn.py -v`
Expected: FAIL with `ModuleNotFoundError: No module named 'cairn_matcher.eval.learner'`.

- [ ] **Step 3: Write minimal implementation**

Create `matcher/src/cairn_matcher/eval/learner.py`:

```python
"""Supervised Fellegi–Sunter weight + threshold learning from a labelled dataset.

Pure, deterministic, no I/O. The scorer (scoring.score) sums per-field log2(m/u)
log-weights; this module estimates those weights the canonical F-S way — by counting
agreement levels across labelled match / non-match pairs — then (Task 2/3) derives the two
banding thresholds that go with them. Same math as scoring, run backwards from ground truth.

PoC (design docs/superpowers/specs/2026-07-06-b3-weight-learning-design.md): ships the
mechanism, not the shipped defaults. Production weights come from real local adjudication
(§5.13 / ADR-0014); synthetic-learned weights reflect the generator's corruption model.
"""

import math
from collections.abc import Sequence

from cairn_matcher.agreement import AgreementLevel
from cairn_matcher.eval.dataset import (
    LabelledDataset,
    all_pairs,
    record_to_candidate,
    truth_pairs,
)
from cairn_matcher.orchestrator import DEFAULT_CONFIG, ComparatorConfig, field_comparisons
from cairn_matcher.records import FieldComparison
from cairn_matcher.scoring import FieldWeights, Weights

LabelledPair = tuple[bool, list[FieldComparison]]


def labelled_comparisons(
    ds: LabelledDataset, config: ComparatorConfig = DEFAULT_CONFIG
) -> list[LabelledPair]:
    """(is_match, field comparisons) for every record pair via the REAL scoring path.

    Reuses the production adapter (record_to_candidate) + orchestrator (field_comparisons)
    so the learner sees exactly what ships; candidates are built once per record, not once
    per pair. The ground-truth label is membership in the dataset's within-cluster pairs.
    """
    candidates = {r.record_id: record_to_candidate(r) for r in ds.all_records()}
    truth = truth_pairs(ds)
    out: list[LabelledPair] = []
    for low, high in all_pairs(ds):
        comps = field_comparisons(candidates[low], candidates[high], config)
        out.append(((low, high) in truth, comps))
    return out


def estimate_weights(labelled: Sequence[LabelledPair], *, alpha: float = 0.5) -> Weights:
    """Learn a Weights table by counting agreement levels across labelled pairs.

    For each field f and each level L it prices (INSUFFICIENT_DATA excluded — a missing
    field is zero evidence, never a weight, §3.7), estimate
        m = P(L | match, f comparable),  u = P(L | non-match, f comparable)
    with additive (Laplace) smoothing (alpha) over the field's observed level set, so a
    zero-count cell yields a bounded weight, never ±inf. weight = log2(m / u).

    Every field seen in ANY comparison gets an entry (empty if it was never comparable in
    training) so score() never raises on held-out data for a known field; an empty entry
    scores 0.0 at every level — the honest 'no learned evidence' degrade.
    """
    if alpha <= 0.0:
        raise ValueError(f"alpha (Laplace smoothing) must be > 0, got {alpha}")
    if not labelled:
        raise ValueError("estimate_weights needs at least one labelled pair")

    # counts[field] = {"match": {level: n}, "nonmatch": {level: n}}; seen_fields tracks
    # every field name, including those only ever INSUFFICIENT_DATA, for the entry guarantee.
    counts: dict[str, dict[str, dict[AgreementLevel, int]]] = {}
    seen_fields: set[str] = set()
    for is_match, comps in labelled:
        cls = "match" if is_match else "nonmatch"
        for comp in comps:
            seen_fields.add(comp.field)
            if comp.level is AgreementLevel.INSUFFICIENT_DATA:
                continue
            bucket = counts.setdefault(comp.field, {"match": {}, "nonmatch": {}})
            bucket[cls][comp.level] = bucket[cls].get(comp.level, 0) + 1

    per_field: dict[str, FieldWeights] = {}
    for f in seen_fields:
        bucket = counts.get(f, {"match": {}, "nonmatch": {}})
        levels = sorted(set(bucket["match"]) | set(bucket["nonmatch"]))
        if not levels:
            per_field[f] = FieldWeights({})  # only ever INSUFFICIENT_DATA -> prices nothing
            continue
        k = len(levels)
        m_total = sum(bucket["match"].values())
        u_total = sum(bucket["nonmatch"].values())
        weights: dict[AgreementLevel, float] = {}
        for level in levels:
            m = (bucket["match"].get(level, 0) + alpha) / (m_total + alpha * k)
            u = (bucket["nonmatch"].get(level, 0) + alpha) / (u_total + alpha * k)
            weights[level] = math.log2(m / u)
        per_field[f] = FieldWeights(weights)
    return Weights(per_field=per_field)
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd matcher && uv run pytest tests/test_eval_learn.py -v`
Expected: PASS (7 passed).

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/eval/learner.py matcher/tests/test_eval_learn.py
git commit -m "feat(matcher): supervised F-S weight estimator (estimate_weights + labelled_comparisons)"
```

---

### Task 2: Safety-first threshold derivation (`derive_thresholds`)

**Files:**
- Modify: `matcher/src/cairn_matcher/eval/learner.py` (append)
- Test: `matcher/tests/test_eval_thresholds.py`

**Interfaces:**
- Consumes: `pipeline/banding.Thresholds`.
- Produces: `derive_thresholds(scored: Sequence[tuple[bool, float]], *, recall_target: float = 0.99, margin: float = 0.5) -> tuple[Thresholds, bool]` — returns `(thresholds, collided)` where `collided` is `review >= auto`.

- [ ] **Step 1: Write the failing test**

Create `matcher/tests/test_eval_thresholds.py`:

```python
"""Tests for safety-first threshold derivation (eval/learner.derive_thresholds)."""

import pytest

from cairn_matcher.eval.learner import derive_thresholds


def test_auto_is_above_every_nonmatch_score():
    # Zero false auto-links by construction: auto > every non-match score.
    scored = [(True, 10.0), (True, 9.0), (False, 4.0), (False, 3.0)]
    thresholds, collided = derive_thresholds(scored, recall_target=0.99, margin=0.5)
    assert thresholds.auto == 4.0 + 0.5
    assert not collided
    assert all(s < thresholds.auto for is_m, s in scored if not is_m)


def test_review_is_the_top_nonmatch_and_surfaces_matches_above_it():
    # review anchors to the best impostor; every match above it is surfaced.
    scored = [(True, float(i)) for i in range(1, 101)] + [(False, 0.0)]
    thresholds, collided = derive_thresholds(scored, recall_target=0.99, margin=0.5)
    assert thresholds.review == 0.0
    surfaced = sum(1 for is_m, s in scored if is_m and s >= thresholds.review)
    assert surfaced == 100
    assert not collided


def test_separated_classes_keep_review_below_auto_and_do_not_collide():
    # The ideal case: matches far above non-matches. review must stay < auto (band invariant),
    # collided must be False — this is the regression guard for the review>auto inversion bug.
    scored = [(True, 20.0), (True, 18.0), (False, 5.0), (False, 4.0)]
    thresholds, collided = derive_thresholds(scored, recall_target=0.99, margin=0.5)
    assert thresholds.review == 5.0
    assert thresholds.auto == 5.5
    assert thresholds.review < thresholds.auto
    assert not collided


def test_impostor_outscoring_matches_flags_a_collision():
    # A non-match out-scores every true match -> safety-first placement cannot surface them
    # without also surfacing the impostor -> collided True, but auto stays above the impostor.
    scored = [(True, 5.0), (True, 2.0), (False, 6.0)]
    thresholds, collided = derive_thresholds(scored, recall_target=0.99, margin=0.5)
    assert thresholds.review == 6.0
    assert thresholds.auto == 6.5  # auto still above the top non-match: zero false-auto
    assert thresholds.review < thresholds.auto
    assert collided  # no true match reaches review=6.0 -> recall floor unmet


def test_recall_shortfall_sets_the_collision_flag():
    # recall_target 1.0 with matches entangled below the impostor -> collided.
    scored = [(True, 1.0), (True, 2.0), (False, 10.0)]
    thresholds, collided = derive_thresholds(scored, recall_target=1.0, margin=0.5)
    assert collided
    assert thresholds.review < thresholds.auto


def test_no_match_pair_raises():
    with pytest.raises(ValueError):
        derive_thresholds([(False, 1.0), (False, 2.0)])


def test_recall_target_out_of_range_raises():
    with pytest.raises(ValueError):
        derive_thresholds([(True, 1.0)], recall_target=1.5)
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd matcher && uv run pytest tests/test_eval_thresholds.py -v`
Expected: FAIL with `ImportError: cannot import name 'derive_thresholds'`.

- [ ] **Step 3: Write minimal implementation**

Append to `matcher/src/cairn_matcher/eval/learner.py` (add `from cairn_matcher.pipeline.banding import Thresholds` to the imports at the top):

```python
def derive_thresholds(
    scored: Sequence[tuple[bool, float]],
    *,
    recall_target: float = 0.99,
    margin: float = 0.5,
) -> tuple[Thresholds, bool]:
    """Pick (review, auto) safety-first from labelled scores. Returns (thresholds, collided).

    Both thresholds anchor to the BEST-SCORING non-match (the strongest impostor the data
    contains) — that anchoring is what makes the placement safety-first and keeps the
    band() invariant review <= auto true by construction:

      * auto = max(non-match score) + margin -> ZERO false auto-links: no non-match reaches
        auto (an auto-link is an un-attested, if recallable, link; false-auto is the
        matcher's stated dangerous rate).
      * review = max(non-match score) -> surface any pair that OUT-SCORES the best impostor,
        never below it (a review below the top non-match would flood the worklist with
        impostor-grade pairs — the opposite of safety-first). Strictly below auto whenever
        margin > 0, so review <= auto always holds — no inversion, no clamp, however well or
        poorly the classes separate. (No non-matches at all -> fall back to the match range:
        review = min(match), auto = max(match) + margin.)

    recall_target (default 0.99) is a DIAGNOSTIC, not a lever on review. With review fixed at
    the safe placement, 'collided' is True when that placement fails to surface
    recall_target of the true matches (achieved_recall < recall_target) — i.e. some true
    matches are entangled BELOW the best impostor, so safety-first placement and the recall
    floor genuinely conflict on this data. The learner flags, never compromises: it will not
    drag review into the impostor range to chase recall.
    """
    if not 0.0 < recall_target <= 1.0:
        raise ValueError(f"recall_target must be in (0, 1], got {recall_target}")
    match_scores = [s for is_m, s in scored if is_m]
    nonmatch_scores = [s for is_m, s in scored if not is_m]
    if not match_scores:
        raise ValueError("derive_thresholds needs at least one true-match pair")

    if nonmatch_scores:
        review = max(nonmatch_scores)
    else:
        review = min(match_scores)
    auto = review + margin

    surfaced = sum(1 for s in match_scores if s >= review)
    achieved_recall = surfaced / len(match_scores)
    collided = achieved_recall < recall_target

    return Thresholds(review=review, auto=auto), collided
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd matcher && uv run pytest tests/test_eval_thresholds.py -v`
Expected: PASS (7 passed).

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/eval/learner.py matcher/tests/test_eval_thresholds.py
git commit -m "feat(matcher): safety-first threshold derivation (zero-false-auto + recall floor)"
```

---

### Task 3: Model composition (`LearnedModel` + `learn_model`)

**Files:**
- Modify: `matcher/src/cairn_matcher/eval/learner.py` (append)
- Test: `matcher/tests/test_eval_learn_model.py`

**Interfaces:**
- Consumes: `scoring.score`; Task 1 `labelled_comparisons`, `estimate_weights`; Task 2 `derive_thresholds`.
- Produces:
  - `LearnMetadata(alpha: float, recall_target: float, margin: float, train_pairs: int, train_matches: int, review_auto_collided: bool)` (frozen dataclass)
  - `LearnedModel(weights: Weights, thresholds: Thresholds, metadata: LearnMetadata)` (frozen dataclass)
  - `learn_model(ds: LabelledDataset, *, config: ComparatorConfig = DEFAULT_CONFIG, alpha: float = 0.5, recall_target: float = 0.99, margin: float = 0.5) -> LearnedModel`

- [ ] **Step 1: Write the failing test**

Create `matcher/tests/test_eval_learn_model.py`:

```python
"""Tests for learn_model — estimate weights then derive the thresholds that go with them."""

from cairn_matcher.eval.learner import LearnedModel, learn_model
from cairn_matcher.eval.loader import load_bundled_gold
from cairn_matcher.eval.scorer_eval import evaluate_scorer


def test_learn_model_on_gold_returns_a_usable_model():
    ds = load_bundled_gold()
    model = learn_model(ds)
    assert isinstance(model, LearnedModel)
    # the learned weights/thresholds drive the real eval path without raising
    metrics = evaluate_scorer(ds, weights=model.weights, thresholds=model.thresholds)
    assert metrics.pair_count == 45
    # zero false auto-links on the training set (safety-first threshold, self-consistency)
    assert metrics.confusion.nonmatch_auto == 0


def test_metadata_records_the_knobs_and_counts():
    ds = load_bundled_gold()
    model = learn_model(ds, alpha=0.7, recall_target=0.95, margin=1.0)
    assert model.metadata.alpha == 0.7
    assert model.metadata.recall_target == 0.95
    assert model.metadata.margin == 1.0
    assert model.metadata.train_pairs == 45
    assert model.metadata.train_matches == 3
    assert isinstance(model.metadata.review_auto_collided, bool)


def test_learned_review_below_auto_on_gold():
    # gold's matches separate from non-matches, so the two objectives should not collide.
    model = learn_model(load_bundled_gold())
    assert model.thresholds.review < model.thresholds.auto
    assert not model.metadata.review_auto_collided
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd matcher && uv run pytest tests/test_eval_learn_model.py -v`
Expected: FAIL with `ImportError: cannot import name 'learn_model'`.

- [ ] **Step 3: Write minimal implementation**

Append to `matcher/src/cairn_matcher/eval/learner.py` (add `from dataclasses import dataclass` and `from cairn_matcher.scoring import score` — extend the existing `scoring` import to also bring in `score`):

```python
@dataclass(frozen=True)
class LearnMetadata:
    """Provenance of a learned model: the knobs + training-set size + collision flag."""

    alpha: float
    recall_target: float
    margin: float
    train_pairs: int
    train_matches: int
    review_auto_collided: bool


@dataclass(frozen=True)
class LearnedModel:
    """A learned (weights, thresholds) pair plus the metadata that produced it.

    weights/thresholds are the exact production types (scoring.Weights,
    banding.Thresholds), so a caller can drop them straight into score()/band().
    """

    weights: Weights
    thresholds: Thresholds
    metadata: LearnMetadata


def learn_model(
    ds: LabelledDataset,
    *,
    config: ComparatorConfig = DEFAULT_CONFIG,
    alpha: float = 0.5,
    recall_target: float = 0.99,
    margin: float = 0.5,
) -> LearnedModel:
    """Estimate weights, then derive the thresholds that go with them, from one dataset.

    Coupled by design: learned weights rescale the total score, so the shipped
    (review, auto) defaults are meaningless afterward — thresholds are re-derived on the
    SAME training pairs, scored with the freshly-learned weights (provenance applied via
    the real score()).
    """
    labelled = labelled_comparisons(ds, config)
    weights = estimate_weights(labelled, alpha=alpha)
    scored = [(is_m, score(comps, weights).total) for is_m, comps in labelled]
    thresholds, collided = derive_thresholds(
        scored, recall_target=recall_target, margin=margin
    )
    metadata = LearnMetadata(
        alpha=alpha,
        recall_target=recall_target,
        margin=margin,
        train_pairs=len(labelled),
        train_matches=sum(1 for is_m, _ in labelled if is_m),
        review_auto_collided=collided,
    )
    return LearnedModel(weights=weights, thresholds=thresholds, metadata=metadata)
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd matcher && uv run pytest tests/test_eval_learn_model.py -v`
Expected: PASS (3 passed).

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/eval/learner.py matcher/tests/test_eval_learn_model.py
git commit -m "feat(matcher): learn_model — coupled weight+threshold learning from a dataset"
```

---

### Task 4: Held-out measurement (`scorer_outcomes` refactor + `eval/crossval.py`)

**Files:**
- Modify: `matcher/src/cairn_matcher/eval/scorer_eval.py` (extract `scorer_outcomes`, keep `evaluate_scorer` behavior identical)
- Create: `matcher/src/cairn_matcher/eval/crossval.py`
- Test: `matcher/tests/test_eval_crossval.py`

**Interfaces:**
- Consumes: `eval/scorer_eval.scorer_outcomes` (new), `eval/metrics` (`PairOutcome`, `ScorerMetrics`, `scorer_metrics`), `eval/report.format_scorer`, `eval/dataset` (`LabelledDataset`, `EntityCluster`), `eval/learner.learn_model`, `orchestrator` (`DEFAULT_CONFIG`, `ComparatorConfig`), `scoring.DEFAULT_WEIGHTS`, `banding.DEFAULT_THRESHOLDS`.
- Produces:
  - `scorer_outcomes(ds, *, weights=DEFAULT_WEIGHTS, thresholds=DEFAULT_THRESHOLDS, config=DEFAULT_CONFIG) -> list[PairOutcome]`
  - `split_clusters(ds: LabelledDataset, folds: int) -> list[LabelledDataset]`
  - `LiftReport(folds: int, skipped_folds: int, before: ScorerMetrics, after: ScorerMetrics)` (frozen dataclass)
  - `kfold_lift(ds, *, folds=5, config=DEFAULT_CONFIG, alpha=0.5, recall_target=0.99, margin=0.5) -> LiftReport`
  - `format_lift(report: LiftReport, *, dataset_name: str = "") -> str`

- [ ] **Step 1: Write the failing test**

Create `matcher/tests/test_eval_crossval.py`:

```python
"""Tests for k-fold held-out weight-learning measurement (eval/crossval.py)."""

import pytest

from cairn_matcher.eval.crossval import (
    LiftReport,
    format_lift,
    kfold_lift,
    split_clusters,
)
from cairn_matcher.eval.dataset import DatasetRecord, EntityCluster, LabelledDataset
from cairn_matcher.eval.loader import load_bundled_gold


def _cluster(entity_id, *record_ids):
    return EntityCluster(
        entity_id=entity_id,
        records=tuple(DatasetRecord(record_id=r) for r in record_ids),
    )


def _synthetic(n_clusters):
    # Each cluster has 2 records (so every cluster yields a match pair, robust to any split).
    ents = tuple(_cluster(f"e{i}", f"e{i}-a", f"e{i}-b") for i in range(n_clusters))
    return LabelledDataset(name="synthetic", entities=ents)


def test_split_never_straddles_a_cluster_and_covers_every_cluster_once():
    ds = _synthetic(7)
    parts = split_clusters(ds, folds=3)
    all_ids = [e.entity_id for p in parts for e in p.entities]
    assert sorted(all_ids) == [f"e{i}" for i in range(7)]  # each cluster exactly once
    assert len(all_ids) == len(set(all_ids))


def test_split_is_deterministic_across_calls():
    ds = _synthetic(7)
    a = [[e.entity_id for e in p.entities] for p in split_clusters(ds, 3)]
    b = [[e.entity_id for e in p.entities] for p in split_clusters(ds, 3)]
    assert a == b


def test_split_rejects_too_few_clusters_or_folds():
    with pytest.raises(ValueError):
        split_clusters(_synthetic(2), folds=3)
    with pytest.raises(ValueError):
        split_clusters(_synthetic(7), folds=1)


def test_kfold_lift_on_gold_reports_before_and_after():
    ds = load_bundled_gold()
    report = kfold_lift(ds, folds=5)
    assert isinstance(report, LiftReport)
    assert report.folds == 5
    # held-out pooling covers only within-fold pairs; both before/after on the SAME set
    assert report.before.pair_count == report.after.pair_count
    assert report.before.pair_count > 0


def test_kfold_lift_skips_a_fold_whose_training_has_no_match_pairs():
    # 3-fold on gold puts all 3 match clusters in one fold -> training the other folds has
    # zero match pairs; that fold is skipped (not a crash).
    report = kfold_lift(load_bundled_gold(), folds=3)
    assert report.skipped_folds >= 1
    assert report.before.pair_count == report.after.pair_count


def test_format_lift_shows_both_blocks():
    text = format_lift(kfold_lift(_synthetic(6), folds=3), dataset_name="synthetic")
    assert "BEFORE" in text and "AFTER" in text
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd matcher && uv run pytest tests/test_eval_crossval.py -v`
Expected: FAIL with `ModuleNotFoundError: No module named 'cairn_matcher.eval.crossval'`.

- [ ] **Step 3a: Refactor `scorer_eval.py` to expose `scorer_outcomes`**

In `matcher/src/cairn_matcher/eval/scorer_eval.py`, replace the body of `evaluate_scorer` (lines ~28–54, the `candidates`/`outcomes` loop) so the per-pair outcome construction lives in a new `scorer_outcomes` function and `evaluate_scorer` just aggregates it. The final file's two functions read:

```python
def scorer_outcomes(
    ds: LabelledDataset,
    *,
    weights: Weights = DEFAULT_WEIGHTS,
    thresholds: Thresholds = DEFAULT_THRESHOLDS,
    config: ComparatorConfig = DEFAULT_CONFIG,
) -> list[PairOutcome]:
    """Score+band every record pair against ground truth -> a per-pair outcome list.

    Extracted from evaluate_scorer so k-fold measurement (crossval.py) can POOL the raw
    outcomes across held-out folds before computing metrics once, instead of averaging
    per-fold metric bundles. Candidates are built once per record (not per pair).
    """
    candidates = {r.record_id: record_to_candidate(r) for r in ds.all_records()}
    truth = truth_pairs(ds)

    outcomes: list[PairOutcome] = []
    for low, high in all_pairs(ds):
        comparisons = field_comparisons(candidates[low], candidates[high], config)
        match_score = score(comparisons, weights)
        outcomes.append(
            PairOutcome(
                is_match=(low, high) in truth,
                score_total=match_score.total,
                band=band(match_score, (), thresholds),
            )
        )
    return outcomes


def evaluate_scorer(
    ds: LabelledDataset,
    *,
    weights: Weights = DEFAULT_WEIGHTS,
    thresholds: Thresholds = DEFAULT_THRESHOLDS,
    config: ComparatorConfig = DEFAULT_CONFIG,
) -> ScorerMetrics:
    """Score every record pair, band it, and aggregate against ground truth."""
    return scorer_metrics(
        scorer_outcomes(ds, weights=weights, thresholds=thresholds, config=config)
    )
```

- [ ] **Step 3b: Verify the refactor preserved behavior**

Run: `cd matcher && uv run pytest tests/test_eval_scorer_driver.py -v`
Expected: PASS (the existing scorer-driver tests are unchanged behavior).

- [ ] **Step 3c: Write `crossval.py`**

Create `matcher/src/cairn_matcher/eval/crossval.py`:

```python
"""K-fold, entity-cluster held-out measurement of the weight learner (pure).

Honesty is the whole point of this module: reporting train-set lift as generalization
would be a precise untruth. So we (1) split on WHOLE entity clusters — never on pairs, or a
cluster's within-cluster match pairs would straddle the train/test boundary and leak truth
— and (2) report metrics only on the held-out fold, pooling held-out outcomes across folds.

Deterministic: clusters are striped by sorted entity_id (no RNG), matching the generator's
reproducibility contract.
"""

from collections.abc import Sequence
from dataclasses import dataclass

from cairn_matcher.eval.dataset import EntityCluster, LabelledDataset
from cairn_matcher.eval.learner import learn_model
from cairn_matcher.eval.metrics import PairOutcome, ScorerMetrics, scorer_metrics
from cairn_matcher.eval.report import format_scorer
from cairn_matcher.eval.scorer_eval import scorer_outcomes
from cairn_matcher.orchestrator import DEFAULT_CONFIG, ComparatorConfig


def split_clusters(ds: LabelledDataset, folds: int) -> list[LabelledDataset]:
    """Partition ds's entity clusters into `folds` disjoint LabelledDatasets, deterministic.

    Split on whole clusters so no cluster's within-cluster match pairs straddle folds. Stripe
    by SORTED entity_id (round-robin) — reproducible, and every cluster lands in exactly one
    fold. Raises if there are fewer clusters than folds, or fewer than 2 folds.
    """
    if folds < 2:
        raise ValueError(f"need at least 2 folds, got {folds}")
    ordered = sorted(ds.entities, key=lambda e: e.entity_id)
    if len(ordered) < folds:
        raise ValueError(f"{len(ordered)} clusters < {folds} folds")
    buckets: list[list[EntityCluster]] = [[] for _ in range(folds)]
    for i, ent in enumerate(ordered):
        buckets[i % folds].append(ent)
    return [
        LabelledDataset(
            name=f"{ds.name}#fold{i}", entities=tuple(b), description=ds.description
        )
        for i, b in enumerate(buckets)
    ]


def _union(parts: Sequence[LabelledDataset], name: str) -> LabelledDataset:
    """Concatenate several folds' clusters back into one training LabelledDataset."""
    entities = tuple(e for ds in parts for e in ds.entities)
    return LabelledDataset(name=name, entities=entities)


def _has_match_pairs(ds: LabelledDataset) -> bool:
    """True iff some cluster has >= 2 records (i.e. the dataset yields a true-match pair)."""
    return any(len(e.records) >= 2 for e in ds.entities)


@dataclass(frozen=True)
class LiftReport:
    """Pooled held-out before/after metrics from k-fold learning.

    before = the shipped defaults; after = the learned model. Both are computed on exactly
    the same held-out pairs, so the two ScorerMetrics are directly comparable. skipped_folds
    counts folds whose training partition had no match pairs to learn from (honest, not a
    crash) — those folds contribute to neither before nor after.
    """

    folds: int
    skipped_folds: int
    before: ScorerMetrics
    after: ScorerMetrics


def kfold_lift(
    ds: LabelledDataset,
    *,
    folds: int = 5,
    config: ComparatorConfig = DEFAULT_CONFIG,
    alpha: float = 0.5,
    recall_target: float = 0.99,
    margin: float = 0.5,
) -> LiftReport:
    """Learn on k-1 folds, measure on the held-out fold; pool held-out outcomes across all
    folds and report before (shipped defaults) vs after (learned). Never reports train-set
    metrics. A fold whose training partition has no match pairs is skipped and counted.
    """
    parts = split_clusters(ds, folds)
    before: list[PairOutcome] = []
    after: list[PairOutcome] = []
    skipped = 0
    for i, test in enumerate(parts):
        train = _union([p for j, p in enumerate(parts) if j != i], name=f"{ds.name}#train{i}")
        if not _has_match_pairs(train):
            skipped += 1
            continue
        model = learn_model(
            train, config=config, alpha=alpha, recall_target=recall_target, margin=margin
        )
        before += scorer_outcomes(test, config=config)  # shipped DEFAULT_WEIGHTS/THRESHOLDS
        after += scorer_outcomes(
            test, weights=model.weights, thresholds=model.thresholds, config=config
        )
    return LiftReport(
        folds=folds,
        skipped_folds=skipped,
        before=scorer_metrics(before),
        after=scorer_metrics(after),
    )


def format_lift(report: LiftReport, *, dataset_name: str = "") -> str:
    """Render a before/after lift report, reusing the scorer report for each block."""
    title = f"Weight-learning lift — {dataset_name}" if dataset_name else "Weight-learning lift"
    return "\n".join(
        [
            f"{title}  ({report.folds}-fold held-out, {report.skipped_folds} fold(s) skipped)",
            "--- BEFORE (shipped defaults) ---",
            format_scorer(report.before),
            "--- AFTER (learned) ---",
            format_scorer(report.after),
        ]
    )
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd matcher && uv run pytest tests/test_eval_crossval.py tests/test_eval_scorer_driver.py -v`
Expected: PASS (all crossval + scorer-driver tests green).

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/eval/scorer_eval.py matcher/src/cairn_matcher/eval/crossval.py matcher/tests/test_eval_crossval.py
git commit -m "feat(matcher): k-fold entity-cluster held-out lift measurement (+ scorer_outcomes extract)"
```

---

### Task 5: Model serialization (`eval/model_io.py`)

**Files:**
- Create: `matcher/src/cairn_matcher/eval/model_io.py`
- Test: `matcher/tests/test_eval_model_io.py`

**Interfaces:**
- Consumes: `scoring` (`Weights`, `FieldWeights`), `banding.Thresholds`, `agreement.AgreementLevel`, `eval/learner` (`LearnedModel`, `LearnMetadata`).
- Produces:
  - `ModelIOError(ValueError)`
  - `model_to_json(model: LearnedModel) -> dict`
  - `model_from_json(obj: Mapping) -> LearnedModel`
  - `write_model(model: LearnedModel, path) -> None`
  - `read_model(path) -> LearnedModel`

- [ ] **Step 1: Write the failing test**

Create `matcher/tests/test_eval_model_io.py`:

```python
"""Tests for LearnedModel JSON serialization (eval/model_io.py)."""

import pytest

from cairn_matcher.eval.learner import learn_model
from cairn_matcher.eval.loader import load_bundled_gold
from cairn_matcher.eval.model_io import (
    ModelIOError,
    model_from_json,
    model_to_json,
    read_model,
    write_model,
)
from cairn_matcher.eval.scorer_eval import evaluate_scorer


def test_round_trip_reconstructs_a_model_that_scores_identically():
    ds = load_bundled_gold()
    model = learn_model(ds)
    restored = model_from_json(model_to_json(model))
    before = evaluate_scorer(ds, weights=model.weights, thresholds=model.thresholds)
    after = evaluate_scorer(ds, weights=restored.weights, thresholds=restored.thresholds)
    assert before == after
    assert restored.metadata == model.metadata


def test_file_round_trip(tmp_path):
    model = learn_model(load_bundled_gold())
    path = tmp_path / "model.json"
    write_model(model, path)
    restored = read_model(path)
    assert restored.thresholds == model.thresholds
    assert restored.metadata == model.metadata


def test_unknown_agreement_level_rejected():
    bad = {
        "weights": {"dob": {"NOT_A_LEVEL": 1.0}},
        "thresholds": {"review": 1.0, "auto": 2.0},
        "metadata": {
            "alpha": 0.5, "recall_target": 0.99, "margin": 0.5,
            "train_pairs": 1, "train_matches": 1, "review_auto_collided": False,
        },
    }
    with pytest.raises(ModelIOError):
        model_from_json(bad)


def test_missing_top_level_key_rejected():
    with pytest.raises(ModelIOError):
        model_from_json({"weights": {}})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd matcher && uv run pytest tests/test_eval_model_io.py -v`
Expected: FAIL with `ModuleNotFoundError: No module named 'cairn_matcher.eval.model_io'`.

- [ ] **Step 3: Write minimal implementation**

Create `matcher/src/cairn_matcher/eval/model_io.py`:

```python
"""JSON serialization for a LearnedModel (pure value transforms + a thin file edge).

Lets a learned model be written to disk and reloaded into the exact production types
(scoring.Weights, banding.Thresholds) so a future deployment could adopt it. No pipeline
code reads these files yet — this is an advisory desk artifact. Malformed input raises
ModelIOError loudly rather than silently defaulting (house rule #5).
"""

import json
from collections.abc import Mapping

from cairn_matcher.agreement import AgreementLevel
from cairn_matcher.eval.learner import LearnedModel, LearnMetadata
from cairn_matcher.pipeline.banding import Thresholds
from cairn_matcher.scoring import FieldWeights, Weights

_META_FIELDS = (
    "alpha", "recall_target", "margin", "train_pairs", "train_matches", "review_auto_collided",
)


class ModelIOError(ValueError):
    """The model JSON is structurally invalid (bad shape, unknown level, missing key)."""


def _weights_to_json(weights: Weights) -> dict:
    """{field: {LEVEL_NAME: weight}} — agreement levels keyed by their stable enum NAME."""
    return {
        field: {level.name: w for level, w in fw.weights.items()}
        for field, fw in weights.per_field.items()
    }


def _weights_from_json(obj: Mapping) -> Weights:
    """Inverse of _weights_to_json; rejects any unknown agreement-level name."""
    per_field: dict[str, FieldWeights] = {}
    for field, levels in obj.items():
        table: dict[AgreementLevel, float] = {}
        for name, w in levels.items():
            try:
                level = AgreementLevel[name]
            except KeyError as exc:
                raise ModelIOError(
                    f"unknown agreement level {name!r} for field {field!r}"
                ) from exc
            table[level] = float(w)
        per_field[field] = FieldWeights(table)
    return Weights(per_field=per_field)


def model_to_json(model: LearnedModel) -> dict:
    """Serialize a LearnedModel to a plain JSON-ready dict (weights/thresholds/metadata)."""
    return {
        "weights": _weights_to_json(model.weights),
        "thresholds": {"review": model.thresholds.review, "auto": model.thresholds.auto},
        "metadata": {f: getattr(model.metadata, f) for f in _META_FIELDS},
    }


def model_from_json(obj: Mapping) -> LearnedModel:
    """Reconstruct a LearnedModel from a decoded JSON mapping; raise on any missing key."""
    for key in ("weights", "thresholds", "metadata"):
        if key not in obj:
            raise ModelIOError(f"model JSON missing top-level key {key!r}")
    thr = obj["thresholds"]
    meta = obj["metadata"]
    try:
        thresholds = Thresholds(review=float(thr["review"]), auto=float(thr["auto"]))
        metadata = LearnMetadata(**{f: meta[f] for f in _META_FIELDS})
    except (KeyError, TypeError) as exc:
        raise ModelIOError(f"malformed thresholds/metadata: {exc}") from exc
    return LearnedModel(
        weights=_weights_from_json(obj["weights"]),
        thresholds=thresholds,
        metadata=metadata,
    )


def write_model(model: LearnedModel, path) -> None:
    """Write a LearnedModel to `path` as UTF-8 JSON (sorted keys, deterministic)."""
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(model_to_json(model), fh, ensure_ascii=False, indent=2, sort_keys=True)


def read_model(path) -> LearnedModel:
    """Read and reconstruct a LearnedModel from a JSON file at `path`."""
    with open(path, encoding="utf-8") as fh:
        return model_from_json(json.load(fh))
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd matcher && uv run pytest tests/test_eval_model_io.py -v`
Expected: PASS (4 passed).

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/eval/model_io.py matcher/tests/test_eval_model_io.py
git commit -m "feat(matcher): LearnedModel JSON serialization (round-trips into production types)"
```

---

### Task 6: CLI (`eval/learn.py`) + smoke test

**Files:**
- Create: `matcher/src/cairn_matcher/eval/learn.py`
- Test: `matcher/tests/test_eval_learn_cli.py`

**Interfaces:**
- Consumes: `eval/loader` (`load_bundled_gold`, `load_dataset_file`), `eval/crossval` (`kfold_lift`, `format_lift`), `eval/learner.learn_model`, `eval/model_io.write_model`, `eval/dataset.DatasetError`.
- Produces: `main(argv: list[str] | None = None) -> int`.

- [ ] **Step 1: Write the failing test**

Create `matcher/tests/test_eval_learn_cli.py`:

```python
"""Smoke tests for the weight-learning CLI (python -m cairn_matcher.eval.learn)."""

from cairn_matcher.eval.learn import main
from cairn_matcher.eval.model_io import read_model


def test_cli_runs_on_bundled_gold_and_prints_before_after(capsys):
    rc = main(["--folds", "5"])
    assert rc == 0
    out = capsys.readouterr().out
    assert "BEFORE" in out and "AFTER" in out


def test_cli_writes_a_loadable_artifact(tmp_path):
    path = tmp_path / "model.json"
    rc = main(["--folds", "5", "--out", str(path)])
    assert rc == 0
    model = read_model(path)  # reloads without error
    assert model.thresholds.auto > model.thresholds.review


def test_cli_reports_a_bad_dataset_path_gracefully(capsys):
    rc = main(["/no/such/dataset.json"])
    assert rc == 2
    assert "error" in capsys.readouterr().err.lower()
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd matcher && uv run pytest tests/test_eval_learn_cli.py -v`
Expected: FAIL with `ModuleNotFoundError: No module named 'cairn_matcher.eval.learn'`.

- [ ] **Step 3: Write minimal implementation**

Create `matcher/src/cairn_matcher/eval/learn.py`:

```python
"""`python -m cairn_matcher.eval.learn [dataset.json]` — learn matcher weights + thresholds.

The disk/CLI edge for the pure learner (learner.py / crossval.py stay filesystem-free).
Prints a k-fold held-out before/after lift report; with --out, ALSO learns a model on the
full dataset and writes it as JSON. Defaults to the bundled gold set.

    python -m cairn_matcher.eval.learn --folds 5
    python -m cairn_matcher.eval.generate --entities 400 --seed 1 --out synth.json
    python -m cairn_matcher.eval.learn synth.json --folds 5 --out learned.json

PoC: ships the mechanism, not the shipped defaults (§5.13 / ADR-0014). See
docs/superpowers/specs/2026-07-06-b3-weight-learning-design.md for the honest limits.
"""

import argparse
import sys

from cairn_matcher.eval.crossval import format_lift, kfold_lift
from cairn_matcher.eval.dataset import DatasetError
from cairn_matcher.eval.learner import learn_model
from cairn_matcher.eval.loader import load_bundled_gold, load_dataset_file
from cairn_matcher.eval.model_io import write_model


def main(argv: list[str] | None = None) -> int:
    """Parse args, run the k-fold lift, print it, optionally write a full-data model.

    Returns a process exit code: 0 on success, 2 if the dataset could not be loaded.
    """
    parser = argparse.ArgumentParser(prog="cairn_matcher.eval.learn", description=__doc__)
    parser.add_argument(
        "dataset", nargs="?",
        help="path to a dataset JSON file; default: the bundled gold_v1 set",
    )
    parser.add_argument("--folds", type=int, default=5, help="k-fold count (>= 2)")
    parser.add_argument("--recall-target", type=float, default=0.99,
                        help="fraction of true matches the review threshold must surface")
    parser.add_argument("--margin", type=float, default=0.5,
                        help="added above max non-match score for the auto threshold")
    parser.add_argument("--alpha", type=float, default=0.5,
                        help="Laplace smoothing pseudo-count (> 0)")
    parser.add_argument("--out", help="write a full-dataset learned model to this JSON path")
    args = parser.parse_args(argv)

    try:
        ds = load_dataset_file(args.dataset) if args.dataset else load_bundled_gold()
    except (DatasetError, OSError, ValueError) as exc:
        print(f"error: could not load dataset: {exc}", file=sys.stderr)
        return 2

    report = kfold_lift(
        ds, folds=args.folds, alpha=args.alpha,
        recall_target=args.recall_target, margin=args.margin,
    )
    print(format_lift(report, dataset_name=ds.name))

    if args.out:
        model = learn_model(
            ds, alpha=args.alpha, recall_target=args.recall_target, margin=args.margin,
        )
        write_model(model, args.out)
        print(f"\nwrote full-dataset learned model to {args.out}")
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd matcher && uv run pytest tests/test_eval_learn_cli.py -v`
Expected: PASS (3 passed).

- [ ] **Step 5: Run the whole pure suite + ruff, then commit**

```bash
cd matcher && uv run pytest && uv run ruff check .
```
Expected: all pure tests pass, ruff clean.

```bash
git add matcher/src/cairn_matcher/eval/learn.py matcher/tests/test_eval_learn_cli.py
git commit -m "feat(matcher): weight-learning CLI (python -m cairn_matcher.eval.learn)"
```

---

## Self-Review

**Spec coverage:**
- §3.1 weight estimation → Task 1. §3.1 smoothing / zero-count → Task 1 (`test_zero_count_cell_is_bounded_not_infinite`). §3.1 field-coverage guarantee → Task 1 (`test_every_seen_field_gets_an_entry...`). §3.1 INSUFFICIENT_DATA excluded → Task 1.
- §3.2 provenance orthogonal → inherent (estimation counts levels; `score()` applies `provenance_factor` unchanged). Verified indirectly by Task 3's `evaluate_scorer` round-trip.
- §3.3 safety-first thresholds (zero-false-auto, recall floor, `review<auto` flag) → Task 2. Coupling / re-derivation → Task 3.
- §4 honest measurement (cluster split, deterministic, held-out, no train-set metrics) → Task 4. Fold-skip robustness (gold's imbalance) → Task 4 (`test_kfold_lift_skips_a_fold...`).
- §5 model artifact + CLI → Task 5 (`model_io`) + Task 6 (`learn.py`).
- §6 files → learner.py (T1–3), crossval.py (T4), model_io.py (T5), learn.py (T6). All < 500 lines.
- §7 testing → each task's test file matches the spec's test list.
- §8 honest limits → documented in `learner.py`, `crossval.py`, `model_io.py`, `learn.py` docstrings + the design doc (already committed).

**Placeholder scan:** none — every step carries full test + implementation code.

**Type consistency:** `LearnedModel`/`LearnMetadata`/`LearnMetadata._META_FIELDS` names align across Tasks 3/5. `scorer_outcomes` signature (Task 4) matches its use in `crossval.kfold_lift`. `Thresholds(review, auto)`, `Weights(per_field=...)`, `FieldWeights(dict)`, `AgreementLevel[...]` all match the existing production types read in Read of `scoring.py`/`banding.py`/`agreement.py`. `LabelledPair` used consistently in Tasks 1/3.

**One deliberate design→plan refinement:** the design's §6 file table named the core `learn.py`; the plan splits it into pure core `learner.py` + CLI `learn.py`, mirroring the existing `generator.py` (pure) / `generate.py` (CLI) pattern exactly. Same modules, clearer separation.
