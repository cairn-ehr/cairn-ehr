"""Tests for k-fold held-out weight-learning measurement (eval/crossval.py)."""

import pytest

from cairn_matcher.eval.crossval import (
    LiftReport,
    format_lift,
    kfold_lift,
    split_clusters,
)
from cairn_matcher.eval.dataset import DatasetRecord, EntityCluster, LabelledDataset
from cairn_matcher.eval.loader import load_bundled_gold


def _cluster(entity_id, *record_ids):
    return EntityCluster(
        entity_id=entity_id,
        records=tuple(DatasetRecord(record_id=r) for r in record_ids),
    )


def _synthetic(n_clusters):
    # Each cluster has 2 records (so every cluster yields a match pair, robust to any split).
    ents = tuple(_cluster(f"e{i}", f"e{i}-a", f"e{i}-b") for i in range(n_clusters))
    return LabelledDataset(name="synthetic", entities=ents)


def test_split_never_straddles_a_cluster_and_covers_every_cluster_once():
    ds = _synthetic(7)
    parts = split_clusters(ds, folds=3)
    all_ids = [e.entity_id for p in parts for e in p.entities]
    assert sorted(all_ids) == [f"e{i}" for i in range(7)]  # each cluster exactly once
    assert len(all_ids) == len(set(all_ids))


def test_split_is_deterministic_across_calls():
    ds = _synthetic(7)
    a = [[e.entity_id for e in p.entities] for p in split_clusters(ds, 3)]
    b = [[e.entity_id for e in p.entities] for p in split_clusters(ds, 3)]
    assert a == b


def test_split_rejects_too_few_clusters_or_folds():
    with pytest.raises(ValueError):
        split_clusters(_synthetic(2), folds=3)
    with pytest.raises(ValueError):
        split_clusters(_synthetic(7), folds=1)


def test_kfold_lift_on_gold_reports_before_and_after():
    ds = load_bundled_gold()
    report = kfold_lift(ds, folds=5)
    assert isinstance(report, LiftReport)
    assert report.folds == 5
    # held-out pooling covers only within-fold pairs; both before/after on the SAME set
    assert report.before.pair_count == report.after.pair_count
    assert report.before.pair_count > 0


def test_kfold_lift_skips_a_fold_whose_training_has_no_match_pairs():
    # 3-fold on gold puts all 3 match clusters in one fold -> training the other folds has
    # zero match pairs; that fold is skipped (not a crash).
    report = kfold_lift(load_bundled_gold(), folds=3)
    assert report.skipped_folds >= 1
    assert report.before.pair_count == report.after.pair_count


def test_format_lift_shows_both_blocks():
    text = format_lift(kfold_lift(_synthetic(6), folds=3), dataset_name="synthetic")
    assert "BEFORE" in text and "AFTER" in text
