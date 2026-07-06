"""Run the real scoring+banding pipeline over a labelled dataset -> ScorerMetrics.

Pure: it reuses the production scoring path (orchestrator -> scoring -> banding), so the
metrics describe the real matcher, not a stand-in. weights/thresholds/config are
PARAMETERS — sweeping them is exactly how weight-learning will use this harness.

Caveat (documented in the spec): banding is called with NO vetoes here. The pure eval
measures scorer+threshold quality in isolation; the in-DB veto can cap a high score at
REVIEW, so these metrics are slightly optimistic vs the end-to-end banded outcome. A
veto-aware mode is a later, additive extension.

Complexity is O(N^2) in records (every pair scored). Fine for the small gold set; that
O(N^2) is precisely what the blocking layer measures how to avoid at scale.
"""

from cairn_matcher.eval.dataset import (
    LabelledDataset,
    all_pairs,
    record_to_candidate,
    truth_pairs,
)
from cairn_matcher.eval.metrics import PairOutcome, ScorerMetrics, scorer_metrics
from cairn_matcher.orchestrator import DEFAULT_CONFIG, ComparatorConfig, field_comparisons
from cairn_matcher.pipeline.banding import DEFAULT_THRESHOLDS, Thresholds, band
from cairn_matcher.scoring import DEFAULT_WEIGHTS, Weights, score


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
