"""R4 Task 3: one chart's candidate pairs equal the full sweep's pairs that include it.

The drift canary is the load-bearing test: targeted.py composes the SAME CTE constants as the
sweep and only filters its groups, so this proves the filter, for every chart of a generated
population, with no cap on either side.
"""

import uuid

from cairn_matcher.eval.blocking_eval import record_uuid, seed_dataset
from cairn_matcher.eval.dataset import load_dataset
from cairn_matcher.eval.generator import GenSpec, generate_dataset
from cairn_matcher.pipeline.db import generate_candidate_pairs
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


def test_targeted_pairs_equal_the_sweeps_pairs_for_every_chart(pg_conn):
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
