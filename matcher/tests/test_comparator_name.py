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


def test_empty_token_name_is_insufficient_data_not_disagree():
    # §3.7 (no-data-is-never-disagreement): a name that projected to NO tokens is
    # absence, not a clash. It must contribute zero evidence, never the DISAGREE penalty.
    a = frozenset({n(["alice"], ["kim"])})
    b = frozenset({Name(tokens={})})  # a present name record carrying no tokens
    assert compare_name_set(a, b, CTX) is AgreementLevel.INSUFFICIENT_DATA


def test_real_name_rescues_a_co_present_empty_token_name():
    # If one side carries both an empty-token name AND a real matching name, the real
    # pair must still win (best across the history-set cross-product).
    a = frozenset({Name(tokens={}), n(["alex"], ["kim"])})
    b = frozenset({n(["alex"], ["kim"])})
    assert compare_name_set(a, b, CTX) is AgreementLevel.EXACT


def test_subset_name_grades_partial_not_disagree():
    # §3.7 at token granularity (the fix for the ADR-0014 cultural-bias footgun): a
    # SHORTER recorded name is overwhelmingly MISSING tokens, not CONFLICTING ones.
    # "Mary Smith" vs "Mary Jane Smith" — every token in the shorter bag matches — must
    # be PARTIAL positive evidence (a consistent coarsening), never the DISAGREE penalty
    # that systematically punishes multi-given-name / compound-surname cultures.
    a = frozenset({n(["mary"], ["smith"])})
    b = frozenset({n(["mary", "jane"], ["smith"])})
    assert compare_name_set(a, b, CTX) is AgreementLevel.PARTIAL


def test_subset_name_with_a_second_extra_token_still_partial():
    # More than one extra token on the longer side is still pure coarsening as long as
    # every shorter-bag token finds a partner.
    a = frozenset({n(["mary"], ["smith"])})
    b = frozenset({n(["mary", "jane", "wei"], ["smith"])})
    assert compare_name_set(a, b, CTX) is AgreementLevel.PARTIAL


def test_subset_name_with_a_real_token_clash_disagrees():
    # A genuinely conflicting token (no partner for the shorter bag) is still DISAGREE —
    # PARTIAL is reserved for pure missing-token coarsening, not for a real mismatch.
    a = frozenset({n(["mary", "zhang"], ["smith"])})
    b = frozenset({n(["mary", "jane", "wei"], ["smith"])})  # 'zhang' has no partner
    assert compare_name_set(a, b, CTX) is AgreementLevel.DISAGREE


def test_subset_grading_is_symmetric():
    # Argument order must not change the outcome (design §7: score(A,B) == score(B,A)).
    a = frozenset({n(["mary"], ["smith"])})
    b = frozenset({n(["mary", "jane"], ["smith"])})
    assert compare_name_set(a, b, CTX) is compare_name_set(b, a, CTX)
