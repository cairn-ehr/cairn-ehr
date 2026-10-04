"""Measure the commit-time duplicate check (repair path R4): per-chart latency vs one sweep.

Why this exists: the R4 worker has two modes (per-chart check, or one bulk sweep), and two
constants decide between them -- ``Settings.bulk_threshold`` (how many waiting charts make a
sweep cheaper than checking each) and ``targeted.DEFAULT_TARGETED_CAP`` (the biggest block the
per-chart check will pair up). Both must come from a measurement, not a guess.

Everything is seeded and measured inside ONE transaction that is rolled back, so the database
is left as found. The seed is ANALYZEd inside that transaction so the planner sees the real row
counts (a freshly-seeded, never-analyzed table gets a misleadingly bad or good plan).

Usage:
    CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test" \
        uv run --extra pipeline python -m cairn_matcher.eval.measure_check \
        [--sizes 2000 10000 50000] [--sample 50] [--cap 1000]
"""

import argparse
import os
import random
import statistics
import time

import psycopg

from cairn_matcher.eval.blocking_eval import record_uuid, seed_dataset
from cairn_matcher.eval.dataset import load_dataset
from cairn_matcher.eval.generator import GenSpec, generate_dataset
from cairn_matcher.pipeline.db import generate_candidate_pairs
from cairn_matcher.pipeline.runner import assess
from cairn_matcher.pipeline.targeted import candidate_pairs_for

_SEEDED_TABLES = ("patient_demographic", "patient_name", "patient_identifier")


def _per_chart(conn, charts, cap):
    """Seconds for each chart: its candidate pairs plus an assess of each (no persist)."""
    times = []
    for me in charts:
        t0 = time.perf_counter()
        pairs, _ = candidate_pairs_for(conn, me, max_block_size=cap)
        for lo, hi in pairs:
            assess(conn, lo, hi)
        times.append(time.perf_counter() - t0)
    return times


def main(argv=None) -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--sizes", type=int, nargs="+", default=[2000, 10000, 50000])
    p.add_argument("--sample", type=int, default=50)
    p.add_argument("--cap", type=int, default=1000)
    args = p.parse_args(argv)
    with psycopg.connect(os.environ["CAIRN_TEST_PG"]) as conn:
        for n in args.sizes:
            t_start = time.perf_counter()
            ds = load_dataset(generate_dataset(GenSpec(seed=11, n_entities=n // 2)))
            seed_dataset(conn, ds)
            for table in _SEEDED_TABLES:
                conn.execute(f"ANALYZE {table}")
            ids = [record_uuid(r.record_id) for r in ds.all_records()]
            sample = random.Random(11).sample(ids, min(args.sample, len(ids)))
            per = _per_chart(conn, sample, args.cap)
            t0 = time.perf_counter()
            pairs, _ = generate_candidate_pairs(conn, max_block_size=100)
            for lo, hi in pairs:
                assess(conn, lo, hi)
            sweep_s = time.perf_counter() - t0
            q = statistics.quantiles(per, n=20)
            print(f"N={n:>6}  per-chart p50={statistics.median(per) * 1000:7.0f} ms  "
                  f"p95={q[18] * 1000:7.0f} ms  sweep={sweep_s:7.1f} s ({len(pairs)} pairs)  "
                  f"break-even~{sweep_s / statistics.median(per):6.0f} charts  "
                  f"(total {time.perf_counter() - t_start:.0f} s)", flush=True)
            conn.rollback()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
