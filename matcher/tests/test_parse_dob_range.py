"""parse_dob understands the §5.4 'year-range' precision (e.g. '1981/1991')."""
from cairn_matcher.pipeline.adapter import parse_dob


def test_year_range_parses_to_an_interval():
    dv = parse_dob("1981/1991", "year-range")
    assert dv is not None and dv.is_range
    assert (dv.year_min, dv.year_max) == (1981, 1991)


def test_single_year_range_is_a_degenerate_interval():
    dv = parse_dob("1990/1990", "year-range")
    assert (dv.year_min, dv.year_max) == (1990, 1990)


def test_reversed_or_malformed_range_degrades_to_none():
    assert parse_dob("1991/1981", "year-range") is None      # min > max
    assert parse_dob("1981", "year-range") is None            # no separator
    assert parse_dob("1981/xx", "year-range") is None         # non-numeric
    assert parse_dob("81/91", "year-range") is None           # not 4-digit years


def test_point_precision_still_parses_a_point_date():
    dv = parse_dob("1985-03-12", "day")
    assert dv is not None and not dv.is_range
    assert (dv.year, dv.month, dv.day) == (1985, 3, 12)
