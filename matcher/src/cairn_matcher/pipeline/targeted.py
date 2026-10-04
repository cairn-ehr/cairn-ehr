# matcher/src/cairn_matcher/pipeline/targeted.py
"""One chart's candidate pairs (repair path R4): the sweep's blocking, kept to ONE chart.

The commit-time duplicate check (#679) asks "who might this ONE chart be a duplicate of?". It
answers with the sweep's own blocking SQL (db._GROUPS_SQL / db._RANGE_GROUPS_SQL, composed from
the same CTE constants) wrapped in a filter that keeps only the groups containing the chart. The
SQL is SHARED, not copied, so a new blocking pass reaches this module the moment it reaches the
sweep; tests/test_targeted_blocking.py's drift canary proves the filter over a generated
population (targeted pairs == the sweep's pairs that include the chart).

Pairs are the chart x each other member only — never member x member, which is the sweep's
business. So a block's pair count grows LINEARLY here, and the cap (DEFAULT_TARGETED_CAP) can sit
far above the sweep's 100. An oversized block is still reported, never silently dropped.

Cost: each call still evaluates the blocking CTEs over the whole population (one scan of the
names), as the sweep does once. That is fine for a fresh change; for a large backlog the worker
runs ONE sweep instead (worker.run_bulk). A materialised token table (#637) would make this
cheaper later.

Requires the optional `pipeline` extra (psycopg) at call time.
"""

import uuid

from cairn_matcher.pipeline.adapter import VALUE_SENTINELS_PARAM
from cairn_matcher.pipeline.blocking import (
    ANCHORED_PASSES,
    SYMMETRIC_PASSES,
    canonical_pair,
    require_registered,
)

# Shared with the sweep on purpose (see the module docstring). Private names, imported
# deliberately: they are the one definition of blocking, and duplicating them here would be the
# drift this module exists to avoid.
from cairn_matcher.pipeline.db import _GROUPS_SQL, _PLACEHOLDER_USES_PARAM, _RANGE_GROUPS_SQL

# A block above this is non-discriminating even when paired linearly. KEPT at 1000 after the
# Task 7 measurement (2026-10-04, Apple M3 Max 128 GB, PostgreSQL 18.1, generated population via
# eval/measure_check.py): per-chart p50/p95 was 453/590 ms at 2 000 records but 8 829/9 651 ms at
# 10 000, so the cost is NOT in the pairing the cap bounds -- it is the blocking SQL scanning the
# whole population per chart (a finding, see the design page's R4 as-built note). Lowering the cap
# would not bring p95 near the 2 s budget, so it was not tuned around the problem.
DEFAULT_TARGETED_CAP = 1000

# The symmetric groups that contain the chart. The trailing %s is the chart id; the first two
# are _GROUPS_SQL's own binds (placeholder uses, value sentinels), in its order.
_TARGETED_GROUPS_SQL = (
    f"SELECT g.pass_name, g.key, g.members FROM ({_GROUPS_SQL}) g "
    "WHERE %s::uuid = ANY(g.members)"
)

# The anchored range groups the chart is in — as the anchor (its own estimated-age window) or
# as a member of another chart's window. The first %s is _RANGE_GROUPS_SQL's sentinel bind.
_TARGETED_RANGE_SQL = (
    f"SELECT g.pass_name, g.anchor, g.members FROM ({_RANGE_GROUPS_SQL}) g "
    "WHERE g.anchor = %s::uuid OR %s::uuid = ANY(g.members)"
)


def pairs_with(me: str, members) -> set[tuple[str, str]]:
    """Canonical pairs of `me` with every OTHER member (pure). Self-pairs are skipped."""
    out: set[tuple[str, str]] = set()
    for m in members:
        if str(uuid.UUID(str(m))) != me:
            out.add(canonical_pair(me, m))
    return out


def candidate_pairs_for(conn, patient, *, max_block_size=DEFAULT_TARGETED_CAP):
    """Every candidate pair involving `patient`, plus the blocks skipped for size.

    Returns (pairs, skipped_blocks) in generate_candidate_pairs' shapes: sorted canonical
    lowercase-uuid pairs, and (pass_name, key, size) for each block over the cap. Block size is
    measured exactly as the sweep measures it (a symmetric group's member count; an anchored
    window's members + its anchor), so "oversized" means the same thing in both.

    Read-only; opens a read transaction the caller must close.
    """
    me = str(uuid.UUID(str(patient)))
    pairs: set[tuple[str, str]] = set()
    skipped: list[tuple[str, str, int]] = []
    with conn.cursor() as cur:
        cur.execute(_TARGETED_GROUPS_SQL, (_PLACEHOLDER_USES_PARAM, VALUE_SENTINELS_PARAM, me))
        for pass_name, key, members in cur.fetchall():
            require_registered(pass_name, SYMMETRIC_PASSES)
            if len(members) > max_block_size:
                skipped.append((pass_name, key, len(members)))
            else:
                pairs.update(pairs_with(me, members))
        cur.execute(_TARGETED_RANGE_SQL, (VALUE_SENTINELS_PARAM, me, me))
        for pass_name, anchor, members in cur.fetchall():
            require_registered(pass_name, ANCHORED_PASSES)
            size = len(members) + 1
            if size > max_block_size:
                skipped.append((pass_name, str(anchor), size))
            elif str(anchor) == me:
                pairs.update(pairs_with(me, members))      # my window: me x each member
            else:
                pairs.add(canonical_pair(anchor, me))      # their window holds me: one pair
    return sorted(pairs), skipped
