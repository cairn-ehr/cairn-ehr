import pytest

from cairn_matcher.agreement import AgreementLevel, Context
from cairn_matcher.comparators import compare_edit_distance, compare_exact
from cairn_matcher.records import MatcherTypeError

CTX = Context()


def test_exact_agrees_after_trim():
    assert compare_exact("  smith ", "smith", CTX) is AgreementLevel.EXACT


def test_exact_disagrees_on_different_values():
    assert compare_exact("smith", "jones", CTX) is AgreementLevel.DISAGREE


def test_exact_does_not_casefold():
    # Casefolding is culture-touching; the core does not do it.
    assert compare_exact("Smith", "smith", CTX) is AgreementLevel.DISAGREE


def test_missing_side_is_insufficient_data():
    assert compare_exact(None, "smith", CTX) is AgreementLevel.INSUFFICIENT_DATA
    assert compare_edit_distance("smith", None, CTX) is AgreementLevel.INSUFFICIENT_DATA


def test_edit_distance_exact_when_identical():
    assert compare_edit_distance("martha", "martha", CTX) is AgreementLevel.EXACT


def test_edit_distance_grades_close_pair_within_band():
    # martha/marhta ~ 0.961 >= 0.90 default threshold
    assert compare_edit_distance("martha", "marhta", CTX) is AgreementLevel.EDIT_DISTANCE


def test_edit_distance_disagrees_below_band():
    assert compare_edit_distance("smith", "jones", CTX) is AgreementLevel.DISAGREE


def test_edit_distance_threshold_is_configurable():
    loose = Context(edit_distance_threshold=0.80)
    # dwayne/duane ~ 0.840: DISAGREE at 0.90, EDIT_DISTANCE at 0.80
    assert compare_edit_distance("dwayne", "duane", CTX) is AgreementLevel.DISAGREE
    assert compare_edit_distance("dwayne", "duane", loose) is AgreementLevel.EDIT_DISTANCE


def test_wrong_type_raises():
    with pytest.raises(MatcherTypeError):
        compare_exact(5, "smith", CTX)
