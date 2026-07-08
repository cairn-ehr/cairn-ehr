"""Pure tests for record->CandidateRecord mapping and ground-truth pair derivation."""

from cairn_matcher.eval.dataset import (
    all_pairs,
    canonical_label_pair,
    load_dataset,
    record_to_candidate,
    truth_pairs,
)
from cairn_matcher.pipeline.adapter import candidate_from_rows

_DS = {
    "name": "t",
    "entities": [
        {
            "entity_id": "e1",
            "records": [{"record_id": "r1"}, {"record_id": "r2"}, {"record_id": "r3"}],
        },
        {"entity_id": "e2", "records": [{"record_id": "r4"}]},
    ],
}


def test_canonical_label_pair_orders_lexically():
    assert canonical_label_pair("b", "a") == ("a", "b")
    assert canonical_label_pair("a", "b") == ("a", "b")


def test_truth_pairs_are_within_cluster_only():
    ds = load_dataset(_DS)
    # e1 has C(3,2)=3 within-cluster pairs; e2 (singleton) has none.
    assert truth_pairs(ds) == frozenset({("r1", "r2"), ("r1", "r3"), ("r2", "r3")})


def test_all_pairs_is_the_full_universe_canonical_and_unique():
    ds = load_dataset(_DS)
    pairs = all_pairs(ds)
    assert len(pairs) == 6  # C(4,2)
    assert len(set(pairs)) == 6
    for low, high in pairs:
        assert low < high


def test_record_to_candidate_matches_a_directly_built_record():
    rec = load_dataset({
        "name": "t",
        "entities": [{"entity_id": "e", "records": [{
            "record_id": "r1",
            "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 70},
            "sex_at_birth": {"value": "female", "provenance_rank": 70},
            "names": [{"value": "Alex Nguyen", "provenance_rank": 30}],
            "identifiers": [{"system": "au-medicare", "match_key": "12345", "value": "12345"}],
        }]}],
    }).entities[0].records[0]

    got = record_to_candidate(rec)
    expected = candidate_from_rows(
        dob_row={"value": "1990-05-12", "facets": {"precision": "day"}, "provenance_rank": 70},
        sex_row={"value": "female", "provenance_rank": 70},
        name_rows=[{"value": "Alex Nguyen", "provenance_rank": 30}],
        identifier_rows=[{"system": "au-medicare", "match_key": "12345"}],
    )
    assert got == expected


def test_record_to_candidate_handles_total_absence():
    rec = load_dataset({"name": "t", "entities": [
        {"entity_id": "e", "records": [{"record_id": "r1"}]}]}).entities[0].records[0]
    got = record_to_candidate(rec)
    assert got.dob is None and got.sex_at_birth is None and got.names is None
    assert got.identifiers == {}
