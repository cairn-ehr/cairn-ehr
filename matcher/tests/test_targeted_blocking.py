"""R4 Task 3: one chart's candidate pairs equal the full sweep's pairs that include it.

The drift canary is the load-bearing test: targeted.py composes the SAME CTE constants as the
sweep and only filters its groups, so this proves the filter, for every chart of a generated
population, with no cap on either side.

`pipeline.db` (psycopg) is imported inside the one test that needs it, so CI's pure suite (no
`pipeline` extra) still collects this module and runs the pure `pairs_with` tests.
"""

import uuid

from cairn_matcher.eval.blocking_eval import record_uuid, seed_dataset
from cairn_matcher.eval.dataset import load_dataset
from cairn_matcher.eval.generator import GenSpec, generate_dataset
from cairn_matcher.pipeline.targeted import candidate_pairs_for, pairs_with
from tests.conftest import seed_patient

UNCAPPED = 10**9


def test_pairs_with_pairs_me_with_each_other_member_canonically():
    me = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
    a = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
    c = "CCCCCCCC-CCCC-CCCC-CCCC-CCCCCCCCCCCC"
    assert pairs_with(me, [a, me, c]) == {
        (a, me), (me, c.lower()),
    }


def test_pairs_with_never_pairs_a_differently_spelled_chart_with_itself():
    me = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
    a = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
    for spelled in ("{" + me + "}", me.upper(), uuid.UUID(me).hex):
        assert pairs_with(spelled, [a, me]) == {(a, me)}, spelled


def test_targeted_pairs_equal_the_sweeps_pairs_for_every_chart(pg_conn):
    from cairn_matcher.pipeline.db import generate_candidate_pairs

    # Range-heavy AND name-heavy, so all eight passes and both range shapes (the chart as anchor,
    # the chart as a member of someone else's window) are exercised.
    ds = load_dataset(generate_dataset(GenSpec(seed=7, n_entities=80, p_dob_estimate=0.4)))
    seed_dataset(pg_conn, ds)          # no commit: the rows live in this transaction
    sweep_pairs, sweep_skipped = generate_candidate_pairs(pg_conn, max_block_size=UNCAPPED)
    assert sweep_skipped == []
    checked = 0
    for rec in ds.all_records():
        me = record_uuid(rec.record_id)
        mine, skipped = candidate_pairs_for(pg_conn, me, max_block_size=UNCAPPED)
        assert skipped == []
        assert set(mine) == {p for p in sweep_pairs if me in p}, rec.record_id
        checked += 1
    assert checked >= 160
    pg_conn.rollback()


def test_an_oversized_block_is_reported_never_silently_dropped(pg_conn):
    ids = [str(uuid.uuid4()) for _ in range(5)]
    for p in ids:
        seed_patient(pg_conn, p, names=[("Commonname Person", 20)])
    pairs, skipped = candidate_pairs_for(pg_conn, ids[0], max_block_size=4)
    assert pairs == []
    assert ("name", "commonname", 5) in skipped


# --- #725: the per-chart range statement is ANCHORED on the chart ---------------------------
# It computes windows only for the range anchors whose window overlaps the chart's. These pin
# what that must not change.

RANGE = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"     # a range-dob chart: the anchor
POINTS = [f"{c * 8}-{c * 4}-{c * 4}-{c * 4}-{c * 12}" for c in "bcd"]


def test_another_anchors_oversized_window_is_reported_at_its_full_size(pg_conn):
    # The chart checked (POINTS[0]) is a MEMBER of RANGE's window. That window holds three
    # charts, so with RANGE itself the block is 4 > cap 3: skipped, under RANGE's uuid, at size
    # 4 -- exactly what the sweep reports. An anchored statement that computed the window only
    # as far as the checked chart would see a block of 2 and pair it, silently below the cap.
    # Distinct point dobs inside the window, so no symmetric pass (exact dob) groups them.
    seed_patient(pg_conn, RANGE, dob=("1981/1991", 30, "year-range"))
    for p, dob in zip(POINTS, ("1984-01-01", "1985-06-15", "1986-02-02"), strict=True):
        seed_patient(pg_conn, p, dob=(dob, 20))
    pairs, skipped = candidate_pairs_for(pg_conn, POINTS[0], max_block_size=3)
    assert pairs == []
    assert ("dob-range", RANGE, 4) in skipped, skipped


def test_a_chart_with_no_birth_year_has_no_range_block(pg_conn):
    # No dob: no window of its own, and inside nobody's. The anchored statement's
    # relevant-anchor set is empty, and that must read "no range block", never an error.
    seed_patient(pg_conn, RANGE, dob=("1981/1991", 30, "year-range"))
    seed_patient(pg_conn, POINTS[0], names=[("Nodob Person", 20)])
    pairs, skipped = candidate_pairs_for(pg_conn, POINTS[0], max_block_size=UNCAPPED)
    assert pairs == [] and skipped == []
