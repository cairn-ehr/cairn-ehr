"""Pure tests for the range-aware half of the generator's blocking-key mirror.

_birth_window / the shares_blocking_key range branch mirror _RANGE_GROUPS_SQL's
birth_window CTE + anchored window_overlap join (pipeline/db.py). The danger
direction is OVER-claiming: a pair the mirror calls recoverable but the SQL never
generates would make _repair stand down and silently break the volume set's
recoverable-by-construction guarantee. These tests pin the safe semantics.
"""

import random

from cairn_matcher.eval.dataset import load_dataset
from cairn_matcher.eval.generator import (
    GenSpec,
    _birth_window,
    _repair,
    corrupt_dob_estimate,
    generate_dataset,
    shares_blocking_key,
)


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
    # "1980/1990\n" pins the fullmatch fix: Python re '$' matches before a trailing
    # '\n' but POSIX ARE '$' (the SQL side) does not, so a trailing newline must
    # still exclude the row here, never yield a guessed window (over-claim).
    for bad in ("about-forty", "1980/199", "1990/1980", "1980-1990", "1980/1990/2000",
                "1980/1990\n"):
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


# --- corrupt_dob_estimate: the estimated-age operator ---------------------------


def test_estimate_replaces_dob_with_a_window_containing_the_year():
    rec = _rec(dob={"value": "1985-05-12", "precision": "day"})
    out = corrupt_dob_estimate(rec, random.Random(0))
    assert out["dob"]["precision"] == "year-range"
    lo, hi = (int(p) for p in out["dob"]["value"].split("/"))
    assert lo <= 1985 <= hi
    assert out["dob"]["provenance_rank"] == 30  # clinician-observed (slice B)


def test_estimate_moves_sex_to_the_observed_facet():
    rec = _rec(dob={"value": "1985-05-12", "precision": "day"})
    rec["sex_at_birth"] = {"value": "female", "provenance_rank": 40}
    out = corrupt_dob_estimate(rec, random.Random(0))
    assert "sex_at_birth" not in out
    assert out["administrative_sex"] == {"value": "female", "provenance_rank": 30}


def test_estimate_draws_a_sex_when_the_seed_has_none():
    rec = _rec(dob={"value": "1985-05-12", "precision": "day"})
    out = corrupt_dob_estimate(rec, random.Random(0))
    assert out["administrative_sex"]["value"] in ("male", "female")


def test_estimate_is_a_noop_without_a_four_digit_run():
    rec = _rec(dob={"value": "12/05/90", "precision": "day"})
    assert corrupt_dob_estimate(rec, random.Random(0))["dob"] == rec["dob"]
    no_dob = _rec(names=("Alex",))
    assert "dob" not in corrupt_dob_estimate(no_dob, random.Random(0))


def test_estimate_is_a_noop_when_the_window_would_leave_yyyy_yyyy():
    # A first-run year too close to the 4-digit boundaries cannot be widened into the
    # '<yyyy>/<yyyy>' shape the SQL and _birth_window accept ("0001"-tol goes negative,
    # "9999"+tol grows a fifth digit). Emitting "-001/0006" would be a malformed range
    # both sides reject — so the operator must no-op, like any other unusable year
    # (the documented safe degrade), never emit an unparseable window.
    for boundary in ("0001-01-01", "9999-01-01"):
        rec = _rec(dob={"value": boundary, "precision": "day"})
        assert corrupt_dob_estimate(rec, random.Random(0))["dob"] == rec["dob"]


def test_estimate_never_mutates_its_input():
    rec = _rec(dob={"value": "1985-05-12", "precision": "day"})
    rec["sex_at_birth"] = {"value": "male", "provenance_rank": 40}
    frozen = {"dob": dict(rec["dob"]), "sex_at_birth": dict(rec["sex_at_birth"])}
    corrupt_dob_estimate(rec, random.Random(0))
    assert rec["dob"] == frozen["dob"] and rec["sex_at_birth"] == frozen["sex_at_birth"]


def test_estimate_heavy_dataset_round_trips_and_carries_the_new_fields():
    ds_dict = generate_dataset(GenSpec(seed=3, n_entities=40, p_dob_estimate=1.0))
    ds = load_dataset(ds_dict)  # round-trips the real loader (Task 1's plumbing)
    clones = [r for e in ds.entities for r in e.records if r.record_id.endswith("-dup")]
    ranged = [r for r in clones if (r.dob or {}).get("precision") == "year-range"]
    assert ranged, "p_dob_estimate=1.0 must produce year-range clones"
    assert all(r.administrative_sex is not None for r in ranged)
    assert all(r.sex_at_birth is None for r in ranged)
