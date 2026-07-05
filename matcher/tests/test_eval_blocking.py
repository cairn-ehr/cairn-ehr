"""DB-gated tests for the blocking eval (pair-completeness / reduction-ratio).

Gated on CAIRN_TEST_PG via the shared pg_conn fixture (skipped cleanly without a DB).
"""

from cairn_matcher.eval.blocking_eval import evaluate_blocking, seed_dataset
from cairn_matcher.eval.dataset import canonical_label_pair, load_dataset
from cairn_matcher.eval.loader import load_bundled_gold
from tests.conftest import seed_patient

RESIDENT = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee"


def test_gold_blocking_recall_is_total(pg_conn):
    # Every true-match pair in gold_v1 shares an identifier or a name token AND a DOB,
    # so blocking must generate all of them: pair_completeness == 1.0, no dropped matches.
    m = evaluate_blocking(pg_conn, load_bundled_gold())
    assert m.pair_completeness == 1.0
    assert m.dropped_true_matches == ()
    assert m.reduction_ratio > 0.0  # blocking generated fewer than all possible pairs


def test_oversized_block_is_skipped_and_estimated(pg_conn):
    # Three records sharing one DOB; cap=2 -> that block (size 3) is skipped, dropping
    # C(3,2)=3 candidate pairs, reported via dropped_pair_estimate.
    ds = load_dataset({"name": "big", "entities": [
        {"entity_id": "e", "records": [
            {"record_id": f"r{i}",
             "dob": {"value": "2000-01-01", "precision": "day", "provenance_rank": 40}}
            for i in range(3)
        ]},
    ]})
    m = evaluate_blocking(pg_conn, ds, max_block_size=2)
    assert any(pn == "dob" and sz == 3 for pn, _key, sz in m.skipped_blocks)
    assert m.dropped_pair_estimate == 3


def test_resident_range_chart_does_not_crash_the_eval(pg_conn):
    # The eval's reverse map knows only the SEEDED uuids, but generate_candidate_pairs
    # scans the WHOLE connected DB -- and a resident year-range chart (a real John Doe,
    # exactly what the slice-B front door writes) anchors a birth-window block that
    # catches essentially every seeded record born inside it. Those resident<->seeded
    # pairs are outside the labelled ground truth: they must be EXCLUDED from the
    # metrics, not crash the run with a KeyError on the resident uuid.
    seed_patient(pg_conn, RESIDENT, dob=("1900/2100", 30, "year-range"))
    m = evaluate_blocking(pg_conn, load_bundled_gold())
    # The labelled-dataset metrics are unchanged by the out-of-scope resident chart.
    assert m.pair_completeness == 1.0
    assert m.dropped_true_matches == ()


def test_skipped_anchored_block_is_estimated_as_anchor_pairs(pg_conn):
    # A resident range chart whose window catches all 3 seeded records: block size 4
    # (anchor + 3) > cap=2 -> skipped. An anchored skip drops s-1=3 pairs (anchor x
    # member only), NOT C(4,2)=6 -- together with the symmetric dob block's C(3,2)=3
    # the estimate must be 6, not 9. (C(s,2) on anchored skips overstates quadratically:
    # a size-500 hub block would report 124,750 phantom drops instead of 499.)
    ds = load_dataset({"name": "big", "entities": [
        {"entity_id": "e", "records": [
            {"record_id": f"r{i}",
             "dob": {"value": "2000-01-01", "precision": "day", "provenance_rank": 40}}
            for i in range(3)
        ]},
    ]})
    seed_patient(pg_conn, RESIDENT, dob=("1990/2010", 30, "year-range"))
    m = evaluate_blocking(pg_conn, ds, max_block_size=2)
    assert any(pn == "dob-range" and sz == 4 for pn, _key, sz in m.skipped_blocks)
    assert m.dropped_pair_estimate == 6


def test_blocking_eval_is_idempotent_and_leaves_no_rows(pg_conn):
    # Seeding must be ephemeral: the eval rolls back its own seed, so a second run on the
    # same connection (deterministic uuid5 labels) must not hit the patient_demographic
    # PK (patient_id, field), and no synthetic rows may persist afterwards.
    gold = load_bundled_gold()
    first = evaluate_blocking(pg_conn, gold)
    second = evaluate_blocking(pg_conn, gold)  # would raise UniqueViolation if seed committed
    assert first.pair_completeness == second.pair_completeness == 1.0
    with pg_conn.cursor() as cur:
        cur.execute("SELECT count(*) FROM patient_demographic")
        assert cur.fetchone()[0] == 0


def test_seeded_admin_sex_feeds_the_range_sex_rescue(pg_conn):
    """A year-range Doe with only administrative-sex must group with a point-DOB
    resident sharing that sex under the dob-range+sex pass ALONE — proving
    seed_dataset's administrative-sex rows are visible to the blocking_sex CTE."""
    from cairn_matcher.pipeline.db import generate_candidate_pairs

    ds = load_dataset({
        "entities": [
            {"entity_id": "doe", "records": [
                {"record_id": "doe-1",
                 "dob": {"value": "1980/1990", "precision": "year-range",
                         "provenance_rank": 30},
                 "administrative_sex": {"value": "male", "provenance_rank": 30}},
            ]},
            {"entity_id": "resident", "records": [
                {"record_id": "resident-1",
                 "dob": {"value": "1985-05-12", "precision": "day",
                         "provenance_rank": 40},
                 "sex_at_birth": {"value": "male", "provenance_rank": 40},
                 "names": [{"value": "Alex Nguyen", "provenance_rank": 30}]},
            ]},
        ],
    })
    reverse = seed_dataset(pg_conn, ds)
    # Large cap: on a shared cairn_test DB, leaked resident charts (issue #84) can
    # sit inside doe-1's window and balloon the anchored block past the default cap,
    # which would skip the block and flake this test.
    pairs, _skipped = generate_candidate_pairs(
        pg_conn, max_block_size=10_000, enabled_passes={"dob-range+sex"}
    )
    pg_conn.rollback()
    labels = {
        canonical_label_pair(reverse[lo], reverse[hi])
        for lo, hi in pairs
        if lo in reverse and hi in reverse
    }
    assert canonical_label_pair("doe-1", "resident-1") in labels
