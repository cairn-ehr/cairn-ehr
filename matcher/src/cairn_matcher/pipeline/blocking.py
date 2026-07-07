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

# Every blocking pass generate_candidate_pairs runs, in execution order. Six are the
# symmetric group passes (_GROUPS_SQL): identifier / exact-dob / name-token /
# name-token+birth-year / birth-year+first-initial / name-token+sex. The last two are the
# anchored range passes (_RANGE_GROUPS_SQL). The A/B toggle validates against this registry.
#
# The two compound passes are ADDITIVE rescues (like name+year): dob+first-initial groups
# charts sharing a birth-year AND a name-token first-initial (a first-initial RELAXATION of
# the name requirement -- it rescues true matches that share no full name token); name+sex
# groups charts sharing a name-token AND a normalized sex value (a per-sex split that rescues
# an oversized unisex-token 'name' block the cap would otherwise drop wholesale, and the only
# name rescue that fires for the §5.4 John-Doe population, whose DOB is a range or absent).
ALL_PASSES = (
    "identifier", "dob", "name", "name+year",
    "dob+first-initial", "name+sex", "dob-range", "dob-range+sex",
)

_ALL_PASSES_SET = frozenset(ALL_PASSES)

# A pass's SHAPE is machine-readable here, not prose: downstream code branches on it
# (dropped_pair_estimate's arithmetic; db.py's statement-level toggle skip). An anchored
# block of size s yields s-1 pairs (anchor x member); a symmetric one C(s,2).
ANCHORED_PASSES = frozenset({"dob-range", "dob-range+sex"})
SYMMETRIC_PASSES = _ALL_PASSES_SET - ANCHORED_PASSES


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


def require_registered(pass_name: str, declared: frozenset[str]) -> str:
    """Assert a SQL-emitted pass_name is one its statement DECLARES; return it unchanged.

    The complement of resolve_enabled_passes' caller-side check: the fetch loops in
    db.generate_candidate_pairs filter rows by pass_name, so a SQL arm whose literal is
    missing from ALL_PASSES (a typo, or a future pass added to the SQL but not here)
    would be SILENTLY dropped on every run — a pass that looks built but contributes
    zero pairs, faking exactly the A/B measurements the toggle exists to keep honest.

    `declared` is the emitting statement's own pass set (SYMMETRIC_PASSES for
    _GROUPS_SQL, ANCHORED_PASSES for _RANGE_GROUPS_SQL), not just the registry: the
    statement-level toggle skip ("skip a statement when none of its declared passes is
    enabled") is only sound while every arm's literal belongs to its statement's set —
    a registered-but-misplaced arm would be skipped with the wrong statement and
    silently contribute nothing when solely enabled. RuntimeError (not ValueError):
    this is SQL<->registry drift, an internal invariant violation, never a caller
    mistake.
    """
    if pass_name not in declared:
        raise RuntimeError(
            f"blocking SQL emitted pass {pass_name!r}, not one its statement declares "
            f"({sorted(declared)}); register it in blocking.ALL_PASSES and the correct "
            f"shape set (ANCHORED_PASSES/SYMMETRIC_PASSES)"
        )
    return pass_name


def canonical_pair(a, b) -> tuple[str, str]:
    """Order a patient-id pair canonically by uuid VALUE, emitted as lowercase text.

    THE single definition of pair identity (runner re-exports it; pairs_from_anchor and
    db._pairs_from_members build on the same order). The match_proposal CHECK
    (patient_low < patient_high) compares normalized `uuid` values, not their text form.
    Text-sorting the canonical string happens to agree for lowercase UUIDs but diverges
    for upper/mixed case — flipping the pair would violate the CHECK or, worse, store
    propose(a,b) and propose(b,a) as two mirror rows. We compare uuid.UUID objects
    (128-bit integer order = Postgres byte order) so any input case yields one stable
    row identity. Accepts str or uuid.UUID; pure (no DB), hence this module.
    """
    ua, ub = uuid.UUID(str(a)), uuid.UUID(str(b))
    return (str(ua), str(ub)) if ua < ub else (str(ub), str(ua))


def pairs_from_anchor(anchor, members) -> set[tuple[str, str]]:
    """Canonical (low, high) lowercase-uuid pairs of anchor x each member — nothing else.

    The anchored counterpart to db._pairs_from_members (see module docstring for why the
    range passes must not all-pair). Pair identity is canonical_pair's, so each pair has
    one stable identity across passes. A member equal to the anchor yields a would-be
    self-pair (both sides equal after normalization) and is skipped, purely as defense
    in depth: the SQL join already excludes it.
    """
    out: set[tuple[str, str]] = set()
    for m in members:
        low, high = canonical_pair(anchor, m)
        if low != high:
            out.add((low, high))
    return out


def dropped_pair_estimate(skipped_blocks) -> int:
    """Pairs the skipped blocks would have contributed, by pass SHAPE.

    A skipped SYMMETRIC block of size s dropped C(s,2) within-group pairs; a skipped
    ANCHORED block dropped only s-1 (anchor x member — member x member pairs are never
    generated). Charging C(s,2) to an anchored block would overstate its drop
    quadratically (a size-500 hub block: 124,750 phantom pairs instead of 499) and skew
    the blocking-recall numbers the A/B toggle exists to make trustworthy.

    Takes (pass_name, key, size) tuples — the skipped_blocks shape
    generate_candidate_pairs returns.
    """
    return sum(
        (size - 1) if pass_name in ANCHORED_PASSES else size * (size - 1) // 2
        for pass_name, _key, size in skipped_blocks
    )
