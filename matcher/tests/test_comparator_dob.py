import pytest

from cairn_matcher.agreement import AgreementLevel, Context
from cairn_matcher.comparators import compare_dob
from cairn_matcher.records import DateValue, MatcherTypeError

CTX = Context()


def test_identical_full_dates_are_exact():
    assert compare_dob(DateValue(1980, 3, 15), DateValue(1980, 3, 15), CTX) is AgreementLevel.EXACT


def test_year_only_vs_full_is_partial():
    # 1980 (year precision) vs 1980-03-15 (day precision): consistent coarsening.
    assert compare_dob(DateValue(1980), DateValue(1980, 3, 15), CTX) is AgreementLevel.PARTIAL


def test_shared_part_differs_is_disagree():
    assert (
        compare_dob(DateValue(1980, 3, 15), DateValue(1980, 3, 16), CTX)
        is AgreementLevel.DISAGREE
    )
    assert compare_dob(DateValue(1980), DateValue(1981, 3, 15), CTX) is AgreementLevel.DISAGREE


def test_missing_side_is_insufficient_data():
    assert compare_dob(None, DateValue(1980, 3, 15), CTX) is AgreementLevel.INSUFFICIENT_DATA


def test_no_overlapping_parts_is_insufficient_data():
    # One has only a year, the other only a day-of-month: nothing comparable in common.
    assert (
        compare_dob(DateValue(year=1980), DateValue(day=15), CTX)
        is AgreementLevel.INSUFFICIENT_DATA
    )


def test_is_symmetric():
    a, b = DateValue(1980), DateValue(1980, 3, 15)
    assert compare_dob(a, b, CTX) is compare_dob(b, a, CTX)


def test_wrong_type_raises():
    with pytest.raises(MatcherTypeError):
        compare_dob("1980-03-15", DateValue(1980, 3, 15), CTX)
