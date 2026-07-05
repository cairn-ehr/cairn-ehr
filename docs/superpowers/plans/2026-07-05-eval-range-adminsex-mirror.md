# B3 Eval Mirror (range-DOB emission + administrative-sex) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the B3 eval harness carry the field set the shipped matcher scores (composite sex) and blocks on (anchored dob-range passes), so weight-learning trains on the real field set — per `docs/superpowers/specs/2026-07-05-eval-range-adminsex-mirror-design.md`.

**Architecture:** Four additive parts, all in `matcher/`: (1) `DatasetRecord.administrative_sex` plumbed through the real adapter; (2) a pure `_birth_window` mirror of `_RANGE_GROUPS_SQL`'s CTE + a range branch in `shares_blocking_key` (and a fix to its over-claiming exact-DOB branch); (3) a `corrupt_dob_estimate` generator operator emitting year-range DOBs + observed administrative sex; (4) drift-canary fragments + DB-gated recoverability proof.

**Tech Stack:** Python 3.12, uv (never venv/pip), pytest; DB-gated tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + cairn_pgx) and the `pipeline` extra (psycopg).

## Global Constraints

- Advisory Python only: no `db/` floor change, no SCHEMA bump, no ADR/spec edit, no changes to `pipeline/db.py`, `comparators.py`, `orchestrator.py`, or `scoring.py` (production is the fixed mirror target).
- AGPL-3.0; zero new runtime dependencies (`re` is stdlib).
- House rules: TDD red-first; junior-legible inline docs (match the module's existing comment style — *why*, not *what*); files stay under 500 lines; pure functions.
- Pure suite: `cd matcher && uv run pytest` (no DB). DB-gated: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest`. Lint: `cd matcher && uv run ruff check .`
- All commits on branch `claude/eval-range-adminsex-mirror`.

---

### Task 1: `DatasetRecord.administrative_sex` through the real adapter

**Files:**
- Modify: `matcher/src/cairn_matcher/eval/dataset.py` (DatasetRecord ~line 28–43, `_record_from` ~line 82–99, `record_to_candidate` ~line 132–162)
- Test: `matcher/tests/test_eval_dataset.py` (append)

**Interfaces:**
- Produces: `DatasetRecord.administrative_sex: Mapping | None` (shape `{"value": str, "provenance_rank": int}`), loaded from the dataset JSON key `"administrative_sex"`; `record_to_candidate` maps it to `CandidateRecord.administrative_sex` via the existing `candidate_from_rows(..., admin_sex_row=...)` parameter. Tasks 3 and 5 rely on the JSON key name `"administrative_sex"`.

- [ ] **Step 1: Write the failing tests** — append to `matcher/tests/test_eval_dataset.py`:

```python
def test_administrative_sex_loads_and_reaches_the_candidate():
    """The composite-sex fallback rides the REAL adapter: an admin-sex-only record
    must land on CandidateRecord.administrative_sex (slice D's field), not sex_at_birth."""
    ds = load_dataset({
        "entities": [{"entity_id": "e1", "records": [
            {"record_id": "r1",
             "administrative_sex": {"value": "male", "provenance_rank": 30}},
        ]}],
    })
    rec = ds.entities[0].records[0]
    assert rec.administrative_sex == {"value": "male", "provenance_rank": 30}
    cand = record_to_candidate(rec)
    assert cand.administrative_sex is not None
    assert cand.administrative_sex.value == "male"
    assert cand.administrative_sex.provenance_rank == 30
    assert cand.sex_at_birth is None


def test_admin_sex_absent_stays_none():
    ds = load_dataset({
        "entities": [{"entity_id": "e1", "records": [{"record_id": "r1"}]}],
    })
    assert ds.entities[0].records[0].administrative_sex is None
    assert record_to_candidate(ds.entities[0].records[0]).administrative_sex is None


def test_sab_vs_admin_pair_grades_sex_via_the_composite_fallback():
    """One chart carries only sex-at-birth, the other only administrative-sex — the
    §5.4 slice-D union fallback must produce a graded 'sex' comparison (EXACT here),
    proving the eval path exercises the composite the shipped scorer uses."""
    from cairn_matcher.agreement import AgreementLevel
    from cairn_matcher.orchestrator import DEFAULT_CONFIG, field_comparisons

    ds = load_dataset({
        "entities": [{"entity_id": "e1", "records": [
            {"record_id": "a", "sex_at_birth": {"value": "male", "provenance_rank": 40}},
            {"record_id": "b",
             "administrative_sex": {"value": "male", "provenance_rank": 30}},
        ]}],
    })
    a, b = (record_to_candidate(r) for r in ds.entities[0].records)
    by_field = {c.field: c for c in field_comparisons(a, b, DEFAULT_CONFIG)}
    assert by_field["sex"].level is AgreementLevel.EXACT
```

Also extend the imports at the top of the test file if `record_to_candidate` is not already imported there (check the existing import block; it imports from `cairn_matcher.eval.dataset`).

- [ ] **Step 2: Run to verify failure**

Run: `cd matcher && uv run pytest tests/test_eval_dataset.py -v`
Expected: the three new tests FAIL — `DatasetRecord` has no `administrative_sex` attribute.

- [ ] **Step 3: Implement** — in `matcher/src/cairn_matcher/eval/dataset.py`:

In `DatasetRecord`, add the field and extend the docstring:

```python
@dataclass(frozen=True)
class DatasetRecord:
    """One patient record as projection-shaped field dicts. Every field is optional
    except record_id; absence is a safe, gradeable absence (principle 4), not an error.

    dob: {"value": ISO str, "precision": "year"|"month"|"day", "provenance_rank": int}
         — or a §5.4 estimated-age window: {"value": "<yyyy>/<yyyy>",
         "precision": "year-range", "provenance_rank": int}
    sex_at_birth: {"value": str, "provenance_rank": int}
    administrative_sex: {"value": str, "provenance_rank": int} — the apparent/phenotypic
         facet a clinician-observed sex lands on (slice D's composite-sex fallback input)
    names: tuple of {"value": display str, "provenance_rank": int}
    identifiers: tuple of {"system": str, "match_key": str, "value": str}
    """

    record_id: str
    dob: Mapping | None = None
    sex_at_birth: Mapping | None = None
    administrative_sex: Mapping | None = None
    names: tuple[Mapping, ...] = ()
    identifiers: tuple[Mapping, ...] = ()
```

In `_record_from`, pass it through (add one kwarg to the return):

```python
    return DatasetRecord(
        record_id=record_id,
        dob=obj.get("dob"),
        sex_at_birth=obj.get("sex_at_birth"),
        administrative_sex=obj.get("administrative_sex"),
        names=names,
        identifiers=identifiers,
    )
```

In `record_to_candidate`, build the row and hand it to the real adapter (after the
`sex_row` block; extend the final call):

```python
    admin_sex_row = None
    if rec.administrative_sex is not None:
        admin_sex_row = {
            "value": rec.administrative_sex.get("value"),
            "provenance_rank": rec.administrative_sex.get("provenance_rank", 0),
        }
```

```python
    return candidate_from_rows(
        dob_row=dob_row,
        sex_row=sex_row,
        name_rows=name_rows,
        identifier_rows=identifier_rows,
        admin_sex_row=admin_sex_row,
    )
```

- [ ] **Step 4: Run the pure suite**

Run: `cd matcher && uv run pytest`
Expected: all PASS (227 pre-existing + 3 new).

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/eval/dataset.py matcher/tests/test_eval_dataset.py
git commit -m "feat(matcher): eval DatasetRecord carries administrative_sex through the real adapter"
```

---

### Task 2: Range-aware `shares_blocking_key` (+ the exact-branch over-claim fix)

**Files:**
- Modify: `matcher/src/cairn_matcher/eval/generator.py` (`shares_blocking_key` ~line 60–80; add `_birth_window` above it; module imports)
- Test: create `matcher/tests/test_eval_generator_range.py`

**Interfaces:**
- Consumes: nothing new.
- Produces: `_birth_window(record: Mapping) -> tuple[int, int, bool] | None` (`(y_min, y_max, is_range)`); `shares_blocking_key(a, b)` now returns True for anchored range-overlap pairs and no longer over-claims on year-range values in its exact-DOB branch. Task 3's `_repair` behaviour and Task 5's volume proof lean on both.

- [ ] **Step 1: Write the failing tests** — create `matcher/tests/test_eval_generator_range.py`:

```python
"""Pure tests for the range-aware half of the generator's blocking-key mirror.

_birth_window / the shares_blocking_key range branch mirror _RANGE_GROUPS_SQL's
birth_window CTE + anchored window_overlap join (pipeline/db.py). The danger
direction is OVER-claiming: a pair the mirror calls recoverable but the SQL never
generates would make _repair stand down and silently break the volume set's
recoverable-by-construction guarantee. These tests pin the safe semantics.
"""

from cairn_matcher.eval.generator import _birth_window, _repair, shares_blocking_key


def _rec(dob=None, names=(), identifiers=()):
    out = {"record_id": "r"}
    if dob is not None:
        out["dob"] = dict(dob)
    if names:
        out["names"] = [{"value": n} for n in names]
    if identifiers:
        out["identifiers"] = [
            {"system": s, "match_key": k, "value": k} for s, k in identifiers
        ]
    return out


# --- _birth_window: the birth_window CTE mirror -------------------------------

def test_window_parses_a_wellformed_year_range():
    assert _birth_window(_rec(dob={"value": "1980/1990", "precision": "year-range"})) \
        == (1980, 1990, True)


def test_window_excludes_malformed_and_inverted_ranges():
    # Mirrors the SQL regex + min<=max guards: excluded, never a guessed window.
    for bad in ("about-forty", "1980/199", "1990/1980", "1980-1990", "1980/1990/2000"):
        assert _birth_window(_rec(dob={"value": bad, "precision": "year-range"})) is None


def test_window_point_dob_uses_first_four_digit_run():
    # substring(value FROM '[0-9]{4}') takes the FIRST 4-digit run — "12/05/1990"
    # has runs 12, 05, 1990; the first 4-digit one is 1990.
    assert _birth_window(_rec(dob={"value": "1985-05-12", "precision": "day"})) \
        == (1985, 1985, False)
    assert _birth_window(_rec(dob={"value": "12/05/1990", "precision": "day"})) \
        == (1990, 1990, False)


def test_window_absent_without_a_four_digit_run_or_dob():
    assert _birth_window(_rec(dob={"value": "12/05/90", "precision": "day"})) is None
    assert _birth_window(_rec()) is None


# --- shares_blocking_key: the anchored range branch ----------------------------

def test_range_overlapping_point_shares_a_key():
    a = _rec(dob={"value": "1980/1990", "precision": "year-range"}, names=("Alex",))
    b = _rec(dob={"value": "1985-05-12", "precision": "day"}, names=("Zed",))
    assert shares_blocking_key(a, b)


def test_range_disjoint_point_shares_nothing():
    a = _rec(dob={"value": "1980/1990", "precision": "year-range"}, names=("Alex",))
    b = _rec(dob={"value": "1995-05-12", "precision": "day"}, names=("Zed",))
    assert not shares_blocking_key(a, b)


def test_two_overlapping_ranges_share_a_key():
    # Two John Does, two sites — the only key that pair can ever share (db.py comment).
    a = _rec(dob={"value": "1980/1990", "precision": "year-range"}, names=("Alex",))
    b = _rec(dob={"value": "1988/1995", "precision": "year-range"}, names=("Zed",))
    assert shares_blocking_key(a, b)


def test_point_point_same_year_is_not_a_range_key():
    # window_overlap requires a.is_range: two point DOBs in the same year never key
    # via the range passes (they need exact-DOB/name/identifier instead).
    a = _rec(dob={"value": "1985-01-01", "precision": "day"}, names=("Alex",))
    b = _rec(dob={"value": "1985-12-28", "precision": "day"}, names=("Zed",))
    assert not shares_blocking_key(a, b)


def test_identical_yearrange_values_do_not_fake_an_exact_dob_key():
    # THE over-claim fix: _GROUPS_SQL's exact-'dob' arm excludes year-range rows
    # (IS DISTINCT FROM 'year-range'). Two identical but MALFORMED range values parse
    # to no window, so neither the range branch nor the exact branch may claim them.
    a = _rec(dob={"value": "199x/1999", "precision": "year-range"}, names=("Alex",))
    b = _rec(dob={"value": "199x/1999", "precision": "year-range"}, names=("Zed",))
    assert not shares_blocking_key(a, b)


def test_repair_stands_down_when_the_window_carries_the_pair():
    # With the range branch mirrored, _repair must NOT append the seed's name to an
    # estimated-age clone the dob-range pass already recovers.
    seed = _rec(dob={"value": "1985-05-12", "precision": "day"}, names=("Alex Nguyen",))
    clone = _rec(dob={"value": "1983/1988", "precision": "year-range"}, names=("Zed Q",))
    repaired = _repair(seed, clone)
    assert repaired is clone  # _repair returns the clone UNTOUCHED when a key exists
    assert [n["value"] for n in repaired.get("names", [])] == ["Zed Q"]
```

- [ ] **Step 2: Run to verify failure**

Run: `cd matcher && uv run pytest tests/test_eval_generator_range.py -v`
Expected: FAIL — `_birth_window` does not exist (ImportError).

- [ ] **Step 3: Implement** — in `matcher/src/cairn_matcher/eval/generator.py`:

Add `import re` to the imports (after `import random`). Above `shares_blocking_key`, add:

```python
# Mirrors of _RANGE_GROUPS_SQL's birth_window guards (pipeline/db.py): a range value
# must be exactly '<yyyy>/<yyyy>' with min <= max; a point value contributes its FIRST
# 4-digit run. Kept as module constants so the two branches below can't drift apart.
_YEAR_RANGE_RE = re.compile(r"^([0-9]{4})/([0-9]{4})$")
_FIRST_YEAR_RE = re.compile(r"[0-9]{4}")


def _birth_window(record: Mapping):
    """(y_min, y_max, is_range) for one record's dob, or None — the birth_window CTE.

    Mirrors _RANGE_GROUPS_SQL exactly, in the safe direction: a malformed or inverted
    year-range value yields NO window (the SQL excludes the row), never a guess; a
    point value needs a 4-digit run (the SQL's `value ~ '[0-9]{4}'`) and contributes
    its first run as a degenerate [y, y] window. A year-range row can never enter the
    point branch (the SQL's IS DISTINCT FROM guard), so a range can't double-enter as
    a false point [min, min].
    """
    dob = record.get("dob")
    if not dob or not isinstance(dob.get("value"), str):
        return None
    value = dob["value"]
    if dob.get("precision") == "year-range":
        m = _YEAR_RANGE_RE.match(value)
        if not m:
            return None
        lo, hi = int(m.group(1)), int(m.group(2))
        if lo > hi:
            return None
        return (lo, hi, True)
    m = _FIRST_YEAR_RE.search(value)
    if m is None:
        return None
    year = int(m.group(0))
    return (year, year, False)
```

Replace `shares_blocking_key` (docstring + body — the deferral comment it carried is
now discharged):

```python
def shares_blocking_key(a: Mapping, b: Mapping) -> bool:
    """True iff records a and b would co-occur in >=1 blocking pass.

    The symmetric keys (pipeline/db.py _GROUPS_SQL): shared non-unknown identifier,
    equal exact-DOB value (excluding year-range rows, mirroring the SQL's
    IS DISTINCT FROM 'year-range' — two identical range strings must NOT fake an
    exact key the SQL never groups), or a shared name token. The 'name+year' pass is
    subsumed by the name-token check (it requires a shared token).

    The ANCHORED range passes (_RANGE_GROUPS_SQL): windows overlap AND at least one
    side is a real year-range (window_overlap requires a.is_range — two point DOBs
    merely sharing a year are never a range key). 'dob-range+sex' is a subset of
    'dob-range''s pair set (same overlap join, intersected with a shared sex), so
    recoverability needs only the plain overlap branch; like every branch here, the
    block-size cap is deliberately not modelled (evaluate_blocking reports skips).
    """
    if _identifier_keys(a) & _identifier_keys(b):
        return True
    da, db_ = a.get("dob"), b.get("dob")
    if (
        da
        and db_
        and da.get("precision") != "year-range"
        and db_.get("precision") != "year-range"
        and da.get("value") is not None
        and da.get("value") == db_.get("value")
    ):
        return True
    wa, wb = _birth_window(a), _birth_window(b)
    if wa and wb and (wa[2] or wb[2]) and wb[0] <= wa[1] and wa[0] <= wb[1]:
        return True
    return bool(name_tokens(a) & name_tokens(b))
```

- [ ] **Step 4: Run the pure suite**

Run: `cd matcher && uv run pytest`
Expected: all PASS (the pre-existing `test_eval_generator.py` shares-key tests must
still pass — they use point/absent dobs only).

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/eval/generator.py matcher/tests/test_eval_generator_range.py
git commit -m "feat(matcher): mirror the anchored dob-range passes in shares_blocking_key; fix exact-branch over-claim on year-range values"
```

---

### Task 3: The estimated-age generator operator

**Files:**
- Modify: `matcher/src/cairn_matcher/eval/generator.py` (new operator after `corrupt_identifier` ~line 184; `GenSpec` ~line 236; `_OPERATORS` ~line 251; `generate_dataset` docstring ~line 282)
- Test: `matcher/tests/test_eval_generator_range.py` (append)

**Interfaces:**
- Consumes: `_FIRST_YEAR_RE` and `_clone` from Task 2 / existing module.
- Produces: `corrupt_dob_estimate(record, rng)`; `GenSpec.p_dob_estimate: float = 0.15`; generated record dicts may now carry `"administrative_sex"` and a `"year-range"` dob (the JSON key shapes Task 1 loads). Task 5's volume proof forces `p_dob_estimate=0.9`.

- [ ] **Step 1: Write the failing tests** — append to `matcher/tests/test_eval_generator_range.py`:

```python
import random

from cairn_matcher.eval.dataset import load_dataset
from cairn_matcher.eval.generator import (
    GenSpec,
    corrupt_dob_estimate,
    generate_dataset,
)


def test_estimate_replaces_dob_with_a_window_containing_the_year():
    rec = _rec(dob={"value": "1985-05-12", "precision": "day"})
    out = corrupt_dob_estimate(rec, random.Random(0))
    assert out["dob"]["precision"] == "year-range"
    lo, hi = (int(p) for p in out["dob"]["value"].split("/"))
    assert lo <= 1985 <= hi
    assert out["dob"]["provenance_rank"] == 30  # clinician-observed (slice B)


def test_estimate_moves_sex_to_the_observed_facet():
    rec = _rec(dob={"value": "1985-05-12", "precision": "day"})
    rec["sex_at_birth"] = {"value": "female", "provenance_rank": 40}
    out = corrupt_dob_estimate(rec, random.Random(0))
    assert "sex_at_birth" not in out
    assert out["administrative_sex"] == {"value": "female", "provenance_rank": 30}


def test_estimate_draws_a_sex_when_the_seed_has_none():
    rec = _rec(dob={"value": "1985-05-12", "precision": "day"})
    out = corrupt_dob_estimate(rec, random.Random(0))
    assert out["administrative_sex"]["value"] in ("male", "female")


def test_estimate_is_a_noop_without_a_four_digit_run():
    rec = _rec(dob={"value": "12/05/90", "precision": "day"})
    assert corrupt_dob_estimate(rec, random.Random(0))["dob"] == rec["dob"]
    no_dob = _rec(names=("Alex",))
    assert "dob" not in corrupt_dob_estimate(no_dob, random.Random(0))


def test_estimate_never_mutates_its_input():
    rec = _rec(dob={"value": "1985-05-12", "precision": "day"})
    rec["sex_at_birth"] = {"value": "male", "provenance_rank": 40}
    frozen = {"dob": dict(rec["dob"]), "sex_at_birth": dict(rec["sex_at_birth"])}
    corrupt_dob_estimate(rec, random.Random(0))
    assert rec["dob"] == frozen["dob"] and rec["sex_at_birth"] == frozen["sex_at_birth"]


def test_estimate_heavy_dataset_round_trips_and_carries_the_new_fields():
    ds_dict = generate_dataset(GenSpec(seed=3, n_entities=40, p_dob_estimate=1.0))
    ds = load_dataset(ds_dict)  # round-trips the real loader (Task 1's plumbing)
    clones = [r for e in ds.entities for r in e.records if r.record_id.endswith("-dup")]
    ranged = [r for r in clones if (r.dob or {}).get("precision") == "year-range"]
    assert ranged, "p_dob_estimate=1.0 must produce year-range clones"
    assert all(r.administrative_sex is not None for r in ranged)
    assert all(r.sex_at_birth is None for r in ranged)
```

- [ ] **Step 2: Run to verify failure**

Run: `cd matcher && uv run pytest tests/test_eval_generator_range.py -v`
Expected: FAIL — `corrupt_dob_estimate` does not exist (ImportError).

- [ ] **Step 3: Implement** — in `matcher/src/cairn_matcher/eval/generator.py`:

After `corrupt_identifier`, add:

```python
def corrupt_dob_estimate(record, rng):
    """Rewrite the clone as a §5.4 estimated-age registration of the same person.

    The dob becomes an inclusive birth-year window CONTAINING the current value's
    first 4-digit run (an honest interval, never a false-precise midpoint —
    principle 4; tol 2..5 gives the 5–11-year widths slice B's evidence produces),
    at provenance 30 (clinician-observed). Sex moves to the OBSERVED facet: a
    clinician observes apparent sex but cannot know the birth fact (slice B), so
    sex_at_birth is dropped and administrative_sex carries the seed's value when
    present (a correct observation) or a random draw when the seed recorded none.
    Runs LAST in _OPERATORS: an estimated-age record supersedes format/typo dob
    corruption wholesale; a typo'd year windows around the typo (honest corruption —
    the window may or may not still overlap the seed's). No-op without a 4-digit
    run (safe degrade, like every operator).
    """
    out = _clone(record)
    dob = out.get("dob")
    if not dob or not isinstance(dob.get("value"), str):
        return out
    m = _FIRST_YEAR_RE.search(dob["value"])
    if m is None:
        return out
    year = int(m.group(0))
    tol = rng.randint(2, 5)
    out["dob"] = {
        "value": f"{year - tol:04d}/{year + tol:04d}",
        "precision": "year-range",
        "provenance_rank": 30,
    }
    sab = out.pop("sex_at_birth", None)
    observed = sab["value"] if sab else rng.choice(("male", "female"))
    out["administrative_sex"] = {"value": observed, "provenance_rank": 30}
    return out
```

In `GenSpec`, add the knob (after `p_identifier`) and a determinism note to the
docstring:

```python
    p_identifier: float = 0.5
    p_dob_estimate: float = 0.15
```

Docstring addition (append to the existing GenSpec docstring body):

```python
    Adding an operator changes RNG consumption, so a given seed's output differs
    across versions of this module: "deterministic given a seed" is a
    reproducibility contract within one version, not a cross-version stability one.
```

In `_OPERATORS`, append the estimate operator LAST:

```python
_OPERATORS = (
    ("p_dob_format", corrupt_dob_format),
    ("p_dob_typo", corrupt_dob_typo),
    ("p_name", corrupt_name),
    ("p_identifier", corrupt_identifier),
    ("p_dob_estimate", corrupt_dob_estimate),
)
```

- [ ] **Step 4: Run the pure suite**

Run: `cd matcher && uv run pytest`
Expected: all PASS — including the pre-existing determinism and
`test_every_true_pair_shares_a_blocking_key` generator tests (the mirror from Task 2
recognises the windows the new operator emits).

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/eval/generator.py matcher/tests/test_eval_generator_range.py
git commit -m "feat(matcher): generator emits estimated-age clones — year-range dob + observed administrative sex"
```

---

### Task 4: Drift-canary extension to the range SQL

**Files:**
- Modify: `matcher/tests/test_eval_generator_sync.py`

**Interfaces:**
- Consumes: `_GROUPS_SQL`, `_RANGE_GROUPS_SQL` from `cairn_matcher.pipeline.db` (import-only, no connection).
- Produces: nothing runtime — the fast-suite tripwire later tasks (and future SQL edits) rely on.

- [ ] **Step 1: Extend the canary table** — in `matcher/tests/test_eval_generator_sync.py`, import both constants and restructure entries as `(assumption, sql_text, fragment)`:

Replace the import line and `_MIRRORED_PASSES` list + test with:

```python
from cairn_matcher.pipeline.db import _GROUPS_SQL, _RANGE_GROUPS_SQL  # noqa: E402


# Each entry: the recoverability assumption in shares_blocking_key -> the SQL fragment
# that must survive for it to hold, in the statement that owns it. Narrowing/renaming
# any of these breaks the "recoverable by construction" guarantee, so tripping this
# test points straight at the mismatch.
_MIRRORED_PASSES = [
    ("exact-DOB pass (shares_blocking_key dob branch)",
     _GROUPS_SQL, "FROM patient_demographic WHERE field = 'dob'"),
    ("identifier pass excluding 'unknown' (_identifier_keys)",
     _GROUPS_SQL, "FROM patient_identifier WHERE system <> 'unknown'"),
    ("name-token pass: NFC + lower + whitespace split (name_tokens)",
     _GROUPS_SQL, "regexp_split_to_table(lower(normalize(value, NFC)), '\\s+')"),
    # §5.4: the placeholder-use exclusion the name_tokens mirror
    # (generator._is_placeholder_name) depends on.
    ("placeholder-use exclusion in name_tokens (§5.4)",
     _GROUPS_SQL, "use_key <> ALL(%s)"),
    # The exact-'dob' arm must keep EXCLUDING year-range rows: shares_blocking_key's
    # exact branch mirrors this exclusion, and without it two identical range strings
    # would be grouped by the SQL but not by the mirror (under-claim, safe) — while
    # DROPPING the guard from the mirror side would over-claim. Pin the SQL side.
    ("exact-dob arm excludes year-range rows (shares_blocking_key exact branch)",
     _GROUPS_SQL, "IS DISTINCT FROM 'year-range'"),
    # The anchored range mirror (_birth_window + the overlap branch). Any of these
    # fragments disappearing means the range passes changed shape under the mirror.
    ("range rows keyed on precision 'year-range' (_birth_window range branch)",
     _RANGE_GROUPS_SQL, "facets ->> 'precision' = 'year-range'"),
    ("range min extracted as ^([0-9]{4})/ (_YEAR_RANGE_RE)",
     _RANGE_GROUPS_SQL, "substring(value FROM '^([0-9]{4})/')"),
    ("window overlap join (shares_blocking_key range branch)",
     _RANGE_GROUPS_SQL, "AND m.y_min <= a.y_max"),
    ("anchored on range charts only (point-point never keys)",
     _RANGE_GROUPS_SQL, "WHERE a.is_range"),
    ("blocking_sex unions both sex facets (dob-range+sex subset claim)",
     _RANGE_GROUPS_SQL, "field IN ('sex-at-birth', 'administrative-sex')"),
]


@pytest.mark.parametrize("assumption, sql_text, fragment", _MIRRORED_PASSES)
def test_shares_blocking_key_mirrors_the_blocking_sql(assumption, sql_text, fragment):
    assert fragment in sql_text, (
        f"the blocking SQL no longer contains the pass fragment that "
        f"shares_blocking_key mirrors: {assumption}. Update "
        f"generator.shares_blocking_key/_birth_window to match — otherwise the "
        f"synthetic generator's recoverability guarantee is silently false."
    )
```

Also update the module docstring's first paragraph reference from "`_GROUPS_SQL`" to
"`_GROUPS_SQL` / `_RANGE_GROUPS_SQL`" (one-line edit, keep the rest).

- [ ] **Step 2: Run the canary**

Run: `cd matcher && uv run --extra pipeline pytest tests/test_eval_generator_sync.py -v`
Expected: 10 PASS (fragments verified against today's SQL; a wrong fragment fails
loudly — fix the fragment, not the SQL).

- [ ] **Step 3: Run the pure suite (skip path)**

Run: `cd matcher && uv run pytest tests/test_eval_generator_sync.py -v`
Expected: PASS or SKIP depending on whether psycopg is present in the ambient env —
must not ERROR (the importorskip guards both constants).

- [ ] **Step 4: Commit**

```bash
git add matcher/tests/test_eval_generator_sync.py
git commit -m "test(matcher): drift canary pins the range-pass fragments shares_blocking_key now mirrors"
```

---

### Task 5: DB seeding + the end-to-end recoverability proof

**Files:**
- Modify: `matcher/src/cairn_matcher/eval/blocking_eval.py` (`seed_dataset` ~line 49–100)
- Test: `matcher/tests/test_eval_blocking.py` (append), `matcher/tests/test_eval_generator_volume.py` (append)

**Interfaces:**
- Consumes: `DatasetRecord.administrative_sex` (Task 1), the estimate operator + knob (Task 3), the range mirror (Task 2 — `_repair` standing down is what makes the volume proof non-vacuous).
- Produces: `seed_dataset` writes `administrative-sex` rows, so `evaluate_blocking` exercises the real `blocking_sex` CTE.

- [ ] **Step 1: Write the failing DB-gated tests**

Append to `matcher/tests/test_eval_blocking.py` (it already has `pg_conn`/imports —
match its existing import style):

```python
def test_seeded_admin_sex_feeds_the_range_sex_rescue(pg_conn):
    """A year-range Doe with only administrative-sex must group with a point-DOB
    resident sharing that sex under the dob-range+sex pass ALONE — proving
    seed_dataset's administrative-sex rows are visible to the blocking_sex CTE."""
    from cairn_matcher.pipeline.db import generate_candidate_pairs

    ds = load_dataset({
        "entities": [
            {"entity_id": "doe", "records": [
                {"record_id": "doe-1",
                 "dob": {"value": "1980/1990", "precision": "year-range",
                         "provenance_rank": 30},
                 "administrative_sex": {"value": "male", "provenance_rank": 30}},
            ]},
            {"entity_id": "resident", "records": [
                {"record_id": "resident-1",
                 "dob": {"value": "1985-05-12", "precision": "day",
                         "provenance_rank": 40},
                 "sex_at_birth": {"value": "male", "provenance_rank": 40},
                 "names": [{"value": "Alex Nguyen", "provenance_rank": 30}]},
            ]},
        ],
    })
    reverse = seed_dataset(pg_conn, ds)
    # Large cap: on a shared cairn_test DB, leaked resident charts (issue #84) can
    # sit inside doe-1's window and balloon the anchored block past the default cap,
    # which would skip the block and flake this test.
    pairs, _skipped = generate_candidate_pairs(
        pg_conn, max_block_size=10_000, enabled_passes={"dob-range+sex"}
    )
    pg_conn.rollback()
    labels = {
        canonical_label_pair(reverse[lo], reverse[hi])
        for lo, hi in pairs
        if lo in reverse and hi in reverse
    }
    assert canonical_label_pair("doe-1", "resident-1") in labels
```

(If `load_dataset` / `canonical_label_pair` are not already imported in that file's
header, add them to the existing `cairn_matcher.eval.dataset` import.)

Append to `matcher/tests/test_eval_generator_volume.py`:

```python
def test_estimate_heavy_volume_set_is_fully_recoverable(pg_conn):
    """Names corrupted AND dobs replaced by estimated-age windows: with _repair
    standing down on window-overlap pairs (the Task-2 mirror), only the REAL
    dob-range pass can carry those pairs — the end-to-end proof the mirror never
    over-claims what the SQL recovers."""
    spec = GenSpec(seed=2, n_entities=150, p_dob_estimate=0.9, p_name=0.9)
    ds_dict = generate_dataset(spec)
    ds = load_dataset(ds_dict)
    ranged = [
        r for e in ds.entities for r in e.records
        if (r.dob or {}).get("precision") == "year-range"
    ]
    assert len(ranged) > 100  # non-vacuous: the knob really produced range clones
    metrics = evaluate_blocking(pg_conn, ds, max_block_size=10_000)
    assert metrics.pair_completeness == 1.0
    assert metrics.dropped_true_matches == ()
```

- [ ] **Step 2: Run to verify failure**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_eval_blocking.py tests/test_eval_generator_volume.py -v`
Expected: the two new tests FAIL — `seed_dataset` never writes administrative-sex
rows, so the +sex rescue can't fire for doe-1 (and the volume set may repair via
names, but the seeding gap plus any mirror error surfaces here).

Note: the volume test may PASS red if Tasks 2–3 are complete and seeding of
admin-sex isn't load-bearing for it — the REQUIRED red signal is the
`test_seeded_admin_sex_feeds_the_range_sex_rescue` failure.

- [ ] **Step 3: Implement** — in `matcher/src/cairn_matcher/eval/blocking_eval.py`,
inside `seed_dataset`'s loop after the `sex_at_birth` INSERT block:

```python
            if rec.administrative_sex is not None:
                cur.execute(
                    "INSERT INTO patient_demographic (patient_id, field, value, facets, "
                    "provenance, provenance_rank, asserted_hlc_wall, asserted_hlc_count, "
                    "asserted_origin) VALUES (%s,'administrative-sex',%s,NULL,'seed',%s,0,0,'seed')",
                    (pid, rec.administrative_sex.get("value"),
                     rec.administrative_sex.get("provenance_rank", 0)),
                )
```

Also extend `seed_dataset`'s docstring first line context: it "mirrors
tests/conftest.seed_patient but reads the dataset's dict fields" — no change needed
beyond the insert (the dob INSERT already carries `value` + `facets.precision`
verbatim, which is all `_RANGE_GROUPS_SQL` reads — say so in a one-line comment
above the new block).

- [ ] **Step 4: Run the full DB-gated + pure suites**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest`
Expected: all PASS (298 pre-existing + new).
Run: `cd matcher && uv run pytest`
Expected: all PASS.
Run: `cd matcher && uv run ruff check .`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/eval/blocking_eval.py matcher/tests/test_eval_blocking.py matcher/tests/test_eval_generator_volume.py
git commit -m "feat(matcher): seed administrative-sex rows; DB-gated proof the range mirror never over-claims"
```

---

## Self-review notes

- Spec coverage: §3.1→Task 1+5, §3.2→Task 3, §3.3→Task 2, §3.4→Tasks 4+5; §5 honest limits are recorded in docstrings/comments written by Tasks 2/3.
- The pure-suite count (227) grows across tasks; the exact totals in "Expected" lines are indicative, the requirement is zero failures.
- `record_to_candidate` already receives `admin_sex_row` support from slice D (`candidate_from_rows` signature, adapter.py:167-174) — no adapter change anywhere in this plan.
