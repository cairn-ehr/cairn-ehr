"""Pure tests for the range-aware half of the generator's blocking-key mirror.

_birth_window / the shares_blocking_key range branch mirror _RANGE_GROUPS_SQL's
birth_window CTE + anchored window_overlap join (pipeline/db.py). The danger
direction is OVER-claiming: a pair the mirror calls recoverable but the SQL never
generates would make _repair stand down and silently break the volume set's
recoverable-by-construction guarantee. These tests pin the safe semantics.
"""

from cairn_matcher.eval.generator import _birth_window, _repair, shares_blocking_key


def _rec(dob=None, names=(), identifiers=()):
    out = {"record_id": "r"}
    if dob is not None:
        out["dob"] = dict(dob)
    if names:
        out["names"] = [{"value": n} for n in names]
    if identifiers:
        out["identifiers"] = [
            {"system": s, "match_key": k, "value": k} for s, k in identifiers
        ]
    return out


# --- _birth_window: the birth_window CTE mirror -------------------------------

def test_window_parses_a_wellformed_year_range():
    assert _birth_window(_rec(dob={"value": "1980/1990", "precision": "year-range"})) \
        == (1980, 1990, True)


def test_window_excludes_malformed_and_inverted_ranges():
    # Mirrors the SQL regex + min<=max guards: excluded, never a guessed window.
    for bad in ("about-forty", "1980/199", "1990/1980", "1980-1990", "1980/1990/2000"):
        assert _birth_window(_rec(dob={"value": bad, "precision": "year-range"})) is None


def test_window_point_dob_uses_first_four_digit_run():
    # substring(value FROM '[0-9]{4}') takes the FIRST 4-digit run — "12/05/1990"
    # has runs 12, 05, 1990; the first 4-digit one is 1990.
    assert _birth_window(_rec(dob={"value": "1985-05-12", "precision": "day"})) \
        == (1985, 1985, False)
    assert _birth_window(_rec(dob={"value": "12/05/1990", "precision": "day"})) \
        == (1990, 1990, False)


def test_window_absent_without_a_four_digit_run_or_dob():
    assert _birth_window(_rec(dob={"value": "12/05/90", "precision": "day"})) is None
    assert _birth_window(_rec()) is None


# --- shares_blocking_key: the anchored range branch ----------------------------

def test_range_overlapping_point_shares_a_key():
    a = _rec(dob={"value": "1980/1990", "precision": "year-range"}, names=("Alex",))
    b = _rec(dob={"value": "1985-05-12", "precision": "day"}, names=("Zed",))
    assert shares_blocking_key(a, b)


def test_range_disjoint_point_shares_nothing():
    a = _rec(dob={"value": "1980/1990", "precision": "year-range"}, names=("Alex",))
    b = _rec(dob={"value": "1995-05-12", "precision": "day"}, names=("Zed",))
    assert not shares_blocking_key(a, b)


def test_two_overlapping_ranges_share_a_key():
    # Two John Does, two sites — the only key that pair can ever share (db.py comment).
    a = _rec(dob={"value": "1980/1990", "precision": "year-range"}, names=("Alex",))
    b = _rec(dob={"value": "1988/1995", "precision": "year-range"}, names=("Zed",))
    assert shares_blocking_key(a, b)


def test_point_point_same_year_is_not_a_range_key():
    # window_overlap requires a.is_range: two point DOBs in the same year never key
    # via the range passes (they need exact-DOB/name/identifier instead).
    a = _rec(dob={"value": "1985-01-01", "precision": "day"}, names=("Alex",))
    b = _rec(dob={"value": "1985-12-28", "precision": "day"}, names=("Zed",))
    assert not shares_blocking_key(a, b)


def test_identical_yearrange_values_do_not_fake_an_exact_dob_key():
    # THE over-claim fix: _GROUPS_SQL's exact-'dob' arm excludes year-range rows
    # (IS DISTINCT FROM 'year-range'). Two identical but MALFORMED range values parse
    # to no window, so neither the range branch nor the exact branch may claim them.
    a = _rec(dob={"value": "199x/1999", "precision": "year-range"}, names=("Alex",))
    b = _rec(dob={"value": "199x/1999", "precision": "year-range"}, names=("Zed",))
    assert not shares_blocking_key(a, b)


def test_repair_stands_down_when_the_window_carries_the_pair():
    # With the range branch mirrored, _repair must NOT append the seed's name to an
    # estimated-age clone the dob-range pass already recovers.
    seed = _rec(dob={"value": "1985-05-12", "precision": "day"}, names=("Alex Nguyen",))
    clone = _rec(dob={"value": "1983/1988", "precision": "year-range"}, names=("Zed Q",))
    repaired = _repair(seed, clone)
    assert repaired is clone  # _repair returns the clone UNTOUCHED when a key exists
    assert [n["value"] for n in repaired.get("names", [])] == ["Zed Q"]
