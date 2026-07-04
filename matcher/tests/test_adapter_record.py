"""build_* and candidate_from_rows shape projection rows into a CandidateRecord.

These are pure: they take plain dict rows (as pipeline.db will hand them) and return
B1 value types. They never read the event body and never touch a database.
"""

import pytest

from cairn_matcher.pipeline.adapter import (
    build_identifiers,
    build_names,
    candidate_from_rows,
    single_field,
)
from cairn_matcher.records import CandidateRecord, DateValue, FieldValue, MatcherTypeError, Name


def test_build_names_makes_untagged_bag_and_takes_max_provenance():
    rows = [
        {"value": "Alex Smith", "provenance_rank": 20},
        {"value": "Smith Alex", "provenance_rank": 60},  # order-tolerant -> same bag
    ]
    fv = build_names(rows)
    assert fv is not None
    assert fv.provenance_rank == 60  # weaker-side logic is the orchestrator's; here: max evidence
    assert fv.value == frozenset({Name(tokens={"unspecified": ("alex", "smith")})})


def test_build_names_lowercases_and_splits_on_whitespace():
    fv = build_names([{"value": "  John   Doe ", "provenance_rank": 10}])
    # tokens are sorted for canonical representation (required by order-tolerant bag semantics)
    assert fv.value == frozenset({Name(tokens={"unspecified": ("doe", "john")})})


def test_build_names_empty_is_none():
    assert build_names([]) is None


def test_build_identifiers_groups_by_system_and_skips_unknown():
    rows = [
        {"system": "mrn:hospital-a", "match_key": "12345"},
        {"system": "mrn:hospital-a", "match_key": "67890"},
        {"system": "unknown", "match_key": "ignore-me"},
    ]
    assert build_identifiers(rows) == {"mrn:hospital-a": frozenset({"12345", "67890"})}


def test_build_identifiers_empty_is_empty_mapping():
    assert build_identifiers([]) == {}


def test_single_field_maps_value_and_rank():
    assert single_field({"value": "female", "provenance_rank": 60}) == FieldValue("female", 60)


def test_single_field_none_row_is_none():
    assert single_field(None) is None


def test_single_field_unknown_sentinel_is_absence():
    # `unknown` is a legitimate recorded value (principle 4) but ZERO matching evidence:
    # it must not fabricate EXACT (unknown vs unknown) or DISAGREE (unknown vs male).
    assert single_field({"value": "unknown", "provenance_rank": 60}) is None
    assert single_field({"value": "Unknown", "provenance_rank": 0}) is None


def test_build_names_normalizes_unicode_nfc_vs_nfd():
    # The SAME name arriving precomposed (NFC) vs decomposed (NFD) must fold to identical
    # tokens, else the two grade DISAGREE and never block together. "Jón":
    nfc = "Jón"           # ó as one code point (U+00F3)
    nfd = "Jón"          # o + combining acute (U+006F U+0301)
    a = build_names([{"value": nfc, "provenance_rank": 60}])
    b = build_names([{"value": nfd, "provenance_rank": 60}])
    assert a == b


def test_candidate_from_rows_assembles_all_fields():
    rec = candidate_from_rows(
        dob_row={"value": "1980-07-15", "facets": {"precision": "day"}, "provenance_rank": 60},
        sex_row={"value": "female", "provenance_rank": 60},
        name_rows=[{"value": "Alex Smith", "provenance_rank": 20}],
        identifier_rows=[{"system": "mrn:a", "match_key": "1"}],
    )
    assert rec.dob == FieldValue(DateValue(1980, 7, 15), 60)
    assert rec.sex_at_birth == FieldValue("female", 60)
    assert rec.names == build_names([{"value": "Alex Smith", "provenance_rank": 20}])
    assert rec.identifiers == {"mrn:a": frozenset({"1"})}


def test_candidate_from_rows_all_absent_is_empty_record():
    rec = candidate_from_rows(dob_row=None, sex_row=None, name_rows=[], identifier_rows=[])
    assert rec == CandidateRecord()


def test_candidate_from_rows_unparseable_dob_drops_to_none():
    rec = candidate_from_rows(
        dob_row={"value": "15/07/1980", "facets": {"precision": "day"}, "provenance_rank": 60},
        sex_row=None, name_rows=[], identifier_rows=[],
    )
    assert rec.dob is None  # safe degrade, not a wrong DateValue


def test_candidate_from_rows_wrong_type_raises():
    # A name row whose value is not a string is an adapter/upstream bug -> raise loudly.
    with pytest.raises(MatcherTypeError):
        build_names([{"value": 12345, "provenance_rank": 10}])


def test_value_sentinels_hold_the_lowercase_ascii_invariant():
    # pipeline/db.py binds this set into SQL where btrim(lower(...)) is the normalizer,
    # while the adapter compares with strip().casefold() -- the two only agree on
    # lowercase ASCII members, so a non-conforming addition would silently split the
    # SQL and Python views of "absent". Pin the documented invariant mechanically.
    from cairn_matcher.pipeline.adapter import VALUE_SENTINELS

    for sentinel in VALUE_SENTINELS:
        assert sentinel == sentinel.strip().casefold()
        assert sentinel.isascii()
