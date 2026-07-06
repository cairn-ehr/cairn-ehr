"""Tests for safety-first threshold derivation (eval/learner.derive_thresholds)."""

import pytest

from cairn_matcher.eval.learner import derive_thresholds


def test_auto_is_above_every_nonmatch_score():
    # Zero false auto-links by construction: auto > every non-match score.
    scored = [(True, 10.0), (True, 9.0), (False, 4.0), (False, 3.0)]
    thresholds, collided = derive_thresholds(scored, recall_target=0.99, margin=0.5)
    assert thresholds.auto == 4.0 + 0.5
    assert not collided
    assert all(s < thresholds.auto for is_m, s in scored if not is_m)


def test_review_is_the_top_nonmatch_and_surfaces_matches_above_it():
    # review anchors to the best impostor; every match above it is surfaced.
    scored = [(True, float(i)) for i in range(1, 101)] + [(False, 0.0)]
    thresholds, collided = derive_thresholds(scored, recall_target=0.99, margin=0.5)
    assert thresholds.review == 0.0
    surfaced = sum(1 for is_m, s in scored if is_m and s >= thresholds.review)
    assert surfaced == 100
    assert not collided


def test_separated_classes_keep_review_below_auto_and_do_not_collide():
    # The ideal case: matches far above non-matches. review must stay < auto (band invariant),
    # collided must be False — this is the regression guard for the review>auto inversion bug.
    scored = [(True, 20.0), (True, 18.0), (False, 5.0), (False, 4.0)]
    thresholds, collided = derive_thresholds(scored, recall_target=0.99, margin=0.5)
    assert thresholds.review == 5.0
    assert thresholds.auto == 5.5
    assert thresholds.review < thresholds.auto
    assert not collided


def test_impostor_outscoring_matches_flags_a_collision():
    # A non-match out-scores every true match -> safety-first placement cannot surface them
    # without also surfacing the impostor -> collided True, but auto stays above the impostor.
    scored = [(True, 5.0), (True, 2.0), (False, 6.0)]
    thresholds, collided = derive_thresholds(scored, recall_target=0.99, margin=0.5)
    assert thresholds.review == 6.0
    assert thresholds.auto == 6.5  # auto still above the top non-match: zero false-auto
    assert thresholds.review < thresholds.auto
    assert collided  # no true match reaches review=6.0 -> recall floor unmet


def test_recall_shortfall_sets_the_collision_flag():
    # recall_target 1.0 with matches entangled below the impostor -> collided.
    scored = [(True, 1.0), (True, 2.0), (False, 10.0)]
    thresholds, collided = derive_thresholds(scored, recall_target=1.0, margin=0.5)
    assert collided
    assert thresholds.review < thresholds.auto


def test_no_match_pair_raises():
    with pytest.raises(ValueError):
        derive_thresholds([(False, 1.0), (False, 2.0)])


def test_recall_target_out_of_range_raises():
    with pytest.raises(ValueError):
        derive_thresholds([(True, 1.0)], recall_target=1.5)
