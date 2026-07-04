# matcher/tests/test_blocking_passes.py
"""Pure tests for the blocking-pass registry and the anchored pair helper.

No DB, no psycopg: pipeline/blocking.py is deliberately pure (the placeholder_uses
precedent) so the A/B toggle and anchored-pair semantics are testable in the pure suite.
"""

import pytest

from cairn_matcher.pipeline.blocking import (
    ALL_PASSES,
    pairs_from_anchor,
    resolve_enabled_passes,
)

A = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
B = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
C = "cccccccc-cccc-cccc-cccc-cccccccccccc"


def test_all_passes_is_the_six_known_names():
    assert ALL_PASSES == (
        "identifier", "dob", "name", "name+year", "dob-range", "dob-range+sex",
    )


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
