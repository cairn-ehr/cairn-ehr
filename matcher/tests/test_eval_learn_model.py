"""Tests for learn_model — estimate weights then derive the thresholds that go with them."""

from cairn_matcher.eval.learner import LearnedModel, learn_model
from cairn_matcher.eval.loader import load_bundled_gold
from cairn_matcher.eval.scorer_eval import evaluate_scorer


def test_learn_model_on_gold_returns_a_usable_model():
    ds = load_bundled_gold()
    model = learn_model(ds)
    assert isinstance(model, LearnedModel)
    # the learned weights/thresholds drive the real eval path without raising
    metrics = evaluate_scorer(ds, weights=model.weights, thresholds=model.thresholds)
    assert metrics.pair_count == 45
    # zero false auto-links on the training set (safety-first threshold, self-consistency)
    assert metrics.confusion.nonmatch_auto == 0


def test_metadata_records_the_knobs_and_counts():
    ds = load_bundled_gold()
    model = learn_model(ds, alpha=0.7, recall_target=0.95, margin=1.0)
    assert model.metadata.alpha == 0.7
    assert model.metadata.recall_target == 0.95
    assert model.metadata.margin == 1.0
    assert model.metadata.train_pairs == 45
    assert model.metadata.train_matches == 3
    assert isinstance(model.metadata.review_auto_collided, bool)


def test_learned_review_below_auto_on_gold():
    # gold's matches separate from non-matches, so the two objectives should not collide.
    model = learn_model(load_bundled_gold())
    assert model.thresholds.review < model.thresholds.auto
    assert not model.metadata.review_auto_collided
