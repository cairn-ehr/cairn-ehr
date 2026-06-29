"""The culture-neutral comparator set + the in-house Jaro–Winkler primitive.

Every comparator is pure and field-typed (agreement.Comparator). Each returns
INSUFFICIENT_DATA when a side is absent — a missing field is ZERO evidence, never a
penalty (§3.7, the no-data-is-never-disagreement principle made mechanical).

Jaro–Winkler is implemented here rather than pulled from a dependency: it is short,
fully testable, reviewer-legible, and keeps the project dependency-free (supply-chain
hygiene, house rule #1).
"""

from cairn_matcher.agreement import AgreementLevel, Context
from cairn_matcher.records import MatcherTypeError


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
