# matcher/tests/test_blocking_passes.py
"""Pure tests for the blocking-pass registry and the anchored pair helper.

No DB, no psycopg: pipeline/blocking.py is deliberately pure (the placeholder_uses
precedent) so the A/B toggle and anchored-pair semantics are testable in the pure suite.
"""

import pytest

from cairn_matcher.pipeline.blocking import (
    ALL_PASSES,
    ANCHORED_PASSES,
    SYMMETRIC_PASSES,
    dropped_pair_estimate,
    pairs_from_anchor,
    require_registered,
    resolve_enabled_passes,
)

A = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
B = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
C = "cccccccc-cccc-cccc-cccc-cccccccccccc"


def test_all_passes_is_the_eight_known_names():
    assert ALL_PASSES == (
        "identifier", "dob", "name", "name+year",
        "dob+first-initial", "name+sex", "dob-range", "dob-range+sex",
    )


def test_new_compound_passes_are_symmetric():
    # Both new compound passes pair every within-group member (C(s,2)); neither is
    # anchored. dropped_pair_estimate and the statement-level toggle skip both branch
    # on this membership, so a misfiled pass would corrupt them.
    assert {"dob+first-initial", "name+sex"} <= SYMMETRIC_PASSES
    assert {"dob+first-initial", "name+sex"} & ANCHORED_PASSES == frozenset()


def test_resolve_none_enables_every_pass():
    assert resolve_enabled_passes(None) == frozenset(ALL_PASSES)


def test_resolve_subset_round_trips():
    subset = {"name", "dob-range"}
    assert resolve_enabled_passes(subset) == frozenset(subset)


def test_resolve_unknown_pass_raises_loudly():
    # A silently-ignored typo would fake an A/B measurement (the pass would look
    # "disabled" while actually misspelled) -- it must raise, naming the offender.
    with pytest.raises(ValueError, match="dob-rnage"):
        resolve_enabled_passes({"dob-rnage"})


def test_pairs_from_anchor_are_canonical_both_directions():
    # B is the anchor: pairs still come out (low, high) by uuid value order.
    assert pairs_from_anchor(B, [A, C]) == {(A, B), (B, C)}


def test_pairs_from_anchor_never_pairs_members_with_each_other():
    # 3 members -> exactly 3 anchor-member pairs, never the C(3,2) member-member ones.
    pairs = pairs_from_anchor(A, [B, C, "dddddddd-dddd-dddd-dddd-dddddddddddd"])
    assert len(pairs) == 3
    assert all(A in p for p in pairs)


def test_pairs_from_anchor_empty_members_is_empty():
    assert pairs_from_anchor(A, []) == set()


def test_pairs_from_anchor_skips_a_self_pair():
    # Defensive: the SQL already excludes anchor==member, but a bug there must not
    # produce a self-pair here (the match_proposal CHECK would reject it downstream).
    assert pairs_from_anchor(A, [A, B]) == {(A, B)}


def test_pairs_from_anchor_normalizes_uuid_case():
    assert pairs_from_anchor(A.upper(), [B.upper()]) == {(A, B)}


def test_pass_shape_sets_partition_the_registry():
    # Every pass is exactly one shape; the two sets cover the registry with no overlap.
    # Downstream arithmetic (dropped_pair_estimate) and the statement-level toggle skip
    # both branch on these sets, so a pass in neither (or both) would corrupt them.
    assert SYMMETRIC_PASSES | ANCHORED_PASSES == frozenset(ALL_PASSES)
    assert SYMMETRIC_PASSES & ANCHORED_PASSES == frozenset()
    assert ANCHORED_PASSES == frozenset({"dob-range", "dob-range+sex"})


def test_dropped_pair_estimate_branches_by_pass_shape():
    # A skipped SYMMETRIC block of size s drops C(s,2) pairs; a skipped ANCHORED block
    # drops only s-1 (anchor x member, never member x member). Charging C(s,2) to an
    # anchored block would overstate a size-4 skip as 6 dropped pairs instead of 3 --
    # quadratically worse at real block sizes -- and skew the eval's recall numbers.
    assert dropped_pair_estimate([("dob", "2000-01-01", 3)]) == 3          # C(3,2)
    assert dropped_pair_estimate([("dob-range", "anchor-uuid", 4)]) == 3   # s-1
    assert dropped_pair_estimate([
        ("name", "smith", 5),            # C(5,2) = 10
        ("dob-range+sex", "anchor", 5),  # s-1    = 4
    ]) == 14
    assert dropped_pair_estimate([]) == 0


def test_require_registered_accepts_a_pass_declared_for_its_statement():
    assert require_registered("dob", SYMMETRIC_PASSES) == "dob"
    assert require_registered("dob-range", ANCHORED_PASSES) == "dob-range"


def test_require_registered_raises_on_an_unregistered_sql_pass():
    # The caller-side toggle raises on unknown names (resolve_enabled_passes), but the
    # fetch loops filter rows by pass_name -- a SQL arm emitting a name missing from the
    # registry would be SILENTLY dropped on every run (a pass that looks built but
    # contributes zero pairs, faking any A/B measurement). RuntimeError, not ValueError:
    # this is SQL<->registry drift (an internal invariant), not a caller mistake.
    with pytest.raises(RuntimeError, match="dob-rnage"):
        require_registered("dob-rnage", SYMMETRIC_PASSES)


def test_require_registered_raises_on_a_pass_from_the_wrong_statement():
    # A REGISTERED pass emitted by the wrong statement is drift too: the statement-level
    # toggle skip ("skip a statement when none of its declared passes is enabled") is
    # only sound while every arm's literal belongs to its statement's declared set -- a
    # symmetric arm added to the anchored statement (or vice versa) would be skipped
    # with it and silently contribute nothing when solely enabled.
    with pytest.raises(RuntimeError, match="dob-range"):
        require_registered("dob-range", SYMMETRIC_PASSES)
