# matcher/src/cairn_matcher/pipeline/blocking.py
"""Pure blocking-pass registry, A/B toggle validation, and anchored pair generation.

PURE and psycopg-free (stdlib uuid only), like cairn_matcher.placeholder_uses: db.py
(psycopg-bound) imports from here, and the pure test suite exercises this module with no
`pipeline` extra installed. Keeping the pass-name registry here gives the A/B toggle one
source of truth a future eval CLI can import without dragging in a DB driver.

Two pair-generation shapes exist in blocking:
  * SYMMETRIC groups (db._pairs_from_members): every within-group pair. Right for keys
    where sharing the key is itself the signal (same identifier, same exact DOB).
  * ANCHORED groups (pairs_from_anchor, here): only anchor-x-member pairs. Right for the
    birth-year-range passes, where the group is "charts inside THIS range chart's window"
    -- two point-DOB members merely being in the same window is NOT a signal, and
    all-pairing them would manufacture C(k,2) noise pairs the exact-DOB pass deliberately
    never produces (design 2026-07-04, section 2).
"""

import uuid

# Every blocking pass generate_candidate_pairs runs, in execution order. The first four
# are the symmetric group passes (_GROUPS_SQL); the last two are the anchored range
# passes (_RANGE_GROUPS_SQL). The A/B toggle validates against this registry.
ALL_PASSES = ("identifier", "dob", "name", "name+year", "dob-range", "dob-range+sex")

_ALL_PASSES_SET = frozenset(ALL_PASSES)


def resolve_enabled_passes(enabled_passes) -> frozenset[str]:
    """Validate an A/B toggle value: None means every pass; unknown names raise.

    Raising (rather than ignoring) an unknown name is load-bearing for measurement
    honesty: a silently-dropped typo ("dob-rnage") would present as "pass disabled" and
    fake a before/after comparison. The error names both the offenders and the valid set.
    """
    if enabled_passes is None:
        return _ALL_PASSES_SET
    requested = frozenset(enabled_passes)
    unknown = requested - _ALL_PASSES_SET
    if unknown:
        raise ValueError(
            f"unknown blocking pass(es): {sorted(unknown)}; valid passes: {list(ALL_PASSES)}"
        )
    return requested


def pairs_from_anchor(anchor, members) -> set[tuple[str, str]]:
    """Canonical (low, high) lowercase-uuid pairs of anchor x each member — nothing else.

    The anchored counterpart to db._pairs_from_members (see module docstring for why the
    range passes must not all-pair). Inputs are normalized to canonical lowercase uuid
    text, where plain string order equals the 128-bit uuid value order used by
    runner.canonical_pair and the match_proposal CHECK — so each pair has one stable
    identity across passes. A member equal to the anchor is skipped (never a self-pair),
    purely as defense in depth: the SQL join already excludes it.
    """
    a = str(uuid.UUID(str(anchor)))
    out: set[tuple[str, str]] = set()
    for m in members:
        mm = str(uuid.UUID(str(m)))
        if mm == a:
            continue
        out.add((a, mm) if a < mm else (mm, a))
    return out
