# §5.4 Birth-Year-Range Blocking Pass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A chart carrying a `year-range` DOB (a John Doe's clinician-observed estimated age) gets blocked into candidate pairs by age evidence itself, via two additive *anchored* blocking passes, plus an `enabled_passes` A/B toggle on `generate_candidate_pairs`.

**Architecture:** All changes live in the advisory Python matcher (`matcher/`), per the approved design
`docs/superpowers/specs/2026-07-04-dob-range-blocking-pass-design.md`. A new pure, psycopg-free
`pipeline/blocking.py` holds the pass registry, toggle validation, and the anchored pair helper
(the `placeholder_uses` precedent: pure logic importable without the `pipeline` extra).
`pipeline/db.py` gains a second, *anchored* SQL statement (`birth_window` + `blocking_sex` CTEs,
two `UNION ALL` arms `dob-range` / `dob-range+sex`) and filters all rows by the enabled-pass set.
No `db/` floor change, no SCHEMA bump, no Rust change.

**Tech Stack:** Python 3 (uv project `matcher/`, AGPL-3.0), psycopg (existing `pipeline` extra only), pytest. No new dependencies.

## Global Constraints

- **AGPL-3.0**; no new dependencies of any license (nothing to add).
- **uv, never venv/pip**: pure suite `cd matcher && uv run pytest`; DB-gated suite
  `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest`.
- **The pure core never imports psycopg** — `pipeline/blocking.py` must be importable with no `pipeline` extra.
- **TDD**: write the failing test first, run it, then implement. Every step below follows that order.
- **House comment style**: every non-trivial function documents *why it exists and how it fits* for a junior developer.
- **Files < 500 lines** (db.py is 257 today; the additions keep it well under).
- **Advisory tier only**: no `db/*.sql` edit, no SCHEMA bump, no ADR/spec change.
- **Lint**: `cd matcher && uv run ruff check .` must stay clean.
- All work on branch `claude/dob-range-blocking` (already created; spec committed on it).

---

### Task 1: Pure pass registry + anchored pair helper (`pipeline/blocking.py`)

**Files:**
- Create: `matcher/src/cairn_matcher/pipeline/blocking.py`
- Test: `matcher/tests/test_blocking_passes.py`

**Interfaces:**
- Consumes: nothing (stdlib only — `uuid`).
- Produces (Tasks 2–5 rely on these exact names):
  - `ALL_PASSES: tuple[str, ...]` = `("identifier", "dob", "name", "name+year", "dob-range", "dob-range+sex")`
  - `resolve_enabled_passes(enabled_passes) -> frozenset[str]` — `None` → all six; unknown name → `ValueError`.
  - `pairs_from_anchor(anchor, members) -> set[tuple[str, str]]` — canonical (low, high) lowercase-uuid anchor×member pairs only.

- [ ] **Step 1: Write the failing tests**

Create `matcher/tests/test_blocking_passes.py`:

```python
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd matcher && uv run pytest tests/test_blocking_passes.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'cairn_matcher.pipeline.blocking'`

- [ ] **Step 3: Write the implementation**

Create `matcher/src/cairn_matcher/pipeline/blocking.py`:

```python
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd matcher && uv run pytest tests/test_blocking_passes.py -v`
Expected: 8 passed

- [ ] **Step 5: Lint and commit**

```bash
cd matcher && uv run ruff check .
cd /Users/hherb/src/cairn-ehr
git add matcher/src/cairn_matcher/pipeline/blocking.py matcher/tests/test_blocking_passes.py
git commit -m "feat(matcher): pure blocking-pass registry + anchored pair helper

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: Wire the `enabled_passes` toggle into `generate_candidate_pairs`

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/db.py` (function `generate_candidate_pairs`, currently lines 208–233)
- Test: `matcher/tests/test_candidate_generation.py` (append)

**Interfaces:**
- Consumes: `resolve_enabled_passes` from Task 1.
- Produces: `generate_candidate_pairs(conn, *, max_block_size: int = 100, enabled_passes=None)` —
  same return shape `(pairs, skipped_blocks)`; rows whose `pass_name` is not enabled are ignored.
  Tasks 3–5 rely on this exact signature.

- [ ] **Step 1: Write the failing tests**

Append to `matcher/tests/test_candidate_generation.py`:

```python
# ---------------------------------------------------------------------------
# A/B pass-toggle (enabled_passes): measurement tooling for pass changes.
# ---------------------------------------------------------------------------


def test_toggle_disabling_dob_removes_only_that_pass(pg_conn):
    # PA-PB share an exact DOB; PA-PC share an identifier. Disabling 'dob' must drop
    # the DOB pair while the identifier pair still comes through -- the toggle selects
    # passes, it does not short-circuit the run.
    seed_patient(pg_conn, PA, dob=("1980-07-15", 20), identifiers=[("mrn:a", "7", "7")])
    seed_patient(pg_conn, PB, dob=("1980-07-15", 20))
    seed_patient(pg_conn, PC, identifiers=[("mrn:a", "7", "7")])
    pairs = _pairs(pg_conn, enabled_passes={"identifier", "name", "name+year"})
    assert canonical_pair(PA, PC) in pairs
    assert canonical_pair(PA, PB) not in pairs


def test_toggle_default_none_equals_all_passes(pg_conn):
    # Regression pin: passing nothing and passing every pass name are the same run.
    from cairn_matcher.pipeline.blocking import ALL_PASSES
    seed_patient(pg_conn, PA, dob=("1980-07-15", 20), names=[("Alex Smith", 20)])
    seed_patient(pg_conn, PB, dob=("1980-07-15", 20), names=[("Alex Jones", 20)])
    assert _pairs(pg_conn) == _pairs(pg_conn, enabled_passes=set(ALL_PASSES))


def test_toggle_unknown_pass_name_raises(pg_conn):
    import pytest
    from cairn_matcher.pipeline.db import generate_candidate_pairs
    with pytest.raises(ValueError, match="no-such-pass"):
        generate_candidate_pairs(pg_conn, enabled_passes={"no-such-pass"})
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_candidate_generation.py -k toggle -v`
Expected: 3 FAIL — `TypeError: generate_candidate_pairs() got an unexpected keyword argument 'enabled_passes'`

- [ ] **Step 3: Implement the toggle**

In `matcher/src/cairn_matcher/pipeline/db.py`:

Add to the imports (after the `banding` import, keeping the group sorted):

```python
from cairn_matcher.pipeline.blocking import pairs_from_anchor, resolve_enabled_passes
```

(`pairs_from_anchor` is consumed in Task 3; importing both now keeps this edit single-touch.
If ruff flags the unused import at this task, import only `resolve_enabled_passes` here and
add `pairs_from_anchor` in Task 3.)

Replace the `generate_candidate_pairs` definition and docstring:

```python
def generate_candidate_pairs(
    conn, *, max_block_size: int = 100, enabled_passes=None
) -> tuple[list[tuple[str, str]], list[tuple[str, str, int]]]:
    """Generate canonical candidate pairs via the blocking passes, capping huge blocks.

    Six passes (see blocking.ALL_PASSES): four SYMMETRIC group passes (identifier /
    exact-DOB / name-token / name-token+birth-year, _GROUPS_SQL) and two ANCHORED
    birth-year-range passes (dob-range / dob-range+sex, _RANGE_GROUPS_SQL — added in the
    §5.4 range-blocking slice; Task 3 wires them).

    `enabled_passes` is the A/B measurement toggle: None runs every pass; a set runs only
    the named ones (unknown names raise — see blocking.resolve_enabled_passes). Filtering
    happens on the returned rows' pass_name, so one run issues the same SQL regardless of
    the subset and the toggle can never change what a pass WOULD have produced.

    Returns (pairs, skipped_blocks). `pairs`: unique canonical (low, high) lowercase-uuid
    tuples from every enabled group with <= max_block_size members. `skipped_blocks`: the
    (pass_name, key, size) of each ENABLED group excluded for exceeding the cap — a block
    shared by hundreds of people is non-discriminating (a group of size k contributes
    C(k,2) pairs; an anchored block of size k contributes k-1), and the §5.13 hub
    duplicate-sweep is the declared backstop for what it drops.

    Read-only — opens a read transaction the CALLER must close (sweep does conn.rollback
    before its write loop, so a long sweep does not pin the xmin horizon).
    """
    enabled = resolve_enabled_passes(enabled_passes)
    pairs: set[tuple[str, str]] = set()
    skipped_blocks: list[tuple[str, str, int]] = []
    with conn.cursor() as cur:
        # The single %s binds the placeholder-use exclusion in the name_tokens CTE.
        cur.execute(_GROUPS_SQL, (_PLACEHOLDER_USES_PARAM,))
        for pass_name, key, members in cur.fetchall():
            if pass_name not in enabled:
                continue
            size = len(members)
            if size > max_block_size:
                skipped_blocks.append((pass_name, key, size))
            else:
                pairs.update(_pairs_from_members(members))
    return sorted(pairs), skipped_blocks
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_candidate_generation.py -v`
Expected: all pass (the 12 existing + 3 new). If ruff would flag `pairs_from_anchor` as unused, keep only `resolve_enabled_passes` in the import for now (see Step 3 note).

- [ ] **Step 5: Lint and commit**

```bash
cd matcher && uv run ruff check .
cd /Users/hherb/src/cairn-ehr
git add matcher/src/cairn_matcher/pipeline/db.py matcher/tests/test_candidate_generation.py
git commit -m "feat(matcher): enabled_passes A/B toggle on generate_candidate_pairs

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: The anchored `dob-range` pass (SQL + anchored cap loop)

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/db.py` (add `_RANGE_GROUPS_SQL`; extend `generate_candidate_pairs`)
- Test: `matcher/tests/test_dob_range_blocking.py` (create)

**Interfaces:**
- Consumes: `pairs_from_anchor` (Task 1), the toggle wiring (Task 2), conftest `seed_patient`
  (a `year-range` dob seeds as `dob=("1981/1991", 30, "year-range")` — the existing precision
  slot; no conftest change needed for this task).
- Produces: rows `(pass_name, anchor_uuid, members uuid[])` from `_RANGE_GROUPS_SQL`; the
  `dob-range` arm live end-to-end. Task 4 adds the `dob-range+sex` arm to the SAME statement.

- [ ] **Step 1: Write the failing tests**

Create `matcher/tests/test_dob_range_blocking.py`:

```python
# matcher/tests/test_dob_range_blocking.py
"""Integration tests for the anchored birth-year-range blocking passes (§5.4).

A chart carrying a `year-range` dob (value "<min>/<max>", the clinician-observed
estimated-age window from slice B) anchors a block of every chart whose birth-year
window overlaps its own. Pairs are anchor-x-member ONLY (see pipeline/blocking.py for
why all-pairing would manufacture noise). Gated on CAIRN_TEST_PG.
"""

from cairn_matcher.pipeline.runner import canonical_pair
from tests.conftest import seed_patient

PA = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"   # the John Doe / range chart (anchor)
PB = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
PC = "cccccccc-cccc-cccc-cccc-cccccccccccc"
PD = "dddddddd-dddd-dddd-dddd-dddddddddddd"


def _gen(conn, **kw):
    from cairn_matcher.pipeline.db import generate_candidate_pairs
    return generate_candidate_pairs(conn, **kw)


def _pairs(conn, **kw):
    pairs, _skipped = _gen(conn, **kw)
    return pairs


def test_point_dob_inside_window_pairs_with_the_range_chart(pg_conn):
    # The core §5.4 case: John Doe estimated ~40±5 (window 1981-1991); the prior chart
    # was born 1985. No shared name (callsign only, excluded), no shared identifier --
    # ONLY the range pass can surface this pair.
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"),
                 callsign_names=[("Unknown-ed-bay3-2026-07-04-aaaaaaaa", 10)])
    seed_patient(pg_conn, PB, dob=("1985-06-15", 20), names=[("Alex Smith", 20)])
    assert canonical_pair(PA, PB) in _pairs(pg_conn)


def test_point_dob_outside_window_does_not_pair(pg_conn):
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"))
    seed_patient(pg_conn, PB, dob=("1995-01-01", 20), names=[("Alex Smith", 20)])
    assert canonical_pair(PA, PB) not in _pairs(pg_conn)


def test_window_boundary_is_inclusive(pg_conn):
    # Mirrors compare_dob's inclusive interval semantics: born exactly at the window
    # max (1991) still groups.
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"))
    seed_patient(pg_conn, PB, dob=("1991-12-31", 20))
    assert canonical_pair(PA, PB) in _pairs(pg_conn)


def test_overlapping_ranges_pair_disjoint_do_not(pg_conn):
    # Two John Does at two sites (the two-callsigns case): overlapping windows are the
    # ONLY key such a pair can ever share. A third, disjoint window must not join.
    seed_patient(pg_conn, PA, dob=("1980/1990", 30, "year-range"))
    seed_patient(pg_conn, PB, dob=("1988/1995", 30, "year-range"))
    seed_patient(pg_conn, PC, dob=("2000/2005", 30, "year-range"))
    pairs = _pairs(pg_conn)
    assert canonical_pair(PA, PB) in pairs
    assert canonical_pair(PA, PC) not in pairs
    assert canonical_pair(PB, PC) not in pairs


def test_members_are_never_paired_with_each_other(pg_conn):
    # PB and PC are both inside PA's window but share nothing else. The anchored pass
    # must pair each with PA and NOT with each other (the all-pairs noise the design
    # rejects: being born within the same decade is not a signal).
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"))
    seed_patient(pg_conn, PB, dob=("1983-01-01", 20), names=[("Alex Smith", 20)])
    seed_patient(pg_conn, PC, dob=("1989-01-01", 20), names=[("Robin Jones", 20)])
    pairs = _pairs(pg_conn)
    assert canonical_pair(PA, PB) in pairs
    assert canonical_pair(PA, PC) in pairs
    assert canonical_pair(PB, PC) not in pairs


def test_malformed_range_values_degrade_silently(pg_conn):
    # Inverted ("1991/1981") and non-numeric ("about-forty") year-range values are
    # EXCLUDED (safe degrade mirroring parse_dob): no crash, no pairs, no false group.
    seed_patient(pg_conn, PA, dob=("1991/1981", 30, "year-range"))
    seed_patient(pg_conn, PB, dob=("about-forty", 30, "year-range"))
    seed_patient(pg_conn, PC, dob=("1985-06-15", 20))
    pairs = _pairs(pg_conn)
    assert canonical_pair(PA, PC) not in pairs
    assert canonical_pair(PB, PC) not in pairs


def test_oversized_range_block_is_skipped_and_reported(pg_conn):
    # cap=3: anchor + 3 in-window members = block size 4 > 3 -> the whole anchored
    # block is skipped and reported under the anchor's uuid, pairs withheld (the hub
    # sweep is the declared backstop). Size counts the ANCHOR TOO: it is the block.
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"))
    for p in (PB, PC, PD):
        seed_patient(pg_conn, p, dob=("1985-06-15", 20))
    pairs, skipped = _gen(pg_conn, max_block_size=3)
    assert canonical_pair(PA, PB) not in pairs
    assert any(pn == "dob-range" and key == PA and sz == 4 for pn, key, sz in skipped)


def test_toggle_disables_the_range_pass(pg_conn):
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"))
    seed_patient(pg_conn, PB, dob=("1985-06-15", 20))
    assert canonical_pair(PA, PB) in _pairs(pg_conn)
    off = _pairs(pg_conn, enabled_passes={"identifier", "dob", "name", "name+year"})
    assert canonical_pair(PA, PB) not in off
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_dob_range_blocking.py -v`
Expected: FAIL — the in-window/overlap/boundary/members/oversized/toggle tests fail on missing pairs (`canonical_pair(PA, PB) in []`); the outside-window and malformed tests may already pass (they assert absence). 6 of 8 failing is the expected shape.

- [ ] **Step 3: Implement the anchored SQL + loop**

In `matcher/src/cairn_matcher/pipeline/db.py`, ensure the blocking import carries both names:

```python
from cairn_matcher.pipeline.blocking import pairs_from_anchor, resolve_enabled_passes
```

Add below `_GROUPS_SQL` (after line ~186):

```python
# The two ANCHORED birth-year-range passes (§5.4 slice: design 2026-07-04). Separate
# statement from _GROUPS_SQL because the pair semantics differ: these rows are
# (pass_name, anchor, members) and Python pairs ANCHOR x MEMBER only -- never member x
# member (see pipeline/blocking.py for why all-pairing a birth-year window would
# manufacture C(k,2) noise pairs).
#
# birth_window gives every chart an inclusive birth-year interval:
#   * range rows: facets precision 'year-range', value '<yyyy>/<yyyy>' (slice B's
#     estimated-age window). Guards mirror parse_dob's safe degrade -- a malformed or
#     inverted value is EXCLUDED (never a false group, only a withheld rescue).
#   * point rows: the existing first-4-digit-run rule -> [year, year]. year-range rows
#     are excluded from this branch so a range can never double-enter as a false point
#     [min, min].
# The overlap join (m.y_min <= a.y_max AND a.y_min <= m.y_max) anchored on is_range
# charts yields range<->point AND range<->range (two John Does, two sites -- the only
# key that pair can ever share) from the same predicate.
#
# blocking_sex is the UNION of a chart's sex-at-birth and administrative-sex values:
# recall-first (a trans patient whose administrative-sex matches the clinician's
# observation still groups even though sex-at-birth differs). 'dob-range+sex' is the
# additive RESCUE pass, mirroring name/name+year: in a big DB the plain window block
# exceeds the cap and is skipped+reported; intersecting with a shared sex value roughly
# halves it, so it fires within cap in more settings. Additive-only: a sex mismatch
# merely means the rescue does not fire -- the scorer never sees a suppression.
_RANGE_GROUPS_SQL = """
WITH birth_window AS (
    SELECT patient_id,
           split_part(value, '/', 1)::int AS y_min,
           split_part(value, '/', 2)::int AS y_max,
           TRUE AS is_range
    FROM patient_demographic
    WHERE field = 'dob'
      AND facets ->> 'precision' = 'year-range'
      AND value ~ '^[0-9]{4}/[0-9]{4}$'
      AND split_part(value, '/', 1)::int <= split_part(value, '/', 2)::int
    UNION ALL
    SELECT patient_id,
           substring(value FROM '[0-9]{4}')::int,
           substring(value FROM '[0-9]{4}')::int,
           FALSE
    FROM patient_demographic
    WHERE field = 'dob'
      AND value ~ '[0-9]{4}'
      AND (facets ->> 'precision') IS DISTINCT FROM 'year-range'
),
blocking_sex AS (
    SELECT DISTINCT patient_id, lower(value) AS sex
    FROM patient_demographic
    WHERE field IN ('sex-at-birth', 'administrative-sex') AND value IS NOT NULL
),
window_overlap AS (
    SELECT a.patient_id AS anchor, m.patient_id AS member
    FROM birth_window a
    JOIN birth_window m
      ON m.patient_id <> a.patient_id
     AND m.y_min <= a.y_max
     AND a.y_min <= m.y_max
    WHERE a.is_range
)
SELECT 'dob-range' AS pass_name, anchor, array_agg(DISTINCT member) AS members
FROM window_overlap
GROUP BY anchor
"""
```

Extend `generate_candidate_pairs`: inside the `with conn.cursor() as cur:` block, after the
`_GROUPS_SQL` loop, add:

```python
        # The anchored range passes: pairs are anchor x member ONLY. The cap counts the
        # WHOLE block (members + the anchor itself) so "block size" means the same thing
        # for both pair-generation shapes, and a skipped block is reported under the
        # anchor's uuid (its natural key).
        cur.execute(_RANGE_GROUPS_SQL)
        for pass_name, anchor, members in cur.fetchall():
            if pass_name not in enabled:
                continue
            size = len(members) + 1
            if size > max_block_size:
                skipped_blocks.append((pass_name, str(anchor), size))
            else:
                pairs.update(pairs_from_anchor(anchor, members))
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_dob_range_blocking.py tests/test_candidate_generation.py -v`
Expected: all pass (8 new + 15 existing).

- [ ] **Step 5: Lint and commit**

```bash
cd matcher && uv run ruff check .
cd /Users/hherb/src/cairn-ehr
git add matcher/src/cairn_matcher/pipeline/db.py matcher/tests/test_dob_range_blocking.py
git commit -m "feat(matcher): anchored dob-range blocking pass (§5.4 estimated-age window)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: The `dob-range+sex` rescue arm + union-sex seeding

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/db.py` (append a `UNION ALL` arm to `_RANGE_GROUPS_SQL`)
- Modify: `matcher/tests/conftest.py` (add `admin_sex` param to `seed_patient`)
- Test: `matcher/tests/test_dob_range_blocking.py` (append)

**Interfaces:**
- Consumes: `_RANGE_GROUPS_SQL` + the anchored loop (Task 3) — the new arm returns rows of the
  same `(pass_name, anchor, members)` shape, so the Python loop needs NO change.
- Produces: `seed_patient(..., admin_sex=(value, rank))` seeding a `field='administrative-sex'`
  row (used by any future test needing that field).

- [ ] **Step 1: Extend conftest `seed_patient`**

In `matcher/tests/conftest.py`, change the signature (line 68) to:

```python
def seed_patient(
    conn, patient_id, *, dob=None, sex=None, admin_sex=None, names=(), identifiers=(),
    callsign_names=()
):
```

Update the docstring line for sex to:

```python
    dob: (value, provenance_rank[, precision]) tuple or None.
    sex/admin_sex: (value, provenance_rank) tuples or None — sex seeds
    field='sex-at-birth'; admin_sex seeds field='administrative-sex' (the
    apparent/phenotypic field a §5.4 clinician-observed sex lands on).
```

And after the existing `if sex is not None:` block, add:

```python
        if admin_sex is not None:
            value, rank = admin_sex
            cur.execute(
                "INSERT INTO patient_demographic (patient_id, field, value, facets, "
                "provenance, provenance_rank, asserted_hlc_wall, asserted_hlc_count, asserted_origin) "
                "VALUES (%s,'administrative-sex',%s,NULL,'seed',%s,0,0,'seed')",
                (patient_id, value, rank),
            )
```

- [ ] **Step 2: Write the failing tests**

Append to `matcher/tests/test_dob_range_blocking.py`:

```python
# ---------------------------------------------------------------------------
# The 'dob-range+sex' RESCUE arm: window overlap AND a shared blocking-sex value
# (union of sex-at-birth + administrative-sex). Fires within the cap where the plain
# window block is oversized -- mirroring the name / name+year rescue pattern.
# ---------------------------------------------------------------------------


def test_sex_rescues_pair_from_oversized_window_block(pg_conn):
    # cap=3. PA's window catches PB, PC, PD -> plain 'dob-range' block size 4 is
    # skipped (and reported). Only PB shares PA's observed sex, so the 'dob-range+sex'
    # sub-block is size 2 -- within cap -> the true candidate is rescued.
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"),
                 admin_sex=("male", 30))
    seed_patient(pg_conn, PB, dob=("1985-06-15", 20), sex=("male", 40))
    seed_patient(pg_conn, PC, dob=("1983-01-01", 20), sex=("female", 40))
    seed_patient(pg_conn, PD, dob=("1989-01-01", 20))          # no sex row at all
    pairs, skipped = _gen(pg_conn, max_block_size=3)
    assert any(pn == "dob-range" and sz == 4 for pn, _key, sz in skipped)
    assert canonical_pair(PA, PB) in pairs
    assert canonical_pair(PA, PC) not in pairs
    assert canonical_pair(PA, PD) not in pairs


def test_union_sex_groups_via_administrative_sex_only(pg_conn):
    # The trans case the like-for-like key would drop: the member's sex-at-birth
    # ('female') differs from the observation, but their administrative-sex ('male')
    # matches -- the UNION of both fields still groups the pair. cap=3 forces the
    # rescue arm to be the only path (plain window block: PA+PB+PC+PD = 4 > 3).
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"),
                 admin_sex=("male", 30))
    seed_patient(pg_conn, PB, dob=("1985-06-15", 20), sex=("female", 40),
                 admin_sex=("male", 30))
    seed_patient(pg_conn, PC, dob=("1983-01-01", 20), sex=("female", 40))
    seed_patient(pg_conn, PD, dob=("1989-01-01", 20))
    pairs, skipped = _gen(pg_conn, max_block_size=3)
    assert any(pn == "dob-range" and sz == 4 for pn, _key, sz in skipped)
    assert canonical_pair(PA, PB) in pairs


def test_sex_mismatch_never_suppresses_the_plain_pass(pg_conn):
    # Additive-only guarantee: with NO cap pressure, a pair whose sexes differ still
    # comes through the plain 'dob-range' pass -- the sex arm can only ADD, never veto.
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"),
                 admin_sex=("male", 30))
    seed_patient(pg_conn, PB, dob=("1985-06-15", 20), sex=("female", 40))
    assert canonical_pair(PA, PB) in _pairs(pg_conn)


def test_sex_arm_toggles_independently(pg_conn):
    # cap=3 as above: with 'dob-range+sex' disabled the rescue disappears; the plain
    # pass alone is still capped out -> no pair. Proves the two arms are independently
    # measurable (the A/B use case).
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"),
                 admin_sex=("male", 30))
    seed_patient(pg_conn, PB, dob=("1985-06-15", 20), sex=("male", 40))
    seed_patient(pg_conn, PC, dob=("1983-01-01", 20))
    seed_patient(pg_conn, PD, dob=("1989-01-01", 20))
    on_pairs, _ = _gen(pg_conn, max_block_size=3)
    off_pairs, _ = _gen(
        pg_conn, max_block_size=3,
        enabled_passes={"identifier", "dob", "name", "name+year", "dob-range"},
    )
    assert canonical_pair(PA, PB) in on_pairs
    assert canonical_pair(PA, PB) not in off_pairs
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_dob_range_blocking.py -v`
Expected: the 3 rescue/union/toggle tests FAIL (no `dob-range+sex` rows exist yet);
`test_sex_mismatch_never_suppresses_the_plain_pass` already passes (it exercises Task 3's arm). 3 of 12 failing is the expected shape.

- [ ] **Step 4: Implement the sex arm**

In `_RANGE_GROUPS_SQL`, append after the final `GROUP BY anchor` line:

```sql
UNION ALL
SELECT 'dob-range+sex', o.anchor, array_agg(DISTINCT o.member)
FROM window_overlap o
JOIN blocking_sex sa ON sa.patient_id = o.anchor
JOIN blocking_sex sm ON sm.patient_id = o.member AND sm.sex = sa.sex
GROUP BY o.anchor
```

(The full constant then reads: `WITH birth_window AS (...), blocking_sex AS (...), window_overlap AS (...) SELECT 'dob-range' ... GROUP BY anchor UNION ALL SELECT 'dob-range+sex' ... GROUP BY o.anchor`.)

No Python change: the anchored loop from Task 3 already handles every row of this shape,
and `blocking_sex` was created in Task 3's CTE list.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_dob_range_blocking.py tests/test_candidate_generation.py tests/test_john_doe_exclusion.py -v`
Expected: all pass.

- [ ] **Step 6: Lint and commit**

```bash
cd matcher && uv run ruff check .
cd /Users/hherb/src/cairn-ehr
git add matcher/src/cairn_matcher/pipeline/db.py matcher/tests/conftest.py matcher/tests/test_dob_range_blocking.py
git commit -m "feat(matcher): dob-range+sex rescue arm (union blocking-sex, additive)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: `birth_year` honesty fix + eval-mirror note

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/db.py` (the `birth_year` CTE in `_GROUPS_SQL`, currently lines 165–169)
- Modify: `matcher/src/cairn_matcher/eval/generator.py` (docstring of `shares_blocking_key`)
- Test: `matcher/tests/test_dob_range_blocking.py` (append)

**Interfaces:**
- Consumes: the toggle (Task 2) — the regression test uses it to isolate `name+year` from the range passes.
- Produces: nothing new — a filter and a comment.

- [ ] **Step 1: Write the failing test**

Append to `matcher/tests/test_dob_range_blocking.py`:

```python
def test_name_year_never_keys_on_a_range_window_min(pg_conn):
    # Honesty fix: "1981/1991" used to leak its first 4-digit run (1981) into the
    # 'name+year' pass as if it were a birth year -- a false key (the window MIN is not
    # a birth year; principle 4). Setup: PA (range dob) and PB (born 1981) share the
    # name token "zed"; PC oversizes the single-token 'zed' block at cap=2, so ONLY
    # 'name+year' could pair PA-PB among the name passes. The range passes are toggled
    # OFF to isolate the assertion (they legitimately pair PA-PB via the window).
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"), names=[("Zed", 20)])
    seed_patient(pg_conn, PB, dob=("1981-05-05", 20), names=[("Zed", 20)])
    seed_patient(pg_conn, PC, dob=("2000-01-01", 20), names=[("Zed", 20)])
    pairs, skipped = _gen(
        pg_conn, max_block_size=2,
        enabled_passes={"identifier", "dob", "name", "name+year"},
    )
    assert any(pn == "name" and sz == 3 for pn, _key, sz in skipped)
    assert canonical_pair(PA, PB) not in pairs
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_dob_range_blocking.py::test_name_year_never_keys_on_a_range_window_min -v`
Expected: FAIL — `canonical_pair(PA, PB)` IS in pairs (the leak exists today: `zed|1981` groups PA and PB).

- [ ] **Step 3: Implement the filter + the mirror note**

In `_GROUPS_SQL`, replace the `birth_year` CTE:

```sql
birth_year AS (
    -- year-range values are EXCLUDED: "1981/1991" would otherwise leak its first
    -- 4-digit run (1981) into name+year as if it were a birth year -- a false key
    -- (the window min is not a birth year; principle 4). The anchored range passes
    -- (_RANGE_GROUPS_SQL) own ranges.
    SELECT patient_id, substring(value FROM '[0-9]{4}') AS year
    FROM patient_demographic
    WHERE field = 'dob' AND value ~ '[0-9]{4}'
      AND (facets ->> 'precision') IS DISTINCT FROM 'year-range'
)
```

In `matcher/src/cairn_matcher/eval/generator.py`, extend the `shares_blocking_key` docstring —
after the sentence about `name+year` being subsumed, add:

```python
    The two anchored range passes ('dob-range' / 'dob-range+sex') are DELIBERATELY
    unmirrored: the generator emits no year-range dobs yet, and an unmirrored pass only
    makes _repair conservative (it under-claims recoverability and repairs via a name
    token anyway -- the safe direction; an OVER-claiming mirror would be the bug).
    Mirror them when the generator learns to emit estimated-age records (deferred in the
    2026-07-04 range-blocking design, section 10).
```

- [ ] **Step 4: Run the full matcher suites**

```bash
cd matcher && uv run pytest -v
cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest -v
cd matcher && uv run ruff check .
```
Expected: pure suite all pass (no psycopg needed); DB suite all pass; ruff clean.

- [ ] **Step 5: Commit**

```bash
cd /Users/hherb/src/cairn-ehr
git add matcher/src/cairn_matcher/pipeline/db.py matcher/src/cairn_matcher/eval/generator.py matcher/tests/test_dob_range_blocking.py
git commit -m "fix(matcher): birth_year CTE excludes year-range values (false name+year key)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: Full verification sweep

**Files:** none created — verification only.

- [ ] **Step 1: Run every matcher suite from a clean state**

```bash
cd matcher && uv run pytest
cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest
cd matcher && uv run ruff check .
```
Expected: pure suite green, DB-gated suite green (240+ tests: 226 prior + the ~15 new), ruff clean.

- [ ] **Step 2: Confirm the untouched surfaces**

```bash
git diff main --stat
```
Expected: changes confined to `matcher/src/cairn_matcher/pipeline/{db,blocking}.py`,
`matcher/src/cairn_matcher/eval/generator.py` (docstring only),
`matcher/tests/{conftest,test_blocking_passes,test_candidate_generation,test_dob_range_blocking}.py`,
and `docs/superpowers/`. NO `db/*.sql`, NO `crates/`, NO SCHEMA change.

- [ ] **Step 3: Sanity-check sweep integration**

`sweep()` calls `generate_candidate_pairs(conn, max_block_size=max_block_size)` — the new
keyword-only `enabled_passes=None` default means sweep behavior is unchanged by construction.
Run its suite to confirm:

```bash
cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_sweep.py -v
```
Expected: all pass.

---

## After the plan

Session wrap-up (NOT plan tasks; per docs/HANDOVER.md house rules): update `docs/HANDOVER.md` +
`docs/ROADMAP.md` (prune < 500 lines), then commit, push `claude/dob-range-blocking`, and open a
PR to `main` referencing the design doc. The spec is already committed on this branch.
