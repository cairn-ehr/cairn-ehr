"""Turn a list of per-pair outcomes into scorer/banding quality metrics.

Pure: no scoring, no DB. The scorer eval (scorer_eval.py) produces the PairOutcome list
by running the real pipeline over a dataset; this module just counts and divides. It
imports only Band (from the psycopg-free banding module), keeping the metric core pure.

Zero-denominator convention (no NaNs ever): precision is 0.0 when nothing is predicted
positive; recall is 0.0 when there are no true matches; F1 is 0.0 when precision+recall
is 0; each rate is 0.0 when its denominator is 0.
"""

import statistics
from collections.abc import Sequence
from dataclasses import dataclass

from cairn_matcher.pipeline.banding import Band


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


@dataclass(frozen=True)
class OperatingPoint:
    """Precision/recall/F1 at one band cut-off (strict = auto; lenient = auto|review)."""

    name: str
    precision: float
    recall: float
    f1: float


@dataclass(frozen=True)
class ScoreStats:
    """Score spread for one truth class — the overlap a weight change must reduce."""

    count: int
    minimum: float
    median: float
    maximum: float


@dataclass(frozen=True)
class Confusion:
    """The 2x3 truth (match/nonmatch) x band (auto/review/none) contingency table."""

    match_auto: int
    match_review: int
    match_none: int
    nonmatch_auto: int
    nonmatch_review: int
    nonmatch_none: int


@dataclass(frozen=True)
class ScorerMetrics:
    """Everything the scorer report shows; all derived purely from the outcome list."""

    confusion: Confusion
    strict: OperatingPoint
    lenient: OperatingPoint
    auto_false_link_rate: float
    missed_match_rate: float
    match_scores: ScoreStats
    nonmatch_scores: ScoreStats
    pair_count: int
    repaired_match_pairs: int


def _ratio(numerator: float, denominator: float) -> float:
    """Guarded division: 0.0 when the denominator is 0 (never a NaN/ZeroDivisionError)."""
    return numerator / denominator if denominator else 0.0


def _band_label(band: Band | None) -> str:
    """Collapse a Band (or None) to one of the three confusion column keys."""
    if band is Band.AUTO_CANDIDATE:
        return "auto"
    if band is Band.REVIEW:
        return "review"
    return "none"


def _operating_point(name: str, tp: int, fp: int, fn: int) -> OperatingPoint:
    """Precision/recall/F1 from true/false positives and false negatives, all guarded."""
    precision = _ratio(tp, tp + fp)
    recall = _ratio(tp, tp + fn)
    f1 = _ratio(2 * precision * recall, precision + recall)
    return OperatingPoint(name=name, precision=precision, recall=recall, f1=f1)


def _score_stats(scores: Sequence[float]) -> ScoreStats:
    """min/median/max over one class's scores; all-zero on an empty class (safe)."""
    if not scores:
        return ScoreStats(count=0, minimum=0.0, median=0.0, maximum=0.0)
    return ScoreStats(
        count=len(scores),
        minimum=min(scores),
        median=statistics.median(scores),
        maximum=max(scores),
    )


def scorer_metrics(outcomes: Sequence[PairOutcome]) -> ScorerMetrics:
    """Aggregate per-pair outcomes into the full scorer metric bundle.

    Two operating points are reported because the matcher is two-tiered: 'strict' counts
    only AUTO_CANDIDATE as a predicted link (the aggressive end), 'lenient' also counts
    REVIEW (a human will look). auto_false_link_rate is the dangerous one — the fraction
    of auto-banded pairs that are actually non-matches; it should be ~0 for a sane config.
    """
    cells = {(m, lbl): 0 for m in (True, False) for lbl in ("auto", "review", "none")}
    match_scores: list[float] = []
    nonmatch_scores: list[float] = []
    for o in outcomes:
        cells[(o.is_match, _band_label(o.band))] += 1
        (match_scores if o.is_match else nonmatch_scores).append(o.score_total)

    confusion = Confusion(
        match_auto=cells[(True, "auto")],
        match_review=cells[(True, "review")],
        match_none=cells[(True, "none")],
        nonmatch_auto=cells[(False, "auto")],
        nonmatch_review=cells[(False, "review")],
        nonmatch_none=cells[(False, "none")],
    )

    strict = _operating_point(
        "strict",
        tp=confusion.match_auto,
        fp=confusion.nonmatch_auto,
        fn=confusion.match_review + confusion.match_none,
    )
    lenient = _operating_point(
        "lenient",
        tp=confusion.match_auto + confusion.match_review,
        fp=confusion.nonmatch_auto + confusion.nonmatch_review,
        fn=confusion.match_none,
    )

    total_auto = confusion.match_auto + confusion.nonmatch_auto
    total_true = confusion.match_auto + confusion.match_review + confusion.match_none

    # Report (never drop) the true pairs a synthetic repair made artificially easy (#290): the
    # metric intersects the structural `repaired` flag with is_match, so a repaired record's
    # cross-cluster non-match pairs never inflate the count.
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
