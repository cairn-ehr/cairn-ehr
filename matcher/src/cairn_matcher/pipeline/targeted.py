# matcher/src/cairn_matcher/pipeline/targeted.py
"""One chart's candidate pairs (repair path R4): the sweep's blocking, kept to ONE chart.

The commit-time duplicate check (#679) asks "who might this ONE chart be a duplicate of?". It
answers with the sweep's own blocking SQL (blocking_sql._GROUPS_SQL, and the range statement in
its anchored form -- both composed from the same CTE constants as the sweep's) wrapped in a filter
that keeps only the groups containing the chart.
The SQL is SHARED, not copied, so a new blocking pass reaches this module the moment it reaches the
sweep; tests/test_targeted_blocking.py's drift canary proves the filter over a generated
population (targeted pairs == the sweep's pairs that include the chart).

Pairs are the chart x each other member only — never member x member, which is the sweep's
business. So a block's pair count grows LINEARLY here, and the cap (DEFAULT_TARGETED_CAP) can sit
far above the sweep's 100. An oversized block is still reported, never silently dropped.

Cost (#725): the symmetric statement still groups the whole population (90-180 ms at 10 000
charts on the development machine), as the sweep does once. The range statement once cost 7.5-24
s per chart, mostly a planner cross-product in its '+sex' arm; blocking_sql.py reshaped that arm
for both statements (the unanchored statement then took ~3 s), and the per-chart form is ANCHORED
(blocking_sql._ANCHORED_RANGE_GROUPS_SQL): overlap rows are built only for the range anchors
whose window overlaps the chart's, which takes it to 75-656 ms. For a large backlog the worker
runs ONE sweep instead (worker.run_bulk). A materialised key projection (#637's token table) is
the lever beyond this.

Imports without psycopg; its callers hand it an open connection.
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
# drift this module exists to avoid. From the PURE blocking_sql module, never from db.py: db.py
# imports psycopg, and this module is imported by the worker, `watch` and the CLI
# (tests/test_pure_modules_import_without_psycopg.py).
from cairn_matcher.pipeline.blocking_sql import _ANCHORED_RANGE_GROUPS_SQL, _GROUPS_SQL
from cairn_matcher.placeholder_uses import PLACEHOLDER_USES_PARAM

# A block above this is non-discriminating even when paired linearly. KEPT at 1000 through two
# measurements (Apple M3 Max 128 GB, PostgreSQL 18.1, generated population via
# eval/measure_check.py). R4 (2026-10-04): per-chart p95 9 651 ms at 10 000 records, ~7 s of it the
# blocking SQL whatever the cap (#725's diagnostic later placed it in the range statement) --
# lowering the cap could not have fixed it, so it was not tuned around the problem. After #725
# reshaped the range statement's '+sex' arm and anchored its per-chart form (2026-10-05): p50/p95
# 129/256 ms at 2 000 records and 761/1 398 ms at 10 000, inside the plan's 2 s budget WITH the cap
# at 1000. The cap now does bound real work -- a range-dob chart's estimated-age window holds ~900
# charts on average in that population (some exceed the cap), and pairing it is ~900 assessments
# (~0.9 ms each) -- but the budget holds, so there is no measured reason to drop pairs the sweep
# (cap 100) would skip. Which band of blocks between the two caps the per-chart check should pair is
# #730's decision.
DEFAULT_TARGETED_CAP = 1000

# The symmetric groups that contain the chart. The trailing %s is the chart id; the first two
# are _GROUPS_SQL's own binds (placeholder uses, value sentinels), in its order.
_TARGETED_GROUPS_SQL = (
    f"SELECT g.pass_name, g.key, g.members FROM ({_GROUPS_SQL}) g "
    "WHERE %s::uuid = ANY(g.members)"
)

# The anchored range groups the chart is in — as the anchor (its own estimated-age window) or
# as a member of another chart's window. The inner statement is the sweep's, anchored on the
# chart (#725); its two binds come first, in its order (the chart id, then the value sentinels),
# then the two of this filter.
_TARGETED_RANGE_SQL = (
    f"SELECT g.pass_name, g.anchor, g.members FROM ({_ANCHORED_RANGE_GROUPS_SQL}) g "
    "WHERE g.anchor = %s::uuid OR %s::uuid = ANY(g.members)"
)


def pairs_with(me: str, members) -> set[tuple[str, str]]:
    """Canonical pairs of `me` with every OTHER member (pure). Self-pairs are skipped.

    `me` is canonicalised here (any spelling uuid.UUID accepts): the self-pair test compares
    canonical text, so a braced or upper-case `me` would otherwise pair the chart with itself.
    """
    me = str(uuid.UUID(str(me)))
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
        cur.execute(_TARGETED_GROUPS_SQL, (PLACEHOLDER_USES_PARAM, VALUE_SENTINELS_PARAM, me))
        for pass_name, key, members in cur.fetchall():
            require_registered(pass_name, SYMMETRIC_PASSES)
            if len(members) > max_block_size:
                skipped.append((pass_name, key, len(members)))
            else:
                pairs.update(pairs_with(me, members))
        cur.execute(_TARGETED_RANGE_SQL, (me, VALUE_SENTINELS_PARAM, me, me))
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
