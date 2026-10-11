# DOB near-miss Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A date of birth one keying error away from another (one digit, adjacent digits swapped, day/month swapped, years one apart) grades `NEAR` instead of `DISAGREE`, scores 0.0, never auto-links, says why, and is blocked even on common names — so "same name, typo'd DOB" reaches a human instead of being silently withdrawn.

**Architecture:** All in the advisory Python matcher (`matcher/`, fit-for-purpose tier, §9). A new ordinal `AgreementLevel.NEAR`; a pure `keying_error_between` helper that `compare_dob` consults; a 0.0 weight; a banding cap plus an evidence entry; one new symmetric blocking pass (`name+monthday`) in the shared blocking SQL (the per-chart path reuses it automatically). No database schema, no Rust, no wire change. One ADR records the comparator-contract addition.

**Tech Stack:** Python 3 (uv-managed), pytest, psycopg (DB-gated tests), PostgreSQL 18 test cluster on `:5532`.

**Spec:** `docs/superpowers/specs/2026-10-11-dob-near-miss-design.md` — read it first; this plan argues from it.

## Global Constraints

- AGPL-3.0; **no new dependencies** in this plan.
- Python work **only through `uv`** (`uv run …`), never pip/venv.
- TDD: every behaviour change starts with a failing test that is RUN and seen to fail.
- `NEAR` is emitted **only** by `compare_dob`, **only** for two full-precision (year+month+day) point dates, **only** for the four patterns: one digit differs · two adjacent digits swapped · day and month swapped (same year) · same day and month, years one apart.
- `NEAR` weight is exactly **0.0**; `NEAR` on any field forbids `AUTO_CANDIDATE`.
- Evidence entry shape: `{"kind": "dob_near_miss", "pattern": <one of the four pattern strings>}` — `"kind"` is the one discriminator key for non-field evidence (runner.py's convention; evidence JSONB is immutable).
- Pattern strings, verbatim: `"one digit differs"`, `"adjacent digits swapped"`, `"day and month swapped"`, `"years one apart"`.
- New blocking pass name, verbatim: `name+monthday` (symmetric).
- Package version `0.1.0` → `0.2.0` in **both** `matcher/pyproject.toml` and `matcher/src/cairn_matcher/__init__.py`, then `uv lock` (comparator *code* is not in the config digest, so the version must move for `matcher_version` to change).
- DB-gated tests: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"`. **Never** run them while `scripts/run-db-gated-tests.sh` (or any other DB suite) runs against the same database — DDL replays deadlock (`40P01`).
- Pure-only runs: `CAIRN_ALLOW_DB_SKIP=1 uv run pytest …` (without it the DB-gate guard fails by design).
- Commit messages: `feat(#NNN)`/`fix(…)` style with the parenthesis; never a bare "fixes #N" (scripts/check_closing_keywords.py). End each commit with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Precondition:** PR #749 is merged and this branch is rebased onto `main` (it adds ADR-0078 and touches the same matcher files). Task 7 numbers the ADR **0079**; if `docs/spec/decisions/0078-*.md` is not on the branch when Task 7 starts, STOP and ask.

## Review Focus

1. **A pair that differs in TWO places** (e.g. 1950-01-07 vs 1951-01-08) must stay `DISAGREE` — a near miss is exactly one keying error, never "close enough". → Task 1 negative tests.
2. **Mixed precision** (1950-07 month-precision vs 1950-07-01; 1950 vs 1951-01-07) must keep today's `PARTIAL`/`DISAGREE` — `NEAR` needs a full date on both sides. → Task 1.
3. **A shared strong identifier** lifting name + `NEAR` past the auto threshold must still band `REVIEW`, never `AUTO_CANDIDATE`. → Task 3.
4. **A month-precision or non-ISO DOB value** (`"1950-07"`, `"07/01/1950"`) must not join `name+monthday` (no false key from a value the pass cannot read). → Task 4.
5. **Same name and same day-month in different decades** (1980-01-01 vs 1991-01-01) is now *generated* by `name+monthday` but must still score `DISAGREE` and not be proposed. → Task 4 (fixture note) + Task 5.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** a records clerk pulling the card for "Mary Smith" and seeing the new card's birthday is the old one with two digits swapped — "same woman, someone mistyped it".
- **Steps:** paper N = 1 human act (compare the two cards) → architecture-forced M = 1 (answer one tray entry) → UI bundling target K = 1. M = N; the architecture adds no act.
- **Time + cognitive load:** budget ≤ 30 s per suspected pair; the entry must show both DOBs and the named keying pattern so the comparison is read, not reconstructed. This slice records the pattern in the proposal's evidence; the tray sentence and the RUNBOOK §12 timed walk are owed by the slice that first renders it (Task 7 files that follow-up).

## File Structure

| File | Responsibility | Tasks |
|---|---|---|
| `matcher/src/cairn_matcher/agreement.py` | the `AgreementLevel` vocabulary — gains `NEAR` | 1 |
| `matcher/src/cairn_matcher/comparators.py` | `keying_error_between` + pattern constants; `compare_dob` emits `NEAR` | 1 |
| `matcher/src/cairn_matcher/scoring.py` | `DEFAULT_WEIGHTS["dob"][NEAR] = 0.0` | 2 |
| `matcher/pyproject.toml`, `matcher/src/cairn_matcher/__init__.py`, `matcher/uv.lock` | version 0.2.0 | 2 |
| `matcher/src/cairn_matcher/pipeline/banding.py` | the auto cap; `dob_near_miss_evidence`; `build_payload(near_miss_evidence=…)` | 3 |
| `matcher/src/cairn_matcher/pipeline/runner.py` | `assess` wires the evidence | 3 |
| `matcher/src/cairn_matcher/pipeline/blocking_sql.py`, `blocking.py` | the `name+monthday` pass | 4 |
| `matcher/tests/…` | one test file per concern (below) | 1–5 |
| `docs/spec/decisions/0079-…md`, `README.md`, `mkdocs.yml`, `docs/spec/identity.md`, `docs/spec/index.md`, `docs/HANDOVER.md`, `docs/ROADMAP.md` | the ADR and its index/nav/spec version; handover | 6, 7 |

---

### Task 1: `NEAR` and the keying-error patterns (pure)

**Files:**
- Modify: `matcher/src/cairn_matcher/agreement.py` (the `AgreementLevel` enum, lines ~20-29, and the module docstring)
- Modify: `matcher/src/cairn_matcher/comparators.py` (add helper above `compare_dob`; `compare_dob` lines ~166-207)
- Create: `matcher/tests/test_dob_near_miss.py`
- Modify: `matcher/tests/test_agreement.py` (ordinal test)
- Modify: `matcher/tests/test_comparator_dob.py` (`test_shared_part_differs_is_disagree` uses 15 vs 16 March — one keying error, so now `NEAR`)

**Interfaces:**
- Produces: `AgreementLevel.NEAR`; `keying_error_between(a: DateValue, b: DateValue) -> str | None`; constants `ONE_DIGIT = "one digit differs"`, `ADJACENT_SWAP = "adjacent digits swapped"`, `DAY_MONTH_SWAP = "day and month swapped"`, `YEARS_ONE_APART = "years one apart"` (all in `cairn_matcher.comparators`).

- [ ] **Step 1: Write the failing tests** — create `matcher/tests/test_dob_near_miss.py`:

```python
"""A DOB one keying error away is NEAR, not DISAGREE (spec 2026-10-11, ADR-0079). Pure."""

import pytest

from cairn_matcher.agreement import AgreementLevel, Context
from cairn_matcher.comparators import (
    ADJACENT_SWAP,
    DAY_MONTH_SWAP,
    ONE_DIGIT,
    YEARS_ONE_APART,
    compare_dob,
    keying_error_between,
)
from cairn_matcher.records import DateValue

CTX = Context()
D = DateValue


@pytest.mark.parametrize("a, b, pattern", [
    (D(1950, 1, 7), D(1950, 1, 8), ONE_DIGIT),          # day digit
    (D(1950, 1, 7), D(1960, 1, 7), ONE_DIGIT),          # a year digit: a decade off
    (D(1980, 3, 12), D(1980, 3, 21), ADJACENT_SWAP),    # day 12 <-> 21
    (D(1958, 3, 12), D(1985, 3, 12), ADJACENT_SWAP),    # year 58 <-> 85
    (D(1950, 7, 1), D(1950, 1, 7), DAY_MONTH_SWAP),     # 01/07 <-> 07/01
    (D(1950, 11, 1), D(1950, 1, 11), DAY_MONTH_SWAP),
    (D(1959, 6, 30), D(1960, 6, 30), YEARS_ONE_APART),  # not one digit: 59 -> 60
])
def test_each_keying_error_is_named(a, b, pattern):
    assert keying_error_between(a, b) == pattern
    assert keying_error_between(b, a) == pattern  # symmetric
    assert compare_dob(a, b, CTX) is AgreementLevel.NEAR
    assert compare_dob(b, a, CTX) is AgreementLevel.NEAR


@pytest.mark.parametrize("a, b", [
    (D(1950, 1, 7), D(1951, 1, 8)),      # two places differ (year and day)
    (D(1980, 3, 12), D(1980, 3, 30)),    # two day digits differ, not a swap
    (D(1950, 7, 1), D(1951, 1, 7)),      # day/month swap across different years
    (D(1950, 1, 7), D(1970, 1, 9)),      # two digits, non-adjacent
])
def test_more_than_one_keying_error_stays_disagree(a, b):
    assert keying_error_between(a, b) is None
    assert compare_dob(a, b, CTX) is AgreementLevel.DISAGREE


def test_identical_dates_are_exact_never_near():
    assert keying_error_between(D(1958, 3, 12), D(1958, 3, 12)) is None
    assert compare_dob(D(1958, 3, 12), D(1958, 3, 12), CTX) is AgreementLevel.EXACT


def test_years_two_apart_with_one_digit_changed_is_one_digit():
    # 1950 -> 1952 changes ONE digit: a keying error by the one-digit rule, though "years one
    # apart" does not apply. Pinned so nobody "fixes" it into DISAGREE.
    assert keying_error_between(D(1950, 1, 7), D(1952, 1, 7)) == ONE_DIGIT


@pytest.mark.parametrize("a, b, expected", [
    (D(1950, 7), D(1950, 7, 1), AgreementLevel.PARTIAL),      # month precision vs full
    (D(1950), D(1951, 1, 7), AgreementLevel.DISAGREE),        # year only, year differs
    (D(1950, 7), D(1950, 8, 1), AgreementLevel.DISAGREE),     # month precision, month differs
    (D(year_min=1945, year_max=1955), D(1950, 1, 7), AgreementLevel.PARTIAL),  # §5.4 range
])
def test_near_needs_a_full_date_on_both_sides(a, b, expected):
    assert keying_error_between(a, b) is None
    assert compare_dob(a, b, CTX) is expected
```

Modify `matcher/tests/test_agreement.py` — replace the ordinal test:

```python
def test_levels_are_ordinal_strongest_is_exact():
    assert AgreementLevel.INSUFFICIENT_DATA < AgreementLevel.DISAGREE
    # NEAR: a clash that may be one keying error — weaker than a consistent coarsening,
    # stronger than a clash (spec 2026-10-11).
    assert AgreementLevel.DISAGREE < AgreementLevel.NEAR < AgreementLevel.PARTIAL
    assert AgreementLevel.PARTIAL < AgreementLevel.EDIT_DISTANCE
    assert AgreementLevel.EDIT_DISTANCE < AgreementLevel.PHONETIC
    assert AgreementLevel.PHONETIC < AgreementLevel.NICKNAME
    assert AgreementLevel.NICKNAME < AgreementLevel.EXACT
    assert max(AgreementLevel.DISAGREE, AgreementLevel.EXACT) is AgreementLevel.EXACT


def test_levels_persist_by_name_never_by_number():
    # Inserting NEAR renumbered every level above it. That is safe ONLY because nothing
    # persists the integer: evidence and the config fingerprint store `.name`. If this test
    # ever needs a number, the renumbering broke stored data.
    import inspect

    from cairn_matcher.eval import model_io
    from cairn_matcher.pipeline import banding
    for module in (banding, model_io):
        src = inspect.getsource(module)
        assert "level.value" not in src and "int(level" not in src, module.__name__
```

Modify `matcher/tests/test_comparator_dob.py` — `test_shared_part_differs_is_disagree` (15 vs 16 is one keying error now):

```python
def test_shared_part_differs_is_disagree():
    # Two days far apart (not one keying error — see test_dob_near_miss.py for those).
    assert (
        compare_dob(DateValue(1980, 3, 15), DateValue(1980, 7, 28), CTX)
        is AgreementLevel.DISAGREE
    )
    assert compare_dob(DateValue(1980), DateValue(1981, 3, 15), CTX) is AgreementLevel.DISAGREE
```

- [ ] **Step 2: Run to verify they fail**

Run: `cd matcher && CAIRN_ALLOW_DB_SKIP=1 uv run pytest tests/test_dob_near_miss.py tests/test_agreement.py tests/test_comparator_dob.py -q`
Expected: FAIL — `ImportError: cannot import name 'ADJACENT_SWAP'` and `AttributeError: NEAR`.

- [ ] **Step 3: Implement.** In `agreement.py`, insert `NEAR` and renumber:

```python
class AgreementLevel(IntEnum):
    """Graded agreement between two field values. Ordinal: higher == stronger.

    Persisted BY NAME only (evidence, the config fingerprint) — never by number — so a level
    may be inserted in order; tests/test_agreement.py pins that.
    """

    INSUFFICIENT_DATA = 0  # a side is absent/unknown -> ZERO evidence (not a penalty, §3.7)
    DISAGREE = 1           # both present, no agreement at any level
    NEAR = 2               # both present and differing by ONE keying error (DOB only) —
                           # consistent with one person and a typo AND with two people (§3.7)
    PARTIAL = 3            # precision-coarsened / weak (e.g. year-only DOB vs full)
    EDIT_DISTANCE = 4      # agree within an edit-distance band
    PHONETIC = 5           # reserved for locale packs — not emitted by this core
    NICKNAME = 6           # reserved for locale packs — not emitted by this core
    EXACT = 7              # exact agreement
```

In `comparators.py`, directly above `compare_dob`:

```python
# The keying errors a DOB near miss recognises (spec 2026-10-11, ADR-0079). The strings are
# persisted in proposal evidence ({"kind": "dob_near_miss", "pattern": …}) and shown to a
# human, so they are plain words and never change once shipped.
ONE_DIGIT = "one digit differs"
ADJACENT_SWAP = "adjacent digits swapped"
DAY_MONTH_SWAP = "day and month swapped"
YEARS_ONE_APART = "years one apart"


def _is_full_point(d: DateValue) -> bool:
    """A point date carrying year, month AND day — the only shape a keying error is read on."""
    return not d.is_range and None not in (d.year, d.month, d.day)


def _keyed_digits(d: DateValue) -> str:
    """The date as the eight digits a clerk types, YYYYMMDD."""
    return f"{d.year:04d}{d.month:02d}{d.day:02d}"


def keying_error_between(a: DateValue, b: DateValue) -> str | None:
    """Name the ONE keying error that turns `a` into `b`, or None. **Pure, symmetric.**

    Only for two full-precision point dates that differ. Checked in this order, first match
    wins, so each pair gets exactly one name:
      1. exactly one of the eight digits differs              -> ONE_DIGIT
      2. exactly two ADJACENT digits differ and are swapped   -> ADJACENT_SWAP
      3. same year, day and month swapped                     -> DAY_MONTH_SWAP
      4. same day and month, years one apart (1959 vs 1960,
         which changes two digits, so rule 1 misses it)       -> YEARS_ONE_APART
    Anything else is not one keying error — a real disagreement, or two people.
    """
    if not (_is_full_point(a) and _is_full_point(b)) or a == b:
        return None
    da, db = _keyed_digits(a), _keyed_digits(b)
    diff = [i for i in range(8) if da[i] != db[i]]
    if len(diff) == 1:
        return ONE_DIGIT
    if (len(diff) == 2 and diff[1] == diff[0] + 1
            and da[diff[0]] == db[diff[1]] and da[diff[1]] == db[diff[0]]):
        return ADJACENT_SWAP
    if a.year == b.year and a.month == b.day and a.day == b.month:
        return DAY_MONTH_SWAP
    if a.month == b.month and a.day == b.day and abs(a.year - b.year) == 1:
        return YEARS_ONE_APART
    return None
```

In `compare_dob`, replace the `DISAGREE` return and add a docstring bullet:

```python
    if any(getattr(a, p) != getattr(b, p) for p in shared):
        # One keying error between two full dates is NEAR (weight 0.0, never auto-linked):
        # it fits one person and a typo as well as two people — principle 4's honest middle.
        return AgreementLevel.NEAR if keying_error_between(a, b) else AgreementLevel.DISAGREE
```

Docstring bullet to add under "any shared part differs -> DISAGREE": `* …unless both are full dates one keying error apart -> NEAR (keying_error_between)`. Update the `agreement.py` module docstring's level list if it enumerates levels.

- [ ] **Step 4: Run to verify they pass, then the whole pure suite**

Run: `cd matcher && CAIRN_ALLOW_DB_SKIP=1 uv run pytest -q`
Expected: all pass. If any OTHER pure test fails because it encoded "a one-keying-error DOB is DISAGREE", change its fixture to a non-keying-error date (as in Step 1's `test_comparator_dob.py` edit), never the assertion's meaning, and list each such test in the commit message. Known candidates to check: `test_scoring.py`, `test_banding.py`, `test_eval_learn.py`, `test_properties.py`.

- [ ] **Step 5: Lint and commit**

```bash
cd matcher && uv run ruff check src tests
git add matcher/src/cairn_matcher/agreement.py matcher/src/cairn_matcher/comparators.py matcher/tests/
git commit -m "feat(matcher): a DOB one keying error away grades NEAR, not DISAGREE"
```

---

### Task 2: `NEAR` weighs 0.0; the matcher version moves

**Files:**
- Modify: `matcher/src/cairn_matcher/scoring.py` (`DEFAULT_WEIGHTS`, lines ~96-114)
- Modify: `matcher/pyproject.toml:3`, `matcher/src/cairn_matcher/__init__.py:22`, `matcher/uv.lock` (via `uv lock`)
- Test: `matcher/tests/test_dob_near_miss.py` (append)

**Interfaces:**
- Consumes: `AgreementLevel.NEAR` (Task 1).
- Produces: `DEFAULT_WEIGHTS.per_field["dob"]` prices `NEAR` at `0.0`; `cairn_matcher.__version__ == "0.2.0"`.

- [ ] **Step 1: Write the failing tests** (append to `test_dob_near_miss.py`):

```python
from cairn_matcher.records import FieldComparison
from cairn_matcher.scoring import DEFAULT_WEIGHTS, FieldWeights, Weights, score


def test_near_is_priced_at_zero_explicitly():
    assert DEFAULT_WEIGHTS.per_field["dob"].weights[AgreementLevel.NEAR] == 0.0


def test_a_learned_table_without_near_still_prices_it_at_zero():
    # An older persisted (B3-learned) table has no NEAR key; score() prices a level missing
    # WITHIN a field's entry at 0.0 — exactly the shipped behaviour, never a crash.
    legacy = Weights(per_field={"dob": FieldWeights({
        AgreementLevel.EXACT: 6.0, AgreementLevel.PARTIAL: 1.5, AgreementLevel.DISAGREE: -4.0,
    })})
    s = score([FieldComparison("dob", AgreementLevel.NEAR, 80)], legacy)
    assert s.total == 0.0


def test_the_package_version_moved_with_the_comparator():
    # matcher_version = package version + a digest of weights/thresholds/config. Comparator
    # CODE is not in the digest, so a behaviour change must move the version (spec: Rollout).
    import cairn_matcher
    assert cairn_matcher.__version__ == "0.2.0"
```

(If `FieldComparison`'s constructor differs — check `records.py:126` — match its real field order: `field, level, provenance_rank`.)

- [ ] **Step 2: Run to verify they fail**

Run: `cd matcher && CAIRN_ALLOW_DB_SKIP=1 uv run pytest tests/test_dob_near_miss.py -q`
Expected: FAIL — `KeyError: <AgreementLevel.NEAR: 2>` and `'0.1.0' == '0.2.0'`.

- [ ] **Step 3: Implement**

`scoring.py`:

```python
    "dob": FieldWeights({
        AgreementLevel.EXACT: 6.0,
        AgreementLevel.PARTIAL: 1.5,
        # One keying error (spec 2026-10-11): honest uncertainty — neither for nor against.
        AgreementLevel.NEAR: 0.0,
        AgreementLevel.DISAGREE: -4.0,
    }),
```

`pyproject.toml`: `version = "0.2.0"`; `__init__.py`: `__version__ = "0.2.0"`; then `cd matcher && uv lock`.

- [ ] **Step 4: Run to verify they pass** — `cd matcher && CAIRN_ALLOW_DB_SKIP=1 uv run pytest -q` → all pass.

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/scoring.py matcher/pyproject.toml matcher/src/cairn_matcher/__init__.py matcher/uv.lock matcher/tests/test_dob_near_miss.py
git commit -m "feat(matcher): NEAR weighs 0.0; matcher 0.2.0 so matcher_version moves"
```

---

### Task 3: the auto cap and the `dob_near_miss` evidence

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/banding.py` (`band`, `build_payload`, new `dob_near_miss_evidence`)
- Modify: `matcher/src/cairn_matcher/pipeline/runner.py` (`assess`, around the `build_payload` call ~line 149)
- Test: `matcher/tests/test_banding.py` (append)

**Interfaces:**
- Consumes: `keying_error_between` (Task 1), `AgreementLevel.NEAR`.
- Produces: `dob_near_miss_evidence(a: DateValue | None, b: DateValue | None) -> tuple[dict, ...]` (in `banding.py`); `build_payload(..., near_miss_evidence: Sequence[dict] = ())`.

- [ ] **Step 1: Write the failing tests** (append to `test_banding.py`):

```python
from cairn_matcher.comparators import DAY_MONTH_SWAP
from cairn_matcher.pipeline.banding import _corroborated_positive, dob_near_miss_evidence
from cairn_matcher.records import DateValue


def _near_score(total: float) -> MatchScore:
    return MatchScore(total=total, fields=(
        FieldEvidence("name", AgreementLevel.EXACT, 60, 5.0),
        FieldEvidence("dob", AgreementLevel.NEAR, 60, 0.0),
        FieldEvidence("identifier", AgreementLevel.EXACT, 60, total - 5.0),
    ))


def test_a_near_miss_dob_never_auto_links_even_with_a_shared_identifier():
    # name 5 + dob NEAR 0 + identifier 8 = 13 >= auto (8), no veto: still REVIEW.
    assert band(_near_score(13.0), ()) is Band.REVIEW


def test_a_near_miss_pair_above_review_is_review():
    s = MatchScore(total=6.0, fields=(
        FieldEvidence("name", AgreementLevel.EXACT, 60, 5.0),
        FieldEvidence("dob", AgreementLevel.NEAR, 60, 0.0),
        FieldEvidence("sex", AgreementLevel.EXACT, 60, 1.0),
    ))
    assert band(s, ()) is Band.REVIEW


def test_near_is_neither_corroboration_nor_a_clash():
    # §5.4 unconfirmed-chart gate: >= 2 positive fields (>= PARTIAL) and NO DISAGREE.
    # NEAR counts as neither — so name + sex + NEAR still corroborates, name + NEAR does not.
    two_pos = MatchScore(total=6.0, fields=(
        FieldEvidence("name", AgreementLevel.EXACT, 60, 5.0),
        FieldEvidence("sex", AgreementLevel.EXACT, 60, 1.0),
        FieldEvidence("dob", AgreementLevel.NEAR, 60, 0.0),
    ))
    one_pos = MatchScore(total=5.0, fields=(
        FieldEvidence("name", AgreementLevel.EXACT, 60, 5.0),
        FieldEvidence("dob", AgreementLevel.NEAR, 60, 0.0),
    ))
    assert _corroborated_positive(two_pos) is True
    assert _corroborated_positive(one_pos) is False


def test_near_miss_evidence_names_the_pattern():
    a, b = DateValue(1950, 1, 7), DateValue(1950, 7, 1)
    assert dob_near_miss_evidence(a, b) == ({"kind": "dob_near_miss", "pattern": DAY_MONTH_SWAP},)
    assert dob_near_miss_evidence(a, a) == ()
    assert dob_near_miss_evidence(a, None) == ()
    assert dob_near_miss_evidence(a, DateValue(1975, 11, 30)) == ()


def test_build_payload_appends_the_near_miss_evidence_last():
    entry = {"kind": "dob_near_miss", "pattern": DAY_MONTH_SWAP}
    p = build_payload(_score(6.0), (), Band.REVIEW, near_miss_evidence=(entry,))
    assert p.evidence[-1] == entry
```

- [ ] **Step 2: Run to verify they fail**

Run: `cd matcher && CAIRN_ALLOW_DB_SKIP=1 uv run pytest tests/test_banding.py -q`
Expected: FAIL — `ImportError: dob_near_miss_evidence`; the auto test returns `Band.AUTO_CANDIDATE`.

- [ ] **Step 3: Implement** in `banding.py`:

```python
from cairn_matcher.comparators import keying_error_between
from cairn_matcher.records import DateValue


def _has_near_miss(score: MatchScore) -> bool:
    """Any field graded NEAR (a DOB one keying error off): never auto-linked — a human must
    decide which DOB is right, and that is also when the wrong one gets corrected."""
    return any(e.level is AgreementLevel.NEAR for e in score.fields)


def dob_near_miss_evidence(a: DateValue | None, b: DateValue | None) -> tuple[dict, ...]:
    """The proposal's `dob_near_miss` entry when the two DOBs are one keying error apart, so
    the reviewer is told WHICH error ("day and month swapped"). **Pure.** Empty otherwise."""
    if a is None or b is None:
        return ()
    pattern = keying_error_between(a, b)
    return ({"kind": "dob_near_miss", "pattern": pattern},) if pattern else ()
```

In `band`, change the auto line to:

```python
    if score.total >= thresholds.auto and not vetoes and not _has_near_miss(score):
        return Band.AUTO_CANDIDATE
```

and add to its docstring: "A field graded NEAR (a DOB one keying error off, spec 2026-10-11) forbids AUTO_CANDIDATE exactly as a veto does."

In `build_payload`, add the parameter `near_miss_evidence: Sequence[dict] = (),` after `trust_evidence`, append it last (`... + tuple(trust_evidence) + tuple(near_miss_evidence)`), and add a docstring paragraph: "`near_miss_evidence` (the `dob_near_miss` entry, if any) is appended last so the reviewer sees which keying error the matcher suspects."

In `runner.assess`, before `payload = build_payload(...)`:

```python
    # A DOB one keying error off scores NEAR (0.0) and never auto-links; the evidence names the
    # error so the reviewer can see it ("day and month swapped").
    near_miss_evidence = dob_near_miss_evidence(
        rec_a.dob.value if rec_a.dob else None, rec_b.dob.value if rec_b.dob else None,
    )
```

and pass `near_miss_evidence=near_miss_evidence` to `build_payload`. Import `dob_near_miss_evidence` alongside `build_payload` at the top of `runner.py`.

Check `banding.py` does not already import from `comparators` in a way that cycles: `comparators` imports only `agreement`/`records`, so the import is safe.

- [ ] **Step 4: Run to verify they pass** — `cd matcher && CAIRN_ALLOW_DB_SKIP=1 uv run pytest -q` → all pass; `uv run ruff check src tests`.

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/pipeline/banding.py matcher/src/cairn_matcher/pipeline/runner.py matcher/tests/test_banding.py
git commit -m "feat(matcher): a near-miss DOB never auto-links, and the proposal names the keying error"
```

---

### Task 4: the `name+monthday` blocking pass

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/blocking_sql.py` (new CTE constant; `_GROUPS_SQL`'s WITH list and a new `UNION ALL` arm)
- Modify: `matcher/src/cairn_matcher/pipeline/blocking.py` (`ALL_PASSES`, its comment "Six are the symmetric group passes" → seven)
- Modify: `matcher/tests/test_blocking_passes.py:26-29` (the pinned tuple)
- Modify: `matcher/tests/test_candidate_generation.py` (new tests; and the fixture of `test_name_year_rescues_pair_from_oversized_name_block` — see Step 1)

**Interfaces:**
- Produces: blocking pass `"name+monthday"` in `ALL_PASSES` and `SYMMETRIC_PASSES`; rows `('name+monthday', '<token>|MM-DD', members)`. `targeted.py` reuses `_GROUPS_SQL`, so the per-chart path gets it with no edit (its drift canary, `tests/test_targeted_blocking.py`, proves it).

- [ ] **Step 1: Write the failing tests.** Update the pinned tuple in `test_blocking_passes.py`:

```python
    assert ALL_PASSES == (
        "identifier", "dob", "name", "name+year",
        "dob+first-initial", "name+sex", "name+monthday", "dob-range", "dob-range+sex",
    )
```

and add `assert "name+monthday" in SYMMETRIC_PASSES` to `test_new_compound_passes_are_symmetric`.

Append to `test_candidate_generation.py` (DB-gated, uses the existing `_gen`, `PA/PB/PC`):

```python
def test_name_monthday_rescues_a_year_typo_on_a_common_name(pg_conn):
    # Three Smiths -> the 'name' block (size 3) is over cap 2 and dropped. PA and PB are one
    # person with a YEAR typo (1950 vs 1951): no shared year (name+year misses), no shared
    # date (dob misses). Only name+monthday (smith|01-07) groups them.
    seed_patient(pg_conn, PA, dob=("1950-01-07", 20), names=[("Smith", 20)])
    seed_patient(pg_conn, PB, dob=("1951-01-07", 20), names=[("Smith", 20)])
    seed_patient(pg_conn, PC, dob=("1972-09-30", 20), names=[("Smith", 20)])
    pairs, skipped = _gen(pg_conn, max_block_size=2)
    assert any(pn == "name" and sz == 3 for pn, _key, sz in skipped)
    assert canonical_pair(PA, PB) in pairs
    assert canonical_pair(PA, PC) not in pairs


def test_name_monthday_reads_only_full_iso_dates(pg_conn):
    # Review Focus 4: a month-precision value, or a non-ISO one, must not join the pass — no
    # false key from a value the pass cannot read (precision is seed_patient's 3rd dob slot).
    seed_patient(pg_conn, PA, dob=("1950-01", 20, "month"), names=[("Smith", 20)])
    seed_patient(pg_conn, PB, dob=("07/01/1951", 20), names=[("Smith", 20)])
    seed_patient(pg_conn, PC, dob=("1950-01-07", 20), names=[("Smith", 20)])
    _pairs_found, skipped = _gen(pg_conn, max_block_size=1)
    keys = [key for pn, key, _sz in skipped if pn == "name+monthday"]
    assert keys == [], "no name+monthday block may form from these three values"
```

(It relies on cap 1 reporting every multi-member block as skipped: with only one well-formed full date there is no `smith|MM-DD` block of size ≥ 2, so no `name+monthday` entry may appear in `skipped`.)

Fix the fixture of `test_name_year_rescues_pair_from_oversized_name_block` (Review Focus 5): PA `1980-01-01` and PC `1991-01-01` now share `smith|01-01`, so `name+monthday` would generate PA–PC and break its "not in pairs" assertion, which is about `name+year`. Change PC's DOB to `"1991-02-02"` and add the comment `# not 01-01: name+monthday would pair a shared day-month (that pass has its own tests)`.

- [ ] **Step 2: Run to verify they fail**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_blocking_passes.py tests/test_candidate_generation.py -q`
Expected: FAIL — the tuple test, and `canonical_pair(PA, PB) in pairs` is False.

- [ ] **Step 3: Implement.** In `blocking_sql.py`, beside `_BIRTH_YEAR_CTE`:

```python
_BIRTH_MONTHDAY_CTE = """birth_monthday AS (
    -- name+monthday's key: the MM-DD of a DAY-precision ISO point DOB, and nothing else.
    -- A year keying error (1950 vs 1951) keeps the day and month, so this groups the pair the
    -- year-keyed passes cannot (spec 2026-10-11). Anchored regex + the precision facet, so a
    -- month-precision, non-ISO or range value never forms a key — a withheld rescue, never a
    -- false group (the birth_year discipline).
    SELECT patient_id, substring(value FROM '^[0-9]{4}-([0-9]{2}-[0-9]{2})$') AS monthday
    FROM patient_demographic
    WHERE field = 'dob' AND (facets ->> 'precision') = 'day'
      AND value ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}$'
)"""
```

Add `{_BIRTH_MONTHDAY_CTE},` to `_GROUPS_SQL`'s WITH list (after `{_BIRTH_YEAR_CTE},`), and a new arm after the `name+sex` arm (before the closing `"""`):

```sql
UNION ALL
-- name+monthday: name token + the DOB's day and month. An ADDITIVE rescue for a year keying
-- error on a common name: the bare 'name' block for "smith" exceeds the cap at scale, and
-- name+year / dob / dob+first-initial all need the same year. Blocks are ~token frequency
-- / 365, so the cap rarely bites. Symmetric (sharing the key is itself the signal).
SELECT 'name+monthday', nt.token || '|' || md.monthday, array_agg(DISTINCT nt.patient_id)
FROM name_tokens nt JOIN birth_monthday md USING (patient_id)
GROUP BY nt.token, md.monthday HAVING count(DISTINCT nt.patient_id) >= 2
```

Check how `_GROUPS_SQL`'s `%s` placeholders are bound in `db.generate_candidate_pairs` and `targeted.py` — the new CTE and arm add **no** `%s`, so the parameter tuples are unchanged.

In `blocking.py`:

```python
ALL_PASSES = (
    "identifier", "dob", "name", "name+year",
    "dob+first-initial", "name+sex", "name+monthday", "dob-range", "dob-range+sex",
)
```

and update the comment above it: "Seven are the symmetric group passes (_GROUPS_SQL): identifier / exact-dob / name-token / name-token+birth-year / birth-year+first-initial / name-token+sex / name-token+birth-month-day." Add one sentence: "name+monthday (spec 2026-10-11) rescues a YEAR keying error on a common name." Confirm `SYMMETRIC_PASSES` is derived (`_ALL_PASSES_SET - ANCHORED_PASSES`) so it picks the pass up with no edit.

Update `test_candidate_generation.py`'s module docstring ("the four blocking passes") to say "the blocking passes".

- [ ] **Step 4: Run to verify they pass — the whole matcher suite, DB-gated** (nothing else may be using `cairn_test`):

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest -q`
Expected: all pass, **including `tests/test_targeted_blocking.py`'s drift canary**. If another DB test asserted a same-name, same-day-month pair is NOT generated, fix its fixture date as above (never the assertion) and name it in the commit message.

- [ ] **Step 5: Commit**

```bash
cd matcher && uv run ruff check src tests
git add matcher/src/cairn_matcher/pipeline/blocking_sql.py matcher/src/cairn_matcher/pipeline/blocking.py matcher/tests/
git commit -m "feat(matcher): name+monthday blocking — a year typo on a common name is still a candidate"
```

---

### Task 5: the Mary Smith case, end to end

**Files:**
- Test: `matcher/tests/test_check_chart.py` (append)

**Interfaces:**
- Consumes: everything above, through the real worker entry point `check_chart(conn, patient, Settings())`.

- [ ] **Step 1: Write the test** (append to `test_check_chart.py`; it already imports `uuid`, `check_chart`, `Settings`, `seed_patient` and defines `_count`):

```python
def test_a_day_month_swap_keeps_the_pair_in_front_of_a_human(pg_conn):
    # The spec's Context: Mary Smith, 7 Jan 1950, two charts, no shared identifier. The pair
    # is an auto candidate; then one DOB is overwritten with the day/month swap (1 Jul 1950).
    # Before NEAR it re-scored 5 - 4 + 1 = 2 and was RETRACTED, unseen. Now it must stay open,
    # at review, and say which keying error the matcher suspects.
    import json
    m1, m2 = (str(uuid.UUID(int=i)) for i in (21, 22))
    with pg_conn.cursor() as cur:
        cur.execute("INSERT INTO match_worker_state (matcher_version) VALUES ('v')")
    pg_conn.commit()
    for m in (m1, m2):
        seed_patient(pg_conn, m, dob=("1950-01-07", 60), sex=("female", 60),
                     names=[("Mary Smith", 60)])
    check_chart(pg_conn, m2, Settings())
    assert _count(pg_conn, "SELECT band FROM match_proposal") == "auto_candidate"

    with pg_conn.cursor() as cur:
        cur.execute("UPDATE patient_demographic SET value = '1950-07-01' "
                    "WHERE patient_id = %s AND field = 'dob'", (m2,))
    pg_conn.commit()
    check_chart(pg_conn, m2, Settings())

    with pg_conn.cursor() as cur:
        cur.execute("SELECT band, status, evidence FROM match_proposal")
        band_, status, evidence = cur.fetchone()
    assert (band_, status) == ("review", "pending"), "kept open for a human, not retracted"
    evidence = evidence if isinstance(evidence, list) else json.loads(evidence)
    assert {"kind": "dob_near_miss", "pattern": "day and month swapped"} in evidence
```

- [ ] **Step 2: Prove the test is a real guard.** Temporarily change the `NEAR` weight in `scoring.py` to `-4.0` and the banding cap off (comment out `and not _has_near_miss(score)`), run the test, and see it FAIL (status `retracted`). Restore both — **by re-applying the edit, never `git checkout -- <file>`** (that discards all uncommitted work in the file). Run `git status --short` and `git diff matcher/src` to confirm only intended changes remain.

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_check_chart.py -q -k day_month_swap`
Expected: PASS with the real code; FAIL with the mutation.

- [ ] **Step 3: Commit**

```bash
git add matcher/tests/test_check_chart.py
git commit -m "test(matcher): the Mary Smith day/month swap stays in front of a human, end to end"
```

---

### Task 6: measure it

**Files:** none committed except the ADR's numbers (Task 7). Scratch output goes to the session scratchpad, never the repo.

- [ ] **Step 1: Baseline vs branch on the synthetic eval set.** Generate one dataset (deterministic) and run the scorer eval on `origin/main`'s matcher and on this branch:

```bash
SP="$(mktemp -d)/near-miss-eval" && mkdir -p "$SP"
cd matcher && uv run python -m cairn_matcher.eval.generate --entities 2000 --seed 7 --out "$SP/synth.json"
uv run python -m cairn_matcher.eval "$SP/synth.json" > "$SP/branch.txt"
git -C .. worktree add "$SP/main-wt" origin/main
(cd "$SP/main-wt/matcher" && uv run python -m cairn_matcher.eval "$SP/synth.json" > "$SP/main.txt")
git -C .. worktree remove "$SP/main-wt"
diff "$SP/main.txt" "$SP/branch.txt"
```

Expected: `missed_match_rate` falls (the generator's `corrupt_dob_typo` makes exactly these pairs); `auto_false_link_rate` does **not** rise (NEAR can never auto-link). Record both lines from each file. If `auto_false_link_rate` rises at all, STOP — that contradicts Decision 3 — and report.

- [ ] **Step 2: Blocking eval on the same set (DB-gated)** — `CAIRN_TEST_PG=… uv run --extra pipeline python -m cairn_matcher.eval "$SP/synth.json" --max-block-size 100` on the branch; record the blocking recall and pair-count lines (the cost of `name+monthday`).

- [ ] **Step 3: Note the false-proposal estimate.** Re-run the session's volume script (`est_near.py` in the scratchpad from the 2026-10-11 brainstorm, if still present) or state the brainstorm's figures (spec table). Record in the ADR that the ABS-modelled dataset re-measure is owed (Task 7 files it).

---

### Task 7: the ADR, the spec, the handover

**Files:**
- Create: `docs/spec/decisions/0079-a-dob-one-keying-error-away-is-not-a-contradiction.md`
- Modify: `docs/spec/decisions/README.md` (index row after 0078), `mkdocs.yml` (nav entry after ADR-0078's line), `docs/spec/index.md` (`**Spec version:**` +0.01 from whatever #749 left — expected 0.80 → 0.81), `docs/spec/identity.md` (one sentence in §5.2's "Locale-pluggable comparators" bullet or §5.13), `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Write the ADR** — follow ADR-0077's header block exactly (Status Accepted · Date · Spec version at acceptance · Issues · Relates to: ADR-0014, ADR-0078 · Design: the spec path). Sections: **Context** (the Mary Smith failure and the score table, from the spec); **Decision** (the six spec decisions, one numbered line each, with the four pattern strings verbatim and the 0.0 weight); **Consequences** (the eval numbers from Task 6; the one-time re-scan on upgrade = the backfill; married-*and*-typo still not shown, by decision; the ABS re-measure owed); **Rejected** (the spec's four). Fact-check every sentence against the code before committing (memory: ADR-0077 passed two reviews with a false sentence).

- [ ] **Step 2: Index, nav, version, spec sentence.** README row in the table's format (`| [0079](0079-….md) | **A DOB one keying error away is not a contradiction**: … | Accepted | <date> |`); `mkdocs.yml` line `      - ADR-0079 · A DOB one keying error away is not a contradiction: spec/decisions/0079-a-dob-one-keying-error-away-is-not-a-contradiction.md`; spec version bump; in `identity.md`, add to the §5.2 "Locale-pluggable comparators" bullet: "The default DOB comparator grades two full dates one keying error apart (one digit, adjacent digits swapped, day and month swapped, years one apart) as a near miss — weight 0, never auto-linked ([ADR-0079](decisions/0079-a-dob-one-keying-error-away-is-not-a-contradiction.md))."

- [ ] **Step 3: Build the docs** — `uv run --with-requirements docs/requirements.txt -- mkdocs build` → no warnings beyond the Material banner.

- [ ] **Step 4: File the follow-ups** (house rule 5), each `Refs` the spec: (a) the tray/banner sentence for `dob_near_miss` + its RUNBOOK §12 timed walk (the §1.2 measurement this slice owes); (b) re-measure the false-proposal rate on the ABS-modelled population dataset. Run `scripts/check_closing_keywords.py` over each body.

- [ ] **Step 5: HANDOVER and ROADMAP** — record the slice as built, the eval numbers, the two follow-up issues, and the trap learned (Task 4's fixture: a new blocking key can generate pairs an older test asserts absent — fix the fixture, never the assertion).

- [ ] **Step 6: Commit, push, mark PR #758 ready for review**

```bash
git add docs/
git commit -m "docs(ADR-0079): a DOB one keying error away is not a contradiction"
git -c credential.helper= -c 'credential.helper=!gh auth git-credential' push
gh pr ready 758
```

Before `gh pr ready`, the full gate must be green **after the final edit**: `cd matcher && uv run ruff check src tests && CAIRN_TEST_PG=… uv run --extra pipeline pytest -q`, and the mkdocs build. No Rust changed, so the cargo gates are not needed; CI runs them regardless.
