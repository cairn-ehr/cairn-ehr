"""Tests for the synthetic blocking-eval dataset generator (pure, stdlib-only)."""

import copy
import random

from cairn_matcher.eval.generator import (
    name_tokens, shares_blocking_key,
    corrupt_dob_format, corrupt_dob_typo, corrupt_name, corrupt_identifier,
)


def test_name_tokens_lowercases_and_splits_all_names():
    rec = {"names": [{"value": "Alex Nguyen"}, {"value": "NGUYEN Van Alex"}]}
    assert name_tokens(rec) == {"alex", "nguyen", "van"}


def test_name_tokens_empty_when_no_names():
    assert name_tokens({"record_id": "r"}) == set()


def test_shares_key_via_exact_dob():
    a = {"dob": {"value": "1990-05-12"}, "names": [{"value": "Ann"}]}
    b = {"dob": {"value": "1990-05-12"}, "names": [{"value": "Bob"}]}
    assert shares_blocking_key(a, b) is True


def test_shares_key_via_name_token():
    a = {"dob": {"value": "1990-05-12"}, "names": [{"value": "Alex Nguyen"}]}
    b = {"dob": {"value": "1985-01-01"}, "names": [{"value": "Sam Nguyen"}]}
    assert shares_blocking_key(a, b) is True


def test_shares_key_via_identifier_but_not_unknown():
    a = {"identifiers": [{"system": "au-medicare", "match_key": "111"}]}
    b = {"identifiers": [{"system": "au-medicare", "match_key": "111"}]}
    assert shares_blocking_key(a, b) is True
    a_unk = {"identifiers": [{"system": "unknown", "match_key": "111"}]}
    b_unk = {"identifiers": [{"system": "unknown", "match_key": "111"}]}
    assert shares_blocking_key(a_unk, b_unk) is False


def test_no_shared_key_is_false():
    a = {"dob": {"value": "1990-05-12"}, "names": [{"value": "Alex Nguyen"}],
         "identifiers": [{"system": "au-medicare", "match_key": "111"}]}
    b = {"dob": {"value": "12/05/1990"}, "names": [{"value": "Sam Smith"}],
         "identifiers": [{"system": "au-medicare", "match_key": "222"}]}
    assert shares_blocking_key(a, b) is False


def _seed_rec():
    return {
        "record_id": "e0-seed",
        "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 40},
        "names": [{"value": "Alex Nguyen", "provenance_rank": 30}],
        "identifiers": [{"system": "au-medicare", "match_key": "12345", "value": "12345"}],
    }


def test_dob_format_keeps_birth_year_changes_value():
    rec = _seed_rec()
    before = copy.deepcopy(rec)
    out = corrupt_dob_format(rec, random.Random(1))
    assert rec == before                       # input unmutated (pure)
    assert out["dob"]["value"] != "1990-05-12" # exact value changed
    assert "1990" in out["dob"]["value"]       # birth-year preserved


def test_dob_typo_changes_value():
    out = corrupt_dob_typo(_seed_rec(), random.Random(2))
    assert out["dob"]["value"] != "1990-05-12"


def test_name_corruption_changes_a_name_value():
    out = corrupt_name(_seed_rec(), random.Random(3))
    assert [n["value"] for n in out["names"]] != ["Alex Nguyen"]


def test_identifier_corruption_drops_or_mistypes():
    out = corrupt_identifier(_seed_rec(), random.Random(4))
    ids = out["identifiers"]
    # either dropped (fewer) or the match_key changed
    assert ids == [] or ids[0]["match_key"] != "12345"


def test_operators_are_noops_when_field_absent():
    bare = {"record_id": "x", "names": [{"value": "Sam"}]}
    r = random.Random(5)
    assert corrupt_dob_format(bare, r) == bare
    assert corrupt_dob_typo(bare, r) == bare
    assert corrupt_identifier(bare, r) == bare
