"""A DateValue can carry an inclusive birth-year interval (year_min..year_max)."""
from cairn_matcher.records import DateValue


def test_point_date_is_not_a_range():
    assert DateValue(year=1985, month=3, day=12).is_range is False


def test_year_interval_is_a_range():
    dv = DateValue(year_min=1981, year_max=1991)
    assert dv.is_range is True
    assert dv.year_min == 1981
    assert dv.year_max == 1991
    # A range has no point parts.
    assert dv.year is None and dv.month is None and dv.day is None
