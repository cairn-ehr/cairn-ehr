"""The culture-neutral comparator set + the in-house Jaro–Winkler primitive.

Every comparator is pure and field-typed (agreement.Comparator). Each returns
INSUFFICIENT_DATA when a side is absent — a missing field is ZERO evidence, never a
penalty (§3.7, the no-data-is-never-disagreement principle made mechanical).

Jaro–Winkler is implemented here rather than pulled from a dependency: it is short,
fully testable, reviewer-legible, and keeps the project dependency-free (supply-chain
hygiene, house rule #1).
"""

from collections.abc import Mapping

from cairn_matcher.agreement import AgreementLevel, Context
from cairn_matcher.records import DateValue, MatcherTypeError, Name


def jaro_winkler(s1: str, s2: str, prefix_scale: float = 0.1) -> float:
    """Jaro–Winkler string similarity in [0, 1]. Symmetric.

    Jaro measures matching characters within a sliding window minus transpositions;
    Winkler boosts strings sharing a common prefix (up to 4 chars). Two empty strings
    are identical (1.0); one empty is fully dissimilar (0.0).
    """

    if s1 == s2:
        return 1.0
    if not s1 or not s2:
        return 0.0

    len1, len2 = len(s1), len(s2)
    match_window = max(len1, len2) // 2 - 1
    match_window = max(match_window, 0)

    s1_matches = [False] * len1
    s2_matches = [False] * len2
    matches = 0

    # Count matching characters within the window.
    for i in range(len1):
        start = max(0, i - match_window)
        end = min(i + match_window + 1, len2)
        for j in range(start, end):
            if s2_matches[j] or s1[i] != s2[j]:
                continue
            s1_matches[i] = True
            s2_matches[j] = True
            matches += 1
            break

    if matches == 0:
        return 0.0

    # Count transpositions among the matched characters.
    transpositions = 0
    k = 0
    for i in range(len1):
        if not s1_matches[i]:
            continue
        while not s2_matches[k]:
            k += 1
        if s1[i] != s2[k]:
            transpositions += 1
        k += 1
    transpositions //= 2

    m = matches
    jaro = (m / len1 + m / len2 + (m - transpositions) / m) / 3.0

    # Winkler prefix boost (common prefix up to 4 characters).
    prefix = 0
    for c1, c2 in zip(s1, s2):
        if c1 != c2:
            break
        prefix += 1
        if prefix == 4:
            break

    return jaro + prefix * prefix_scale * (1.0 - jaro)


def _require_str_or_none(value: object, field_name: str) -> str | None:
    """Normalize a string field input: None passes through; a str is trimmed; anything
    else is an adapter bug and raises (we never silently coerce)."""
    if value is None:
        return None
    if not isinstance(value, str):
        raise MatcherTypeError(f"{field_name} expected str or None, got {type(value)!r}")
    return value.strip()


def compare_exact(a: str | None, b: str | None, ctx: Context) -> AgreementLevel:
    """Exact string agreement after trimming surrounding whitespace only.

    Deliberately does NOT casefold or transliterate — those are culture-touching and
    belong to locale packs, not the neutral core.
    """
    sa = _require_str_or_none(a, "compare_exact.a")
    sb = _require_str_or_none(b, "compare_exact.b")
    if sa is None or sb is None:
        return AgreementLevel.INSUFFICIENT_DATA
    return AgreementLevel.EXACT if sa == sb else AgreementLevel.DISAGREE


def compare_edit_distance(a: str | None, b: str | None, ctx: Context) -> AgreementLevel:
    """Graded string agreement by Jaro–Winkler similarity.

    EXACT if identical, EDIT_DISTANCE if similarity >= ctx.edit_distance_threshold,
    else DISAGREE. Missing side -> INSUFFICIENT_DATA.
    """
    sa = _require_str_or_none(a, "compare_edit_distance.a")
    sb = _require_str_or_none(b, "compare_edit_distance.b")
    if sa is None or sb is None:
        return AgreementLevel.INSUFFICIENT_DATA
    if sa == sb:
        return AgreementLevel.EXACT
    if jaro_winkler(sa, sb) >= ctx.edit_distance_threshold:
        return AgreementLevel.EDIT_DISTANCE
    return AgreementLevel.DISAGREE


# DOB parts compared, coarsest to finest. Precision = the prefix of these present.
_DOB_PARTS = ("year", "month", "day")


def compare_dob(a: DateValue | None, b: DateValue | None, ctx: Context) -> AgreementLevel:
    """Precision-aware DOB agreement that PARSES NO DATE STRINGS.

    Compares only the parts BOTH sides carry:
      * every shared part equal AND same precision depth -> EXACT
      * every shared part equal BUT different depth (year-only vs full) -> PARTIAL
        (a consistent coarsening; principle 4 — imprecision is partial agreement)
      * any shared part differs -> DISAGREE
      * a side absent, or no part in common -> INSUFFICIENT_DATA (never a penalty)
    """
    if a is None or b is None:
        return AgreementLevel.INSUFFICIENT_DATA
    if not isinstance(a, DateValue) or not isinstance(b, DateValue):
        raise MatcherTypeError("compare_dob expects DateValue or None")

    shared = [p for p in _DOB_PARTS if getattr(a, p) is not None and getattr(b, p) is not None]
    if not shared:
        return AgreementLevel.INSUFFICIENT_DATA

    if any(getattr(a, p) != getattr(b, p) for p in shared):
        return AgreementLevel.DISAGREE

    # All shared parts agree. Same precision depth on both sides -> EXACT, else PARTIAL.
    depth_a = sum(1 for p in _DOB_PARTS if getattr(a, p) is not None)
    depth_b = sum(1 for p in _DOB_PARTS if getattr(b, p) is not None)
    return AgreementLevel.EXACT if depth_a == depth_b else AgreementLevel.PARTIAL


def _name_token_bag(name: Name) -> list[str]:
    """Flatten a role-tagged name into a flat bag of tokens (role-tolerant).

    Role tolerance matters because given/family is often swapped or mis-tagged on entry,
    and many cultures do not split names the way the data-entry form assumes.
    """
    bag: list[str] = []
    for tokens in name.tokens.values():
        bag.extend(tokens)
    return bag


def _compare_two_names_greedy(a: Name, b: Name, ctx: Context) -> AgreementLevel:
    """Greedy token pairing (smaller bag into larger) — ORDER-DEPENDENT within a bag.

    Each token in the SMALLER bag claims its best-agreeing unused token from the larger.
    The name's level is the WEAKEST link across all matched pairs.

    A differing bag SIZE is treated as MISSING DATA, not a clash. A shorter recorded name
    ("Mary Smith" vs "Mary Jane Smith") is overwhelmingly missing tokens, not conflicting
    ones — §3.7 (no-data-is-never-disagreement) applied at token granularity. So when
    every token of the smaller bag finds a partner but sizes differ, the result is capped
    at PARTIAL (a consistent coarsening — positive evidence), never DISAGREE. This is the
    fix for a systematic cultural bias: multi-given-name and compound-surname conventions
    are recorded at varying token depths far more often than Anglo two-token names, and
    the old equal-size-or-DISAGREE rule penalised exactly the population ADR-0014 protects.

    A genuine token CONFLICT (a smaller-bag token with NO agreeing partner) is still
    DISAGREE — PARTIAL is reserved for pure missing-token coarsening.

    An EMPTY token bag is ABSENCE, not a clash: grades INSUFFICIENT_DATA (zero evidence,
    §3.7), never DISAGREE.

    This helper is intentionally NOT called directly from outside this module.
    Use _compare_two_names, which neutralises the pairing order-dependency.
    """
    bag_a = _name_token_bag(a)
    bag_b = _name_token_bag(b)
    if not bag_a or not bag_b:
        return AgreementLevel.INSUFFICIENT_DATA

    # Pair the smaller bag into the larger so the surplus tokens on the longer side are
    # unmatched DATA (a coarser name), never a penalty. Symmetric in |a|,|b| by taking
    # the shorter as the driver regardless of argument order.
    smaller = bag_a if len(bag_a) <= len(bag_b) else bag_b
    larger = list(bag_b if len(bag_a) <= len(bag_b) else bag_a)

    worst = AgreementLevel.EXACT
    for token in smaller:
        best_level = AgreementLevel.DISAGREE
        best_idx = -1
        for idx, other in enumerate(larger):
            level = compare_exact(token, other, ctx)
            if level is not AgreementLevel.EXACT:
                level = compare_edit_distance(token, other, ctx)
            if level > best_level:
                best_level = level
                best_idx = idx
        # No agreeing partner for this token -> a real conflict -> DISAGREE.
        if best_idx < 0 or best_level is AgreementLevel.DISAGREE:
            return AgreementLevel.DISAGREE
        larger.pop(best_idx)
        worst = min(worst, best_level)

    # Surplus tokens on the longer side are missing data on the shorter side: cap the
    # matched result at PARTIAL (coarsening), never EXACT/EDIT_DISTANCE.
    if len(bag_a) != len(bag_b):
        worst = min(worst, AgreementLevel.PARTIAL)
    return worst


def _compare_two_names(a: Name, b: Name, ctx: Context) -> AgreementLevel:
    """Best agreement between two single names, comparing token bags order-tolerantly.

    The spec (design §7) requires score(A,B) == score(B,A).  Greedy token pairing is
    inherently order-dependent: which side iterates first determines which tokens get
    paired, and the weakest-link result can differ between the two traversal orders.

    Fix: run the greedy pairing in BOTH directions (A→B and B→A) and return the MAXIMUM
    (strongest) result.  AgreementLevel is an IntEnum so max() is a plain integer
    comparison.  Taking the best of both directions is symmetric by construction — the
    same two directions are always available regardless of which argument is called 'a'.
    This is still greedy/best-effort (not Hungarian-optimal), which is appropriate for
    an advisory matcher, and the result is now invariant to argument order.
    """
    return max(
        _compare_two_names_greedy(a, b, ctx),
        _compare_two_names_greedy(b, a, ctx),
    )


def compare_name_set(
    a: frozenset[Name] | None, b: frozenset[Name] | None, ctx: Context
) -> AgreementLevel:
    """Agreement over two NAME HISTORY SETS — match if ANY historical name pair agrees.

    Operating over the retained set (not just the current display name) is principle-
    bearing (§4.2 / ADR-0014): maiden/married switching, changed family names, and
    discarded aliases still match. Returns the BEST agreement across the cross-product;
    an empty/absent set on either side -> INSUFFICIENT_DATA (zero evidence, not a clash).

    The floor is INSUFFICIENT_DATA, not DISAGREE: if every comparable pair carries an
    empty-token name (no real data), the result stays INSUFFICIENT_DATA (§3.7). A real
    name vs a real non-matching name returns DISAGREE (> INSUFFICIENT_DATA), so genuine
    clashes still surface as DISAGREE.
    """
    if not a or not b:
        return AgreementLevel.INSUFFICIENT_DATA
    best = AgreementLevel.INSUFFICIENT_DATA
    for name_a in a:
        for name_b in b:
            best = max(best, _compare_two_names(name_a, name_b, ctx))
            if best is AgreementLevel.EXACT:
                return best
    return best


def compare_identifier_sets(
    a: "Mapping[str, frozenset[str]]", b: "Mapping[str, frozenset[str]]", ctx: Context
) -> AgreementLevel:
    """POSITIVE-ONLY identifier agreement: EXACT if any shared system shares a value.

    A shared strong identifier is powerful POSITIVE evidence. But identifier MISMATCH is
    deliberately NOT a B1 concern: the same-system mismatch veto is the safety-critical
    in-DB floor's job (db/016, cairn_identifier_veto), which knows the normalized form
    and the honest-degradation rules. So a disjoint or non-overlapping comparison grades
    INSUFFICIENT_DATA (no positive evidence), never DISAGREE — B1 never penalises an ID.
    """
    for system, values_a in a.items():
        values_b = b.get(system)
        if values_b and (values_a & values_b):
            return AgreementLevel.EXACT
    return AgreementLevel.INSUFFICIENT_DATA
