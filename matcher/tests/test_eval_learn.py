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
