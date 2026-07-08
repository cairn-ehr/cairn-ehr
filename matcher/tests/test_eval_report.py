"""Pure tests for the plain-text report formatter."""

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
