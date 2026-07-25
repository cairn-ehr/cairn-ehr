"""Pure tests for the scorer metric math (no scoring, no DB — hand-built outcomes)."""

from cairn_matcher.eval.metrics import PairOutcome, scorer_metrics
from cairn_matcher.pipeline.banding import Band


def _o(is_match, total, band):
    return PairOutcome(is_match=is_match, score_total=total, band=band)


def test_confusion_counts_each_cell():
    outcomes = [
        _o(True, 10.0, Band.AUTO_CANDIDATE),
        _o(True, 5.0, Band.REVIEW),
        _o(True, 1.0, None),
        _o(False, 9.0, Band.AUTO_CANDIDATE),
        _o(False, 4.0, Band.REVIEW),
        _o(False, 0.0, None),
    ]
    m = scorer_metrics(outcomes)
    c = m.confusion
    assert (c.match_auto, c.match_review, c.match_none) == (1, 1, 1)
    assert (c.nonmatch_auto, c.nonmatch_review, c.nonmatch_none) == (1, 1, 1)
    assert m.pair_count == 6


def test_strict_and_lenient_operating_points():
    # 2 true matches: one auto, one review. 1 non-match: auto (a false link).
    outcomes = [
        _o(True, 10.0, Band.AUTO_CANDIDATE),
        _o(True, 5.0, Band.REVIEW),
        _o(False, 9.0, Band.AUTO_CANDIDATE),
    ]
    m = scorer_metrics(outcomes)
    # strict: positive == auto. TP=1, FP=1, FN=1 -> P=0.5, R=0.5
    assert m.strict.precision == 0.5
    assert m.strict.recall == 0.5
    # lenient: positive == auto|review. TP=2, FP=1, FN=0 -> P=2/3, R=1.0
    assert abs(m.lenient.precision - 2 / 3) < 1e-9
    assert m.lenient.recall == 1.0


def test_auto_false_link_and_missed_match_rates():
    outcomes = [
        _o(True, 10.0, Band.AUTO_CANDIDATE),
        _o(True, 1.0, None),                 # a missed true match
        _o(False, 9.0, Band.AUTO_CANDIDATE), # a false auto-link
    ]
    m = scorer_metrics(outcomes)
    assert m.auto_false_link_rate == 0.5      # 1 of 2 auto pairs is a non-match
    assert m.missed_match_rate == 0.5         # 1 of 2 true matches banded None


def test_score_separation_stats_per_class():
    outcomes = [
        _o(True, 10.0, Band.AUTO_CANDIDATE),
        _o(True, 6.0, Band.REVIEW),
        _o(False, 2.0, None),
    ]
    m = scorer_metrics(outcomes)
    assert m.match_scores.count == 2
    assert m.match_scores.minimum == 6.0
    assert m.match_scores.maximum == 10.0
    assert m.match_scores.median == 8.0
    assert m.nonmatch_scores.count == 1


def test_zero_denominators_yield_zero_not_nan():
    # No predicted positives, no true matches: every guarded ratio must be 0.0.
    m = scorer_metrics([PairOutcome(is_match=False, score_total=0.0, band=None)])
    assert m.strict.precision == 0.0
    assert m.strict.recall == 0.0
    assert m.strict.f1 == 0.0
    assert m.auto_false_link_rate == 0.0
    assert m.missed_match_rate == 0.0


def test_empty_outcomes_are_safe():
    m = scorer_metrics([])
    assert m.pair_count == 0
    assert m.match_scores.count == 0


def test_pair_outcome_repaired_defaults_false():
    assert PairOutcome(is_match=True, score_total=1.0, band=None).repaired is False


def test_scorer_metrics_counts_only_repaired_true_matches():
    outcomes = [
        PairOutcome(is_match=True, score_total=9.0, band=Band.AUTO_CANDIDATE, repaired=True),
        PairOutcome(is_match=True, score_total=8.0, band=Band.AUTO_CANDIDATE),  # match, unmarked
        PairOutcome(is_match=False, score_total=1.0, band=None, repaired=True),  # non-match: skip
    ]
    assert scorer_metrics(outcomes).repaired_match_pairs == 1


def test_scorer_metrics_repaired_zero_when_nothing_marked():
    m = scorer_metrics([PairOutcome(is_match=True, score_total=8.0, band=None)])
    assert m.repaired_match_pairs == 0
