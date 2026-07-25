"""Pure end-to-end test of evaluate_scorer over a tiny inline dataset."""

from cairn_matcher.eval.dataset import load_dataset, repaired_truth_pairs
from cairn_matcher.eval.scorer_eval import evaluate_scorer

# Two records of the SAME person sharing a strong identifier and an exact high-rank DOB
# (-> AUTO), plus a third unrelated person sharing nothing (-> the non-match pairs).
_DS = load_dataset({
    "name": "driver",
    "entities": [
        {"entity_id": "p", "records": [
            {"record_id": "p-1",
             "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 70},
             "identifiers": [{"system": "mrn", "match_key": "K1", "value": "K1"}]},
            {"record_id": "p-2",
             "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 70},
             "identifiers": [{"system": "mrn", "match_key": "K1", "value": "K1"}]},
        ]},
        {"entity_id": "q", "records": [
            {"record_id": "q-1",
             "dob": {"value": "1970-01-01", "precision": "day", "provenance_rank": 70},
             "identifiers": [{"system": "mrn", "match_key": "K9", "value": "K9"}]},
        ]},
    ],
})


def test_evaluate_scorer_counts_all_pairs_and_finds_the_match():
    m = evaluate_scorer(_DS)
    assert m.pair_count == 3  # C(3,2): one true match (p-1,p-2) + two non-matches
    # The strong same-person pair is auto-banded; no non-match reaches auto.
    assert m.confusion.match_auto == 1
    assert m.auto_false_link_rate == 0.0


def test_evaluate_scorer_respects_a_custom_threshold():
    # With an absurdly high auto threshold nothing is auto-banded.
    from cairn_matcher.pipeline.banding import Thresholds
    m = evaluate_scorer(_DS, thresholds=Thresholds(review=3.0, auto=999.0))
    assert m.confusion.match_auto == 0


# One repaired clone (p-2) eases the p-1/p-2 true pair; q has no clone -> its pairs are all
# non-matches, and none of them counts even though q-1 is unrepaired anyway.
_REPAIRED_DS = load_dataset({
    "name": "repaired-driver",
    "entities": [
        {"entity_id": "p", "records": [
            {"record_id": "p-1",
             "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 70},
             "names": [{"value": "Ana Silva"}]},
            {"record_id": "p-2",
             "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 70},
             "names": [{"value": "Ana Silva"}], "repaired": True},
        ]},
        {"entity_id": "q", "records": [
            {"record_id": "q-1",
             "dob": {"value": "1970-01-01", "precision": "day", "provenance_rank": 70},
             "names": [{"value": "Wei Nguyen"}]},
        ]},
    ],
})


def test_evaluate_scorer_reports_repaired_match_pairs():
    m = evaluate_scorer(_REPAIRED_DS)
    # exactly the one within-cluster true pair (p-1,p-2), whose p-2 is repaired
    assert m.repaired_match_pairs == len(repaired_truth_pairs(_REPAIRED_DS)) == 1


def test_evaluate_scorer_reports_zero_repaired_on_the_unmarked_hand_set():
    assert evaluate_scorer(_DS).repaired_match_pairs == 0
