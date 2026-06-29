from cairn_matcher.agreement import AgreementLevel, Context
from cairn_matcher.comparators import compare_identifier_sets

CTX = Context()


def test_shared_value_in_shared_system_is_exact():
    a = {"au-medicare": frozenset({"2951"})}
    b = {"au-medicare": frozenset({"2951", "3000"})}
    assert compare_identifier_sets(a, b, CTX) is AgreementLevel.EXACT


def test_disjoint_is_insufficient_not_disagree():
    # Identifier MISMATCH is the in-DB veto's job, never a B1 penalty.
    a = {"au-medicare": frozenset({"2951"})}
    b = {"au-medicare": frozenset({"9999"})}
    assert compare_identifier_sets(a, b, CTX) is AgreementLevel.INSUFFICIENT_DATA


def test_no_shared_system_is_insufficient():
    a = {"au-medicare": frozenset({"2951"})}
    b = {"nz-nhi": frozenset({"2951"})}
    assert compare_identifier_sets(a, b, CTX) is AgreementLevel.INSUFFICIENT_DATA


def test_empty_is_insufficient():
    assert compare_identifier_sets({}, {"x": frozenset({"1"})}, CTX) is AgreementLevel.INSUFFICIENT_DATA
