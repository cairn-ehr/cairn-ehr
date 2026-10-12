"""parse_dob extracts ISO date fields at the precision the projection declares.

The patient_demographic dob `value` is ISO-8601 by the cairn-event write convention
(`1980-07-15` day, `1980-07` month, `1980` year) and `facets.precision` declares how
precise it is. parse_dob is NOT a locale date parser — it reads the fields ISO already
exposes, gated by the declared precision. Anything it cannot read at that precision
degrades to None (absence is safe; principle 4), never a wrong DateValue.
"""

import pytest

from cairn_matcher.pipeline.adapter import parse_dob
from cairn_matcher.records import DateValue


def test_day_precision_parses_full_date():
    assert parse_dob("1980-07-15", "day") == DateValue(year=1980, month=7, day=15)


def test_month_precision_parses_year_and_month():
    assert parse_dob("1980-07", "month") == DateValue(year=1980, month=7, day=None)


def test_year_precision_parses_year_only():
    assert parse_dob("1980", "year") == DateValue(year=1980, month=None, day=None)


def test_absent_value_or_precision_is_none():
    assert parse_dob(None, "day") is None
    assert parse_dob("1980-07-15", None) is None


def test_non_iso_value_degrades_to_none():
    # A non-conformant peer wrote a locale string; we never guess. Safe degrade.
    assert parse_dob("15/07/1980", "day") is None
    assert parse_dob("not-a-date", "year") is None


def test_value_too_coarse_for_declared_precision_degrades():
    # precision says day but only a year is present -> cannot honour the claim -> None.
    assert parse_dob("1980", "day") is None


def test_unknown_precision_token_degrades_to_none():
    assert parse_dob("1980-07-15", "hour") is None


def test_out_of_range_month_degrades_to_none():
    # Numeric but not a real month -> a wrong DateValue, so degrade (honour the contract).
    assert parse_dob("1980-13-01", "month") is None
    assert parse_dob("1980-00-01", "month") is None


def test_out_of_range_day_degrades_to_none():
    # Numeric but not a real day-of-month -> degrade rather than emit a wrong DateValue.
    assert parse_dob("1980-07-45", "day") is None
    assert parse_dob("1980-07-00", "day") is None


def test_two_digit_year_degrades_to_none():
    # A legacy import wrote a 2-digit year ("80"). Accepting it would emit DateValue(80),
    # which then grades DISAGREE (the largest negative weight) against the SAME person's
    # 1980 record — a fabricated clash from a formatting artefact. The SQL blocking layer
    # already requires a 4-digit year run; the extractor must match that discipline.
    assert parse_dob("80", "year") is None
    assert parse_dob("80-07-15", "day") is None


def test_non_four_digit_year_degrades_to_none():
    assert parse_dob("198-07-15", "day") is None
    assert parse_dob("19800-07-15", "day") is None


def test_a_day_date_is_iso_digits_and_nothing_after():
    # The name+monthday blocking key reads this grammar on ASCII input; the two must agree, or
    # a pair the scorer can grade is never generated (or a junk value forms a key).
    assert parse_dob(" 1980-07-15 ", "day") == DateValue(year=1980, month=7, day=15)
    assert parse_dob("\t1980-07-15\r\n", "day") == DateValue(year=1980, month=7, day=15)
    assert parse_dob("1980-7-5", "day") == DateValue(year=1980, month=7, day=5)
    assert parse_dob("1980-07-15-x", "day") is None
    assert parse_dob("1980-07-15T00:00", "day") is None
    assert parse_dob("1980-07- 15", "day") is None
    assert parse_dob("1980-007-15", "day") is None
    # Digits in the right shape but no real month or day: no DateValue, and no blocking key.
    assert parse_dob("1980-00-00", "day") is None
    assert parse_dob("1980-13-07", "day") is None
    assert parse_dob("1980-01-32", "day") is None


@pytest.mark.parametrize("value", [
    "١٩٨٠-٠٧-١٥",   # Arabic-Indic digits
    "۱۹۸۰-۰۷-۱۵",   # Extended Arabic-Indic (Persian) digits
    "१९८०-०७-१५",   # Devanagari digits
    "１９８０-０７-１５",  # full-width digits, as an East Asian IME types them
])
def test_a_dob_in_another_scripts_digits_is_read(value):
    # A DOB is a number whatever script its digits are in. Rejecting it throws away the
    # strongest field the pair has (EXACT +6) on a chart the name pass still pairs, so a real
    # duplicate falls below review with nothing to say why (ADR-0014: no cultural capture).
    assert parse_dob(value, "day") == DateValue(year=1980, month=7, day=15)


@pytest.mark.parametrize("value", [
    "\u00a01980-07-15",  # no-break space: not the ASCII whitespace the SQL key strips
    "1980-07-15\u3000",  # ideographic space
    "1980-07-15\x1c",    # HL7 MLLP's end-of-block byte, which str.strip() would remove
    "1980-07-15²",       # a superscript digit: str.isdigit() accepts it, but it is no decimal
])
def test_only_ascii_whitespace_is_stripped_and_only_decimal_digits_read(value):
    # Exactly the set blocking_sql's name+monthday regex strips, in every database locale (a
    # bare \s follows the locale), so a value one side reads the other side keys.
    assert parse_dob(value, "day") is None
