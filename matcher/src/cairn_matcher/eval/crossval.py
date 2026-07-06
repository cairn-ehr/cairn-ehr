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
