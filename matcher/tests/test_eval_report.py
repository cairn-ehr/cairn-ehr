"""Pure tests for the plain-text report formatter."""

from cairn_matcher.eval.dataset import load_dataset
from cairn_matcher.eval.loader import load_bundled_gold
from cairn_matcher.eval.report import format_scorer
from cairn_matcher.eval.scorer_eval import evaluate_scorer


def test_scorer_report_mentions_key_metrics_and_the_caveat():
    text = format_scorer(evaluate_scorer(load_bundled_gold()), dataset_name="gold_v1")
    assert "gold_v1" in text
    assert "auto_false_link_rate" in text
    assert "precision" in text
    # The honest caveat must be in the printed report, not just the docs.
    assert "regression" in text.lower() or "not a statistical" in text.lower()


def test_scorer_report_is_a_single_string():
    text = format_scorer(evaluate_scorer(load_bundled_gold()))
    assert isinstance(text, str)
    assert text.strip()


def test_scorer_report_shows_repaired_match_pair_count():
    ds = load_dataset({
        "entities": [{"entity_id": "p", "records": [
            {"record_id": "p-1", "names": [{"value": "Ana Silva"}],
             "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 70}},
            {"record_id": "p-2", "names": [{"value": "Ana Silva"}], "repaired": True,
             "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 70}},
        ]}],
    })
    text = format_scorer(evaluate_scorer(ds))
    assert "repaired match pairs" in text
    assert "1 of 1" in text  # 1 repaired of 1 true pair


def test_scorer_report_shows_zero_repaired_on_unmarked_data():
    text = format_scorer(evaluate_scorer(load_bundled_gold()))
    assert "repaired match pairs" in text
    assert "0 of 3" in text  # gold has 3 true pairs, none repaired
