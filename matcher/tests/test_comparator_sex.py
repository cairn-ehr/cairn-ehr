"""compare_sex: the composite sex comparator (§5.4 slice, design 2026-07-05).

Branch 1 (both charts carry sex-at-birth) preserves the old compare_exact semantics —
a birth-fact clash stays honest negative evidence. Branch 2 (anything else) is the
positive-only union fallback: intersection -> EXACT, disjoint -> INSUFFICIENT_DATA,
NEVER DISAGREE — clinician-observed evidence may support but never suppress a match.
"""
import pytest

from cairn_matcher.agreement import AgreementLevel, Context
from cairn_matcher.comparators import compare_sex
from cairn_matcher.records import MatcherTypeError, SexValue

CTX = Context()


def test_absent_side_is_insufficient_data():
    assert compare_sex(None, None, CTX) == AgreementLevel.INSUFFICIENT_DATA
    assert compare_sex(SexValue(sex_at_birth="male"), None, CTX) == AgreementLevel.INSUFFICIENT_DATA


def test_both_sex_at_birth_exact():
    a = SexValue(sex_at_birth="male")
    b = SexValue(sex_at_birth="male")
    assert compare_sex(a, b, CTX) == AgreementLevel.EXACT


def test_both_sex_at_birth_disagree_is_preserved():
    # The birth-fact clash stays real negative evidence (aligned with the db/016 veto).
    a = SexValue(sex_at_birth="male")
    b = SexValue(sex_at_birth="female")
    assert compare_sex(a, b, CTX) == AgreementLevel.DISAGREE


def test_both_sab_branch_wins_even_when_admin_would_agree():
    # Branch 1 fires whenever BOTH sides carry sex-at-birth; administrative values do
    # not soften a birth-fact clash (no double-counting, no suppression of the signal).
    a = SexValue(sex_at_birth="male", administrative="female")
    b = SexValue(sex_at_birth="female", administrative="female")
    assert compare_sex(a, b, CTX) == AgreementLevel.DISAGREE


def test_admin_only_vs_sab_intersects_exact():
    # The §5.4 headline shape: Doe has observed administrative-sex only; the prior
    # chart has a real sex-at-birth. Union fallback intersects -> EXACT.
    doe = SexValue(administrative="male")
    prior = SexValue(sex_at_birth="male")
    assert compare_sex(doe, prior, CTX) == AgreementLevel.EXACT


def test_admin_only_vs_admin_only_intersects_exact():
    assert compare_sex(
        SexValue(administrative="male"), SexValue(administrative="male"), CTX
    ) == AgreementLevel.EXACT


def test_fallback_disjoint_is_never_disagree():
    # An apparent-sex misjudgement must not penalise the true pair (positive-only,
    # compare_identifier_sets precedent).
    doe = SexValue(administrative="male")
    prior = SexValue(sex_at_birth="female")
    assert compare_sex(doe, prior, CTX) == AgreementLevel.INSUFFICIENT_DATA


def test_trans_true_match_no_disagree_via_fallback():
    # Chart A: sex-at-birth male + administrative female. Chart B (same person,
    # other site): administrative female only. Branch 2 (B has no sab); A's union
    # {male, female} intersects B's {female} -> EXACT, not DISAGREE.
    a = SexValue(sex_at_birth="male", administrative="female")
    b = SexValue(administrative="female")
    assert compare_sex(a, b, CTX) == AgreementLevel.EXACT


def test_whitespace_trims_and_empty_degrades():
    # Trim-only (no casefold — culture-touching, locale-pack territory, same
    # discipline as compare_exact). An all-whitespace value is absence.
    assert compare_sex(
        SexValue(sex_at_birth=" male "), SexValue(sex_at_birth="male"), CTX
    ) == AgreementLevel.EXACT
    assert compare_sex(
        SexValue(sex_at_birth="   "), SexValue(sex_at_birth="male"), CTX
    ) == AgreementLevel.INSUFFICIENT_DATA


def test_wrong_type_raises():
    with pytest.raises(MatcherTypeError):
        compare_sex("male", SexValue(sex_at_birth="male"), CTX)  # type: ignore[arg-type]
    with pytest.raises(MatcherTypeError):
        compare_sex(SexValue(sex_at_birth=123), SexValue(sex_at_birth="male"), CTX)  # type: ignore[arg-type]
