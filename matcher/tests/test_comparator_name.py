from cairn_matcher.agreement import AgreementLevel, Context
from cairn_matcher.comparators import compare_name_set
from cairn_matcher.records import Name

CTX = Context()


def n(given, family):
    return Name(tokens={"given": tuple(given), "family": tuple(family)})


def test_identical_single_name_is_exact():
    a = frozenset({n(["alex"], ["kim"])})
    b = frozenset({n(["alex"], ["kim"])})
    assert compare_name_set(a, b, CTX) is AgreementLevel.EXACT


def test_given_token_order_is_tolerated():
    a = frozenset({n(["mary", "jane"], ["kim"])})
    b = frozenset({n(["jane", "mary"], ["kim"])})
    assert compare_name_set(a, b, CTX) is AgreementLevel.EXACT


def test_given_family_role_swap_is_tolerated():
    # Same token bag, roles swapped — still matches (role-tolerant bag comparison).
    a = frozenset({n(["kim"], ["alex"])})
    b = frozenset({n(["alex"], ["kim"])})
    assert compare_name_set(a, b, CTX) is AgreementLevel.EXACT


def test_matches_if_any_historical_name_agrees():
    # b's maiden name matches a's only name even though b's current differs.
    a = frozenset({n(["sarah"], ["jones"])})
    b = frozenset({n(["sarah"], ["smith"]), n(["sarah"], ["jones"])})
    assert compare_name_set(a, b, CTX) is AgreementLevel.EXACT


def test_close_tokens_grade_edit_distance():
    a = frozenset({n(["jon"], ["smith"])})
    b = frozenset({n(["john"], ["smith"])})  # jon/john close, smith exact
    assert compare_name_set(a, b, CTX) is AgreementLevel.EDIT_DISTANCE


def test_disjoint_names_disagree():
    a = frozenset({n(["alex"], ["kim"])})
    b = frozenset({n(["chris"], ["jones"])})
    assert compare_name_set(a, b, CTX) is AgreementLevel.DISAGREE


def test_empty_set_is_insufficient_data():
    a = frozenset({n(["alex"], ["kim"])})
    assert compare_name_set(frozenset(), a, CTX) is AgreementLevel.INSUFFICIENT_DATA
    assert compare_name_set(None, a, CTX) is AgreementLevel.INSUFFICIENT_DATA
