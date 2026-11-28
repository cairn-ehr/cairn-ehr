# matcher/tests/test_synthetic_line.py
"""Pure tests for pipeline.synthetic_line: how a blocking group is split along the line between
synthetic patients and real ones (ADR-0115). No database.
"""

from cairn_matcher.pipeline.banding import VetoFinding
from cairn_matcher.pipeline.synthetic_line import (
    REAL,
    SYNTHETIC,
    SYNTHETIC_SIDE_LABEL,
    both_synthetic,
    crosses_synthetic_line,
    kind_of,
    members_on_the_anchors_side,
    sides,
    straddles,
)

R1 = "11111111-1111-1111-1111-111111111111"
R2 = "22222222-2222-2222-2222-222222222222"
S1 = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
S2 = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
U1 = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee"
U2 = "ffffffff-ffff-ffff-ffff-ffffffffffff"
KINDS = {R1: REAL, R2: REAL, S1: SYNTHETIC, S2: SYNTHETIC}


def test_a_chart_the_mapping_does_not_name_is_unknown_never_real():
    assert kind_of(KINDS, U1) == "unknown"


def test_the_kind_is_found_whatever_the_spelling_of_the_id():
    assert kind_of(KINDS, S1.upper()) == SYNTHETIC
    assert kind_of(KINDS, "{" + R1 + "}") == REAL


def test_a_pair_straddles_only_when_both_kinds_are_known_and_differ():
    assert straddles(R1, S1, KINDS)
    assert straddles(S1, R1, KINDS)
    assert not straddles(R1, R2, KINDS)
    assert not straddles(S1, S2, KINDS)
    assert not straddles(U1, S1, KINDS)
    assert not straddles(R1, U1, KINDS)
    assert not straddles(U1, U2, KINDS)


def test_only_two_known_synthetic_charts_are_a_training_pair():
    assert both_synthetic(S1, S2, KINDS)
    assert not both_synthetic(S1, U1, KINDS)
    assert not both_synthetic(S1, R1, KINDS)


def test_a_group_on_one_side_of_the_line_is_not_split():
    assert sides([R1, R2, U1], KINDS) == [("", [R1, R2, U1])]
    assert sides([S1, S2, U1], KINDS) == [("", [S1, S2, U1])]
    assert sides([U1, U2], KINDS) == [("", [U1, U2])]


def test_a_group_on_both_sides_is_split_and_the_unknown_charts_go_to_both_halves():
    assert sides([R1, S1, U1, R2, S2], KINDS) == [
        ("", [R1, R2, U1]),
        (SYNTHETIC_SIDE_LABEL, [S1, S2, U1]),
    ]


def test_two_synthetic_charts_stay_together():
    # The easy implementation drops anything synthetic. Duplicate-repair training needs the pair.
    halves = dict(sides([R1, S1, S2], KINDS))
    assert halves[SYNTHETIC_SIDE_LABEL] == [S1, S2]


def test_an_anchor_keeps_only_the_members_on_its_own_side_and_the_unknown_ones():
    assert members_on_the_anchors_side(R1, [R2, S1, U1], KINDS) == [R2, U1]
    assert members_on_the_anchors_side(S1, [R2, S2, U1], KINDS) == [S2, U1]


def test_an_unknown_anchor_keeps_every_member():
    assert members_on_the_anchors_side(U1, [R1, S1, U2], KINDS) == [R1, S1, U2]


def test_the_floors_veto_row_is_what_says_a_pair_crosses():
    across = VetoFinding("synthetic_line", "hard_veto", "registration", "…")
    other = VetoFinding("dob_clash", "hard_veto", "dob", "…")
    assert crosses_synthetic_line([other, across])
    assert not crosses_synthetic_line([other])
    assert not crosses_synthetic_line([])
