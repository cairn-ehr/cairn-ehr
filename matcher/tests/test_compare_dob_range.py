"""compare_dob is range-aware and positive-only for clinician-observed estimates (§5.4)."""
from cairn_matcher.agreement import AgreementLevel, Context
from cairn_matcher.comparators import compare_dob
from cairn_matcher.records import DateValue

CTX = Context()  # compare_dob does not read ctx for ranges; a default Context is fine


def _point(y, m=None, d=None):
    return DateValue(year=y, month=m, day=d)


def _range(lo, hi):
    return DateValue(year_min=lo, year_max=hi)


def test_point_inside_range_is_partial():
    assert compare_dob(_range(1981, 1991), _point(1985, 3, 12), CTX) == AgreementLevel.PARTIAL
    # order-independent
    assert compare_dob(_point(1985, 3, 12), _range(1981, 1991), CTX) == AgreementLevel.PARTIAL


def test_point_outside_range_is_insufficient_never_disagree():
    got = compare_dob(_range(1981, 1991), _point(1950, 1, 1), CTX)
    assert got == AgreementLevel.INSUFFICIENT_DATA


def test_overlapping_ranges_are_partial():
    assert compare_dob(_range(1981, 1991), _range(1988, 1995), CTX) == AgreementLevel.PARTIAL


def test_disjoint_ranges_are_insufficient():
    assert (
        compare_dob(_range(1981, 1991), _range(2000, 2005), CTX)
        == AgreementLevel.INSUFFICIENT_DATA
    )
    # order-independent for the disjoint case too (symmetry of the overlap test)
    assert (
        compare_dob(_range(2000, 2005), _range(1981, 1991), CTX)
        == AgreementLevel.INSUFFICIENT_DATA
    )


def test_touching_boundary_ranges_are_partial():
    # Intervals that share exactly one endpoint STILL overlap: the overlap test is
    # inclusive (max(lo) <= min(hi)). This pins the inclusive-boundary decision so an
    # accidental switch to exclusive overlap (which would silently drop a real match
    # for the returning John Doe) trips this test. Asserted both orders (symmetry).
    assert compare_dob(_range(1981, 1991), _range(1991, 2000), CTX) == AgreementLevel.PARTIAL
    assert compare_dob(_range(1991, 2000), _range(1981, 1991), CTX) == AgreementLevel.PARTIAL
    # a point exactly on the range boundary is inside the (inclusive) range
    assert compare_dob(_range(1981, 1991), _point(1991), CTX) == AgreementLevel.PARTIAL


def test_range_vs_point_with_no_year_is_insufficient():
    assert compare_dob(_range(1981, 1991), _point(None), CTX) == AgreementLevel.INSUFFICIENT_DATA


def test_point_vs_point_regression_unchanged():
    assert compare_dob(_point(1985, 3, 12), _point(1985, 3, 12), CTX) == AgreementLevel.EXACT
    assert compare_dob(_point(1985), _point(1985, 3, 12), CTX) == AgreementLevel.PARTIAL
    assert compare_dob(_point(1985), _point(1990), CTX) == AgreementLevel.DISAGREE
