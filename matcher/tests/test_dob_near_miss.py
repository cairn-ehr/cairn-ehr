"""A DOB one keying error away is NEAR, not DISAGREE (spec 2026-10-11, ADR-0079). Pure."""

import pytest

from cairn_matcher.agreement import AgreementLevel, Context
from cairn_matcher.comparators import (
    ADJACENT_SWAP,
    DAY_MONTH_SWAP,
    ONE_DIGIT,
    YEARS_ONE_APART,
    compare_dob,
    keying_error_between,
)
from cairn_matcher.records import DateValue, FieldComparison
from cairn_matcher.scoring import DEFAULT_WEIGHTS, FieldWeights, Weights, score

CTX = Context()
D = DateValue


@pytest.mark.parametrize("a, b, pattern", [
    (D(1950, 1, 7), D(1950, 1, 8), ONE_DIGIT),          # day digit
    (D(1950, 1, 7), D(1960, 1, 7), ONE_DIGIT),          # a year digit: a decade off
    (D(1980, 3, 12), D(1980, 3, 21), ADJACENT_SWAP),    # day 12 <-> 21
    (D(1958, 3, 12), D(1985, 3, 12), ADJACENT_SWAP),    # year 58 <-> 85
    (D(1950, 7, 1), D(1950, 1, 7), DAY_MONTH_SWAP),     # 01/07 <-> 07/01
    (D(1950, 11, 1), D(1950, 1, 11), DAY_MONTH_SWAP),
    (D(1959, 6, 30), D(1960, 6, 30), YEARS_ONE_APART),  # not one digit: 59 -> 60
])
def test_each_keying_error_is_named(a, b, pattern):
    assert keying_error_between(a, b) == pattern
    assert keying_error_between(b, a) == pattern  # symmetric
    assert compare_dob(a, b, CTX) is AgreementLevel.NEAR
    assert compare_dob(b, a, CTX) is AgreementLevel.NEAR


@pytest.mark.parametrize("a, b", [
    (D(1950, 1, 7), D(1951, 1, 8)),      # two places differ (year and day)
    (D(1980, 3, 12), D(1980, 3, 30)),    # two day digits differ, not a swap
    (D(1950, 7, 1), D(1951, 1, 7)),      # day/month swap across different years
    (D(1950, 1, 7), D(1970, 1, 9)),      # two digits, non-adjacent
])
def test_more_than_one_keying_error_stays_disagree(a, b):
    assert keying_error_between(a, b) is None
    assert compare_dob(a, b, CTX) is AgreementLevel.DISAGREE


def test_identical_dates_are_exact_never_near():
    assert keying_error_between(D(1958, 3, 12), D(1958, 3, 12)) is None
    assert compare_dob(D(1958, 3, 12), D(1958, 3, 12), CTX) is AgreementLevel.EXACT


def test_years_two_apart_with_one_digit_changed_is_one_digit():
    # 1950 -> 1952 changes ONE digit: a keying error by the one-digit rule, though "years one
    # apart" does not apply. Pinned so nobody "fixes" it into DISAGREE.
    assert keying_error_between(D(1950, 1, 7), D(1952, 1, 7)) == ONE_DIGIT


@pytest.mark.parametrize("a, b, expected", [
    (D(1950, 7), D(1950, 7, 1), AgreementLevel.PARTIAL),      # month precision vs full
    (D(1950), D(1951, 1, 7), AgreementLevel.DISAGREE),        # year only, year differs
    (D(1950, 7), D(1950, 8, 1), AgreementLevel.DISAGREE),     # month precision, month differs
    (D(year_min=1945, year_max=1955), D(1950, 1, 7), AgreementLevel.PARTIAL),  # §5.4 range
])
def test_near_needs_a_full_date_on_both_sides(a, b, expected):
    assert keying_error_between(a, b) is None
    assert compare_dob(a, b, CTX) is expected


def test_near_is_priced_at_zero_explicitly():
    assert DEFAULT_WEIGHTS.per_field["dob"].weights[AgreementLevel.NEAR] == 0.0


def test_a_learned_table_without_near_still_prices_it_at_zero():
    # An older persisted (B3-learned) table has no NEAR key; score() prices a level missing
    # WITHIN a field's entry at 0.0 — exactly the shipped behaviour, never a crash.
    legacy = Weights(per_field={"dob": FieldWeights({
        AgreementLevel.EXACT: 6.0, AgreementLevel.PARTIAL: 1.5, AgreementLevel.DISAGREE: -4.0,
    })})
    s = score([FieldComparison("dob", AgreementLevel.NEAR, 80)], legacy)
    assert s.total == 0.0


def test_the_package_version_moved_with_the_comparator():
    # matcher_version = package version + a digest of weights/thresholds/config. Comparator
    # CODE is not in the digest, so a behaviour change must move the version (spec: Rollout).
    import cairn_matcher
    assert cairn_matcher.__version__ == "0.2.0"
