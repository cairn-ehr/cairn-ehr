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
    (D(1999, 6, 30), D(2000, 6, 30), YEARS_ONE_APART),  # four digits change: the carry
    (D(1909, 6, 30), D(1910, 6, 30), YEARS_ONE_APART),  # rule 4 exists for exactly these
    (D(1950, 1, 7), D(1951, 1, 7), ONE_DIGIT),          # the one rule overlap: rule 1 names it
    (D(1951, 1, 7), D(1950, 11, 7), ADJACENT_SWAP),     # across the ISO year/month boundary
    (D(1950, 12, 7), D(1950, 10, 27), ADJACENT_SWAP),   # across the ISO month/day boundary
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
    (D(1959, 1, 7), D(1961, 1, 7)),      # years two apart, two digits changed (#761)
    (D(1949, 12, 31), D(1950, 1, 1)),    # New Year's Eve vs Day: a day apart, many digits
    (D(1950, 1, 9), D(1950, 1, 10)),     # one day apart, two digits changed
    (D(1950, 11, 10), D(1950, 1, 11)),   # a day-first "10/11" keyed "11/01": not adjacent in ISO
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


def test_near_is_weak_positive_evidence_below_exact():
    # +2: enough that a typo-close DOB plus an agreeing name reaches review even when the
    # names agree only partially (a middle name on one chart), never enough to stand in for
    # an exact DOB. NEAR can never auto-link whatever its price (banding caps it).
    dob = DEFAULT_WEIGHTS.per_field["dob"].weights
    assert dob[AgreementLevel.NEAR] == 2.0
    assert dob[AgreementLevel.DISAGREE] < dob[AgreementLevel.NEAR] < dob[AgreementLevel.EXACT]


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


@pytest.mark.parametrize("a, b", [
    (D(-1, 1, 1), D(1, 1, 1)),           # a negative year renders as "-001": a false digit match
    (D(10000, 1, 1), D(1000, 1, 1)),     # five year digits would misalign the eight
    (D(1950, 31, 1), D(1950, 1, 31)),    # month 31 is no date: a "swap" into an impossible date
    (D(1950, 1, 0), D(1950, 1, 1)),      # day 0
])
def test_a_date_that_cannot_exist_is_never_a_keying_error(a, b):
    # parse_dob never builds these, but compare_dob is public: an impossible date must never
    # be named a typo of a real one (#761).
    assert keying_error_between(a, b) is None
    assert keying_error_between(b, a) is None


def test_keying_errors_are_symmetric_and_agree_with_compare_dob_randomized():
    # Seeded sweep over dates clustered so that keying errors are common. For two full point
    # dates: k(a, b) == k(b, a); k(a, a) is None; compare_dob is NEAR exactly when k names one.
    import random
    rng = random.Random(7)

    def date():
        return D(rng.choice((1909, 1910, 1950, 1951, 1959, 1960, 1999, 2000)),
                 rng.choice((1, 2, 7, 10, 11, 12)), rng.choice((1, 2, 7, 10, 11, 12, 21, 27)))

    for _ in range(5000):
        a, b = date(), date()
        k = keying_error_between(a, b)
        assert k == keying_error_between(b, a), (a, b)
        assert keying_error_between(a, a) is None
        if a != b:
            assert (compare_dob(a, b, CTX) is AgreementLevel.NEAR) == (k is not None), (a, b)


def test_a_middle_name_and_a_day_month_swap_reach_review_at_full_provenance():
    # Mary Smith vs Mary Jane Smith (names agree PARTIAL, priced 0 by default) with the DOB
    # keyed day-first on one chart: 0 + NEAR 2 + sex 1 = 3.0, the review floor. At NEAR 0 this
    # pair was withdrawn unseen. Provenance below 70 scales it under the floor again — the
    # name PARTIAL price is the lever for that, not NEAR (see ADR-0079 Consequences).
    from cairn_matcher.orchestrator import field_comparisons
    from cairn_matcher.pipeline.banding import Band, band
    from cairn_matcher.records import CandidateRecord, FieldValue, Name

    def rec(given, d):
        return CandidateRecord(
            dob=FieldValue(d, 70), sex_at_birth=FieldValue("female", 70),
            names=FieldValue(frozenset({Name({"given": given, "family": ("smith",)})}), 70))

    s = score(field_comparisons(rec(("mary",), D(1950, 1, 7)),
                                rec(("mary", "jane"), D(1950, 7, 1))))
    assert s.total == 3.0
    assert band(s, ()) is Band.REVIEW
