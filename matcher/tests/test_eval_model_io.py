"""Tests for LearnedModel JSON serialization (eval/model_io.py)."""

import pytest

from cairn_matcher.eval.learner import learn_model
from cairn_matcher.eval.loader import load_bundled_gold
from cairn_matcher.eval.model_io import (
    ModelIOError,
    model_from_json,
    model_to_json,
    read_model,
    write_model,
)
from cairn_matcher.eval.scorer_eval import evaluate_scorer


def test_round_trip_reconstructs_a_model_that_scores_identically():
    ds = load_bundled_gold()
    model = learn_model(ds)
    restored = model_from_json(model_to_json(model))
    before = evaluate_scorer(ds, weights=model.weights, thresholds=model.thresholds)
    after = evaluate_scorer(ds, weights=restored.weights, thresholds=restored.thresholds)
    assert before == after
    assert restored.metadata == model.metadata


def test_file_round_trip(tmp_path):
    model = learn_model(load_bundled_gold())
    path = tmp_path / "model.json"
    write_model(model, path)
    restored = read_model(path)
    assert restored.thresholds == model.thresholds
    assert restored.metadata == model.metadata


def test_unknown_agreement_level_rejected():
    bad = {
        "weights": {"dob": {"NOT_A_LEVEL": 1.0}},
        "thresholds": {"review": 1.0, "auto": 2.0},
        "metadata": {
            "alpha": 0.5, "recall_target": 0.99, "margin": 0.5,
            "train_pairs": 1, "train_matches": 1, "review_auto_collided": False,
        },
    }
    with pytest.raises(ModelIOError):
        model_from_json(bad)


def test_missing_top_level_key_rejected():
    with pytest.raises(ModelIOError):
        model_from_json({"weights": {}})
