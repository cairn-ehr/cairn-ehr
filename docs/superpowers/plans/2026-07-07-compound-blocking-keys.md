# Compound Blocking Keys Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add two additive, symmetric compound blocking passes — `dob+first-initial` (genuinely new recall) and `name+sex` (oversized-name-block rescue) — to the advisory matcher's candidate generation.

**Architecture:** Register the two new pass names in the pure `pipeline/blocking.py` registry; extract the three shared CTEs (`name_tokens`, `birth_year`, `blocking_sex`) in `pipeline/db.py` into composable SQL-fragment constants and compose both statements from them, adding two `UNION ALL` arms to `_GROUPS_SQL`; mirror `dob+first-initial` in the pure eval recoverability predicate (`eval/generator.py`). All changes are advisory-tier — no `db/` migration, no floor, no SCHEMA/event/ADR/spec change.

**Tech Stack:** Python 3.12, uv (never venv/pip), psycopg (optional `pipeline` extra), PostgreSQL ≥ 18 + cairn_pgx for DB-gated tests, pytest.

## Global Constraints

- **License:** AGPL-3.0; every dependency AGPL-3.0-compatible. No new dependency is added by this plan.
- **Tooling:** Python env/package work uses **uv**, never venv/pip. Pure suite: `cd matcher && uv run pytest`. DB-gated: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest`.
- **Additive-only invariant:** every pass output is UNIONed + deduped by canonical uuid pair; a new pass may only RAISE recall, never suppress. Never auto-links (banding + `db/016` veto floor own that).
- **TDD:** failing test first, then minimal code. All tests pass before commit.
- **Reviewer-legibility:** every non-trivial change carries junior-readable comments explaining *why*, not just *what* (house rule 3, doubly load-bearing here — the module already fought a hand-mirrored-sex-literal drift bug).
- **No new normalization rules:** both passes reuse the EXISTING `name_tokens` / `birth_year` / `blocking_sex` CTE logic verbatim.
- **`ALL_PASSES` (post-slice), execution order:** `("identifier", "dob", "name", "name+year", "dob+first-initial", "name+sex", "dob-range", "dob-range+sex")`.
- **Bind-parameter order for `_GROUPS_SQL` (post-slice):** `(_PLACEHOLDER_USES_PARAM, VALUE_SENTINELS_PARAM)` — placeholder-uses first (appears first, in `name_tokens`), value-sentinels second (in `blocking_sex`).

---

### Task 1: Register the two new pass names (pure registry)

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/blocking.py:24` (the `ALL_PASSES` tuple + its docstring comment)
- Test: `matcher/tests/test_blocking_passes.py`

**Interfaces:**
- Consumes: nothing new.
- Produces: `ALL_PASSES` (8-tuple, order above); `SYMMETRIC_PASSES` now contains `"dob+first-initial"` and `"name+sex"` automatically (it is derived `_ALL_PASSES_SET - ANCHORED_PASSES`); `ANCHORED_PASSES` unchanged.

- [ ] **Step 1: Update the failing test**

In `matcher/tests/test_blocking_passes.py`, replace `test_all_passes_is_the_six_known_names` (lines 25–28) with:

```python
def test_all_passes_is_the_eight_known_names():
    assert ALL_PASSES == (
        "identifier", "dob", "name", "name+year",
        "dob+first-initial", "name+sex", "dob-range", "dob-range+sex",
    )


def test_new_compound_passes_are_symmetric():
    # Both new compound passes pair every within-group member (C(s,2)); neither is
    # anchored. dropped_pair_estimate and the statement-level toggle skip both branch
    # on this membership, so a misfiled pass would corrupt them.
    assert {"dob+first-initial", "name+sex"} <= SYMMETRIC_PASSES
    assert {"dob+first-initial", "name+sex"} & ANCHORED_PASSES == frozenset()
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd matcher && uv run pytest tests/test_blocking_passes.py::test_all_passes_is_the_eight_known_names tests/test_blocking_passes.py::test_new_compound_passes_are_symmetric -v`
Expected: FAIL — `test_all_passes_is_the_eight_known_names` fails on the tuple mismatch (current `ALL_PASSES` has six names).

- [ ] **Step 3: Add the two names to the registry**

In `matcher/src/cairn_matcher/pipeline/blocking.py`, replace the `ALL_PASSES` definition and its comment (lines 21–24):

```python
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
```

- [ ] **Step 4: Run the pure blocking suite to verify pass**

Run: `cd matcher && uv run pytest tests/test_blocking_passes.py -v`
Expected: PASS — all tests green, including the shape-set partition test (which now confirms both new names landed in `SYMMETRIC_PASSES`).

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/pipeline/blocking.py matcher/tests/test_blocking_passes.py
git commit -m "feat(matcher): register dob+first-initial and name+sex blocking passes"
```

---

### Task 2: Extract shared CTE fragments in db.py (behavior-preserving refactor)

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/db.py:185-338` (the `_GROUPS_SQL` and `_RANGE_GROUPS_SQL` string literals)
- Test: `matcher/tests/test_candidate_generation.py`, `matcher/tests/test_dob_range_blocking.py` (existing DB-gated suites — no new tests this task; the refactor must keep them green)

**Interfaces:**
- Consumes: `_PLACEHOLDER_USES_PARAM`, `VALUE_SENTINELS_PARAM` (already imported).
- Produces: module-level constants `_NAME_TOKENS_CTE`, `_BIRTH_YEAR_CTE`, `_BLOCKING_SEX_CTE` (SQL-fragment strings, each a single named CTE body WITHOUT the leading `WITH`/`,`); `_GROUPS_SQL` and `_RANGE_GROUPS_SQL` recomposed from them. This task adds NO new arms and changes NO bind params — it only relocates the three CTE bodies so both statements share one definition of each. The new arms are added in Task 3.

**This is a pure refactor: same SQL text, same passes, same bind params. Do NOT add the new arms here — that keeps this task's green/red signal clean (existing tests must stay green unchanged).**

- [ ] **Step 1: Run the existing DB-gated blocking suites to capture the green baseline**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_candidate_generation.py tests/test_dob_range_blocking.py -v`
Expected: PASS (baseline). If `CAIRN_TEST_PG` is unavailable, these SKIP — note that and proceed; Task 3's tests will exercise the composed SQL when a DB is present.

- [ ] **Step 2: Introduce the three CTE-fragment constants**

In `matcher/src/cairn_matcher/pipeline/db.py`, ABOVE the `_GROUPS_SQL` definition (currently line 185), add the three fragments. Copy the CTE bodies VERBATIM from the current `_GROUPS_SQL` (`name_tokens`, `birth_year`) and `_RANGE_GROUPS_SQL` (`blocking_sex`), preserving every comment:

```python
# Shared blocking CTE fragments. Both statements (_GROUPS_SQL, _RANGE_GROUPS_SQL) need
# overlapping CTEs, so each CTE body lives ONCE here and the statements compose from these
# constants. This is not premature abstraction: blocking_sex is a load-bearing, sentinel-bound
# normalization, and the module was bitten once by a hand-mirrored sex literal lagging the
# adapter (see the comment inside _BLOCKING_SEX_CTE). Each constant is a CTE BODY only
# ("name AS ( ... )"); the composing statement supplies the leading WITH and comma joins.

_NAME_TOKENS_CTE = """name_tokens AS (
    -- normalize(value, NFC) so a name recorded decomposed (NFD) on one feed and
    -- precomposed (NFC) on another produces the SAME blocking token — otherwise the two
    -- are different code points and a true duplicate is never even grouped. Mirrors the
    -- adapter's _normalize_token (NFC) on the Python comparison side.
    -- Exclude placeholder-use names (callsigns) from BLOCKING (§5.4). A callsign is a
    -- single whitespace-free token, so this bites only when two callsign STRINGS are
    -- identical (the rare same-suffix collision) — defense-in-depth, not what keeps two
    -- ordinary John Does apart (distinct callsigns are already distinct tokens; the
    -- load-bearing exclusion is the scoring one in load_candidate). The name+year and
    -- dob+first-initial passes read this same CTE, so they inherit the exclusion for free.
    SELECT DISTINCT patient_id, token
    FROM patient_name, regexp_split_to_table(lower(normalize(value, NFC)), '\\s+') AS token
    WHERE token <> '' AND use_key <> ALL(%s)
)"""

_BIRTH_YEAR_CTE = """birth_year AS (
    -- year-range values are EXCLUDED: "1981/1991" would otherwise leak its first
    -- 4-digit run (1981) into name+year / dob+first-initial as if it were a birth year --
    -- a false key (the window min is not a birth year; principle 4). The anchored range
    -- passes (_RANGE_GROUPS_SQL) own ranges.
    SELECT patient_id, substring(value FROM '[0-9]{4}') AS year
    FROM patient_demographic
    WHERE field = 'dob' AND value ~ '[0-9]{4}'
      AND (facets ->> 'precision') IS DISTINCT FROM 'year-range'
)"""

_BLOCKING_SEX_CTE = """blocking_sex AS (
    -- Exclude the uncertainty sentinels (principle 4: no-data-is-never-agreement). The set
    -- is BOUND from adapter.VALUE_SENTINELS_PARAM -- the same set the Python scoring side
    -- treats as absent-value -- so the SQL exclusion can never drift from it (the
    -- placeholder_uses parameter-binding pattern; a hand-mirrored literal here once
    -- lagged the adapter's normalization). Without this, two charts that BOTH merely
    -- recorded sex 'unknown' would share a blocking_sex row and the sex-keyed rescues
    -- (dob-range+sex, name+sex) would key on mutual ignorance rather than a real signal.
    --
    -- The trim approximates the adapter's value.strip() for the whitespace a real feed
    -- plausibly emits: space, tab, LF, CR, FF, VT, NBSP (btrim's DEFAULT trims spaces
    -- ONLY -- it would let a tab-padded sentinel through as a tab-residue key). Python's
    -- strip() also removes rarer Unicode spaces (em-space etc.); those are out of scope:
    -- an exotically-padded sentinel keys on its residue, which at worst adds a noise
    -- pair between two identically-mangled values, never suppresses a true one. lower()
    -- stands in for casefold(); identical for the ASCII values this field carries. The
    -- trimmed form is also the grouping key (padding on a REAL value must not hide a
    -- genuine shared signal); an all-whitespace value trims to '' and is excluded.
    SELECT DISTINCT patient_id, sex FROM (
        SELECT patient_id,
               btrim(lower(value), E' \\t\\n\\r\\f\\u000b\\u00a0') AS sex
        FROM patient_demographic
        WHERE field IN ('sex-at-birth', 'administrative-sex') AND value IS NOT NULL
    ) trimmed
    WHERE sex <> '' AND sex <> ALL(%s)
)"""
```

- [ ] **Step 3: Recompose `_GROUPS_SQL` from the fragments (same arms, same params)**

Replace the current `_GROUPS_SQL` literal (lines ~185-237) with a composed version. Keep the existing top-of-block comment about the `name+year` compound pass. The four arms are UNCHANGED this task; the only change is that `name_tokens` and `birth_year` now come from the shared constants:

```python
_GROUPS_SQL = f"""
WITH {_NAME_TOKENS_CTE},
{_BIRTH_YEAR_CTE}
SELECT 'identifier' AS pass_name, system || ':' || match_key AS key,
       array_agg(patient_id) AS members
FROM patient_identifier WHERE system <> 'unknown'
GROUP BY system, match_key HAVING count(DISTINCT patient_id) >= 2
UNION ALL
SELECT 'dob', value, array_agg(patient_id)
FROM patient_demographic WHERE field = 'dob'
  AND (facets ->> 'precision') IS DISTINCT FROM 'year-range'
GROUP BY value HAVING count(DISTINCT patient_id) >= 2
UNION ALL
SELECT 'name', token, array_agg(patient_id)
FROM name_tokens
GROUP BY token HAVING count(*) >= 2
UNION ALL
SELECT 'name+year', nt.token || '|' || byr.year, array_agg(nt.patient_id)
FROM name_tokens nt JOIN birth_year byr USING (patient_id)
GROUP BY nt.token, byr.year HAVING count(DISTINCT nt.patient_id) >= 2
"""
```

Note: the long explanatory comments that preceded the old inline `name_tokens`/`birth_year` CTEs now live inside the fragment constants (Step 2). Keep the module-level comment block above `_GROUPS_SQL` (lines ~165-184) that explains the exact-`dob` year-range exclusion and the `name+year` semantics — it still applies.

- [ ] **Step 4: Recompose `_RANGE_GROUPS_SQL` from the `blocking_sex` fragment**

Replace the `blocking_sex` inline CTE inside `_RANGE_GROUPS_SQL` (lines ~294-319) with the shared constant, keeping `birth_window` and `window_overlap` inline (they are unique to this statement):

```python
_RANGE_GROUPS_SQL = f"""
WITH birth_window AS (
    -- Evaluation-order-proof malformed-range guard: PostgreSQL does NOT guarantee
    -- WHERE-subexpression evaluation order, so a `split_part(...)::int` cast could be
    -- evaluated BEFORE the `value ~ '^[0-9]{{4}}/[0-9]{{4}}$'` regex guard that exists to
    -- filter it out -- and a non-numeric value ("about-forty") would then raise
    -- "invalid input syntax for type integer" and crash the whole sweep on exactly the
    -- input this guard exists to degrade safely. `substring(value FROM '^([0-9]{{4}})/')`
    -- returns NULL on non-match (never raises), so the cast of NULL is safe and the
    -- comparison against NULL is not-true (row filtered) regardless of evaluation order.
    -- The regex guard is kept too (cheap, and documents intent) but correctness must not
    -- -- and no longer does -- depend on it being evaluated first.
    SELECT patient_id,
           substring(value FROM '^([0-9]{{4}})/')::int AS y_min,
           substring(value FROM '/([0-9]{{4}})$')::int AS y_max,
           TRUE AS is_range
    FROM patient_demographic
    WHERE field = 'dob'
      AND facets ->> 'precision' = 'year-range'
      AND value ~ '^[0-9]{{4}}/[0-9]{{4}}$'
      AND substring(value FROM '^([0-9]{{4}})/')::int <= substring(value FROM '/([0-9]{{4}})$')::int
    UNION ALL
    SELECT patient_id,
           substring(value FROM '[0-9]{{4}}')::int,
           substring(value FROM '[0-9]{{4}}')::int,
           FALSE
    FROM patient_demographic
    WHERE field = 'dob'
      AND value ~ '[0-9]{{4}}'
      AND (facets ->> 'precision') IS DISTINCT FROM 'year-range'
),
{_BLOCKING_SEX_CTE},
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
UNION ALL
SELECT 'dob-range+sex', o.anchor, array_agg(DISTINCT o.member)
FROM window_overlap o
JOIN blocking_sex sa ON sa.patient_id = o.anchor
JOIN blocking_sex sm ON sm.patient_id = o.member AND sm.sex = sa.sex
GROUP BY o.anchor
"""
```

**CRITICAL — f-string brace escaping:** because these are now f-strings, every LITERAL brace in the SQL (the regex quantifiers `{4}`) MUST be doubled to `{{4}}`, as shown above. The `{_BLOCKING_SEX_CTE}` / `{_NAME_TOKENS_CTE}` / `{_BIRTH_YEAR_CTE}` interpolations use single braces. Miss a double-brace and Python raises at import time — the pure suite (Step 6) catches it immediately.

- [ ] **Step 5: Verify the two statements still bind exactly one `%s` each (no arm change yet)**

`_GROUPS_SQL` binds one `%s` (in `name_tokens`); `_RANGE_GROUPS_SQL` binds one `%s` (in `blocking_sex`). The `generate_candidate_pairs` `execute` calls are UNCHANGED this task: `cur.execute(_GROUPS_SQL, (_PLACEHOLDER_USES_PARAM,))` and `cur.execute(_RANGE_GROUPS_SQL, (VALUE_SENTINELS_PARAM,))`. Confirm by reading `db.py:398` and `db.py:413` — do not edit them yet.

- [ ] **Step 6: Run the pure suite (import + f-string sanity) then the DB-gated blocking suites**

Run: `cd matcher && uv run pytest -q`
Expected: PASS — imports cleanly (proves no f-string brace error).

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_candidate_generation.py tests/test_dob_range_blocking.py -v`
Expected: PASS — identical to the Step 1 baseline (behavior-preserving). If no DB, SKIP.

- [ ] **Step 7: Commit**

```bash
git add matcher/src/cairn_matcher/pipeline/db.py
git commit -m "refactor(matcher): extract shared blocking CTE fragments (name_tokens/birth_year/blocking_sex)"
```

---

### Task 3: Add the `dob+first-initial` and `name+sex` arms to `_GROUPS_SQL`

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/db.py` (the composed `_GROUPS_SQL` from Task 2; the `_GROUPS_SQL` execute call in `generate_candidate_pairs`, ~line 398)
- Test: `matcher/tests/test_candidate_generation.py`

**Interfaces:**
- Consumes: `_NAME_TOKENS_CTE`, `_BIRTH_YEAR_CTE`, `_BLOCKING_SEX_CTE` (Task 2); `SYMMETRIC_PASSES` now includes both new names (Task 1); `_PLACEHOLDER_USES_PARAM`, `VALUE_SENTINELS_PARAM`.
- Produces: `_GROUPS_SQL` emits rows for pass_names `dob+first-initial` (key `initial|year`) and `name+sex` (key `token|sex`); the statement now binds TWO params in order `(_PLACEHOLDER_USES_PARAM, VALUE_SENTINELS_PARAM)`.

- [ ] **Step 1: Write the failing tests**

Append to `matcher/tests/test_candidate_generation.py`:

```python
def test_dob_first_initial_rescues_pair_with_no_shared_name_token(pg_conn):
    # PA "Jon" and PB "John" share NO full name token (distinct tokens) and NO exact DOB,
    # but share birth-year 1990 and first initial 'j'. Only dob+first-initial can group
    # them. A different-year decoy (PC, also 'j') keeps this honest: PC shares the initial
    # but not the year, so it must NOT pair with PA/PB via this pass.
    seed_patient(pg_conn, PA, dob=("1990-01-01", 20), names=[("Jon", 20)])
    seed_patient(pg_conn, PB, dob=("1990-12-31", 20), names=[("John", 20)])
    seed_patient(pg_conn, PC, dob=("1970-01-01", 20), names=[("Jane", 20)])
    # Without the new pass (name/name+year/dob need a shared token or exact dob): no pair.
    without = _pairs(pg_conn, enabled_passes={"dob", "name", "name+year"})
    assert canonical_pair(PA, PB) not in without
    # With it: PA-PB rescued; the different-year PC is not pulled in.
    with_pass = _pairs(pg_conn, enabled_passes={"dob", "name", "name+year", "dob+first-initial"})
    assert canonical_pair(PA, PB) in with_pass
    assert canonical_pair(PA, PC) not in with_pass
    assert canonical_pair(PB, PC) not in with_pass


def test_dob_first_initial_excludes_year_range_dob(pg_conn):
    # A §5.4 estimated-age chart (year-range precision) must NOT contribute a birth-YEAR to
    # this point-year pass -- the anchored dob-range passes own ranges. PA (range 1988/1992)
    # and PB (point 1990) share initial 'k' and overlapping years, but dob+first-initial is
    # a POINT-year pass: PA has no point year, so this pass does not pair them.
    seed_patient(pg_conn, PA, dob=("1988/1992", 20, "year-range"), names=[("Kim", 20)])
    seed_patient(pg_conn, PB, dob=("1990-05-05", 20), names=[("Kayla", 20)])
    pairs = _pairs(pg_conn, enabled_passes={"dob+first-initial"})
    assert canonical_pair(PA, PB) not in pairs


def test_name_sex_rescues_oversized_unisex_name_block(pg_conn):
    # A heavily unisex token "sasha" is shared by three charts -> the single-token 'name'
    # block is size 3 and, at cap=2, is skipped wholesale (every pair in it dropped). PA and
    # PB share administrative-sex 'female'; PC is 'male'. Only name+sex can rescue PA-PB by
    # splitting the block on sex. (This is the capped-only benefit the uncapped aggregate
    # blocking-recall metric cannot show -- hence a direct test.)
    seed_patient(pg_conn, PA, admin_sex=("female", 20), names=[("Sasha", 20)])
    seed_patient(pg_conn, PB, admin_sex=("female", 20), names=[("Sasha", 20)])
    seed_patient(pg_conn, PC, admin_sex=("male", 20), names=[("Sasha", 20)])
    pairs, skipped = _gen(pg_conn, max_block_size=2)
    # The oversized single-token 'name' block is still reported skipped...
    assert any(pn == "name" and sz == 3 for pn, _key, sz in skipped)
    # ...but the same-sex sub-block (sasha|female) survives and yields PA-PB.
    assert canonical_pair(PA, PB) in pairs
    # The opposite-sex PC is alone in its sub-block -> no pair with it via name+sex.
    assert canonical_pair(PA, PC) not in pairs
    assert canonical_pair(PB, PC) not in pairs


def test_name_sex_uses_union_of_both_sex_facets(pg_conn):
    # blocking_sex is the UNION of sex-at-birth and administrative-sex (recall-first): a
    # chart recording only sex-at-birth and one recording only administrative-sex still
    # share a sex value. PA (sex-at-birth 'female') and PB (administrative-sex 'female')
    # share token "ari"; a male decoy PC oversizes the 'name' block at cap=2.
    seed_patient(pg_conn, PA, sex=("female", 20), names=[("Ari", 20)])
    seed_patient(pg_conn, PB, admin_sex=("female", 20), names=[("Ari", 20)])
    seed_patient(pg_conn, PC, admin_sex=("male", 20), names=[("Ari", 20)])
    pairs, skipped = _gen(pg_conn, max_block_size=2)
    assert any(pn == "name" and sz == 3 for pn, _key, sz in skipped)
    assert canonical_pair(PA, PB) in pairs


def test_name_sex_ignores_unknown_sentinel_sex(pg_conn):
    # principle 4: two charts that BOTH merely recorded sex 'unknown' must NOT be grouped by
    # mutual ignorance. "unknown" is in adapter.VALUE_SENTINELS, so the sasha|unknown
    # sub-block never forms; with the single-token 'name' block skipped at cap=2, no pair.
    seed_patient(pg_conn, PA, admin_sex=("unknown", 20), names=[("Sasha", 20)])
    seed_patient(pg_conn, PB, admin_sex=("unknown", 20), names=[("Sasha", 20)])
    seed_patient(pg_conn, PC, admin_sex=("male", 20), names=[("Sasha", 20)])
    pairs, _skipped = _gen(pg_conn, max_block_size=2)
    assert canonical_pair(PA, PB) not in pairs
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_candidate_generation.py -k "first_initial or name_sex" -v`
Expected: FAIL — `dob+first-initial` / `name+sex` rows are not yet emitted, so the rescued pairs are absent. (`test_dob_first_initial_excludes_year_range_dob` and `test_name_sex_ignores_unknown_sentinel_sex` may pass vacuously now — that is fine; they are guards that must STAY passing after Step 3.)

- [ ] **Step 3: Add the two arms and bind the second param**

In `matcher/src/cairn_matcher/pipeline/db.py`, add `_BLOCKING_SEX_CTE` to `_GROUPS_SQL`'s `WITH` chain and append the two arms. The composed `_GROUPS_SQL` becomes:

```python
_GROUPS_SQL = f"""
WITH {_NAME_TOKENS_CTE},
{_BIRTH_YEAR_CTE},
{_BLOCKING_SEX_CTE}
SELECT 'identifier' AS pass_name, system || ':' || match_key AS key,
       array_agg(patient_id) AS members
FROM patient_identifier WHERE system <> 'unknown'
GROUP BY system, match_key HAVING count(DISTINCT patient_id) >= 2
UNION ALL
SELECT 'dob', value, array_agg(patient_id)
FROM patient_demographic WHERE field = 'dob'
  AND (facets ->> 'precision') IS DISTINCT FROM 'year-range'
GROUP BY value HAVING count(DISTINCT patient_id) >= 2
UNION ALL
SELECT 'name', token, array_agg(patient_id)
FROM name_tokens
GROUP BY token HAVING count(*) >= 2
UNION ALL
SELECT 'name+year', nt.token || '|' || byr.year, array_agg(nt.patient_id)
FROM name_tokens nt JOIN birth_year byr USING (patient_id)
GROUP BY nt.token, byr.year HAVING count(DISTINCT nt.patient_id) >= 2
UNION ALL
-- dob+first-initial: birth-year + the first CHARACTER of each name token. substring(token
-- FROM 1 FOR 1) is character-wise in PostgreSQL (first code point after NFC, not first
-- byte). A first-initial RELAXATION of 'name': it groups charts that share a birth-year and
-- a first initial but NO full name token (a misspelling/transposition/diacritic variant), so
-- it rescues true matches the token passes miss. Point-year only (birth_year excludes
-- year-range) -- the anchored dob-range passes own ranges. An empty first initial is
-- impossible: name_tokens excludes '' tokens.
SELECT 'dob+first-initial', substring(nt.token FROM 1 FOR 1) || '|' || byr.year,
       array_agg(DISTINCT nt.patient_id)
FROM name_tokens nt JOIN birth_year byr USING (patient_id)
GROUP BY substring(nt.token FROM 1 FOR 1), byr.year
HAVING count(DISTINCT nt.patient_id) >= 2
UNION ALL
-- name+sex: name token + normalized sex (blocking_sex: the sentinel-excluded UNION of
-- sex-at-birth and administrative-sex). A SUBSET of the 'name' block when uncapped (it adds
-- no pairs there); its value is the CAPPED case -- it splits an oversized unisex-token
-- 'name' block the cap drops wholesale into per-sex sub-blocks that fit. Recall-first union:
-- a trans patient whose administrative-sex matches an observation still groups though
-- sex-at-birth differs. DISTINCT because a chart can carry the same sex on both facets.
SELECT 'name+sex', nt.token || '|' || bs.sex, array_agg(DISTINCT nt.patient_id)
FROM name_tokens nt JOIN blocking_sex bs USING (patient_id)
GROUP BY nt.token, bs.sex HAVING count(DISTINCT nt.patient_id) >= 2
"""
```

Then update the `_GROUPS_SQL` execute call in `generate_candidate_pairs` (currently `db.py:398`) to bind BOTH params in the documented order:

```python
        if enabled & SYMMETRIC_PASSES:
            # Two binds now: _PLACEHOLDER_USES_PARAM for name_tokens (first, appears first),
            # VALUE_SENTINELS_PARAM for blocking_sex (second) -- the name+sex arm's sex source.
            cur.execute(_GROUPS_SQL, (_PLACEHOLDER_USES_PARAM, VALUE_SENTINELS_PARAM))
```

- [ ] **Step 4: Run the new tests to verify they pass**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_candidate_generation.py -k "first_initial or name_sex" -v`
Expected: PASS — all five new tests green.

- [ ] **Step 5: Run the full DB-gated blocking suites for regression**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_candidate_generation.py tests/test_dob_range_blocking.py tests/test_pipeline_e2e.py -v`
Expected: PASS — existing tests unaffected (additive-only). In particular `test_toggle_default_none_equals_all_passes` still holds with the enlarged `ALL_PASSES`.

- [ ] **Step 6: Commit**

```bash
git add matcher/src/cairn_matcher/pipeline/db.py matcher/tests/test_candidate_generation.py
git commit -m "feat(matcher): emit dob+first-initial and name+sex blocking pairs"
```

---

### Task 4: Mirror `dob+first-initial` in the pure eval recoverability predicate

**Files:**
- Modify: `matcher/src/cairn_matcher/eval/generator.py:122-153` (`shares_blocking_key`) and add `_first_initials` near `name_tokens` (~line 49)
- Test: `matcher/tests/test_eval_generator_range.py` (or a sibling pure eval test — same module surface)

**Interfaces:**
- Consumes: `name_tokens(record)`, `_birth_window(record)` / `_first_year(value)` (existing).
- Produces: `_first_initials(record) -> set[str]` — the NFC-lowercased first character of each non-placeholder name token. `shares_blocking_key` gains a `dob+first-initial` clause.

- [ ] **Step 1: Write the failing tests**

Append to `matcher/tests/test_eval_generator_range.py`:

```python
from cairn_matcher.eval.generator import _first_initials, name_tokens  # noqa: E402


def test_first_initials_are_first_char_of_each_token():
    rec = _rec(names=("Jon Smith", "Al"))
    # tokens {jon, smith, al} -> initials {j, s, a}
    assert _first_initials(rec) == {"j", "s", "a"}


def test_first_initials_empty_for_nameless_record():
    assert _first_initials(_rec()) == set()


def test_shares_key_via_first_initial_and_point_year_without_shared_token():
    # No shared token (jon vs john), no shared exact dob, but same point-year 1990 and same
    # first initial 'j' -> dob+first-initial recovers them. This is the ONLY reason these two
    # share a key, so it isolates the new clause.
    a = _rec(dob={"value": "1990-01-01", "precision": "day"}, names=("Jon",))
    b = _rec(dob={"value": "1990-12-31", "precision": "day"}, names=("John",))
    assert shares_blocking_key(a, b) is True


def test_no_shared_key_when_first_initial_differs():
    a = _rec(dob={"value": "1990-01-01", "precision": "day"}, names=("Jon",))
    b = _rec(dob={"value": "1990-12-31", "precision": "day"}, names=("Alan",))
    assert shares_blocking_key(a, b) is False


def test_no_shared_key_when_initial_matches_but_year_differs():
    a = _rec(dob={"value": "1990-01-01", "precision": "day"}, names=("Jon",))
    b = _rec(dob={"value": "1971-12-31", "precision": "day"}, names=("John",))
    assert shares_blocking_key(a, b) is False


def test_first_initial_clause_excludes_year_range_dob():
    # A year-range dob has no POINT year (mirrors the SQL birth_year exclusion), so the
    # first-initial clause cannot fire off it. These share nothing else.
    a = _rec(dob={"value": "1988/1992", "precision": "year-range"}, names=("Jon",))
    b = _rec(dob={"value": "1990-12-31", "precision": "day"}, names=("John",))
    assert shares_blocking_key(a, b) is False
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd matcher && uv run pytest tests/test_eval_generator_range.py -k "first_initial or first_char or via_first" -v`
Expected: FAIL — `_first_initials` is undefined (ImportError) and the new clause does not exist.

- [ ] **Step 3: Add `_first_initials` and the `shares_blocking_key` clause**

In `matcher/src/cairn_matcher/eval/generator.py`, add after `name_tokens` (~line 49):

```python
def _first_initials(record: Mapping) -> set[str]:
    """First character of each of the record's name tokens -- the dob+first-initial mirror.

    Mirrors the SQL pass's `substring(token FROM 1 FOR 1)` over the same name_tokens (NFC,
    lower, placeholder-excluded). PostgreSQL substring is character-wise, so this takes the
    first code point of each already-NFC-lowercased token. Empty tokens are impossible
    (name_tokens drops them), so every returned initial is a real character.
    """
    return {token[0] for token in name_tokens(record)}
```

Then extend `shares_blocking_key`. Insert the new clause BEFORE the final `return bool(name_tokens(a) & name_tokens(b))` (currently line 153). It REUSES the `wa, wb = _birth_window(a), _birth_window(b)` already computed for the range clause just above (line 150) — do not recompute. Update the docstring to document it. The clause:

```python
    # dob+first-initial: a shared first initial AND a shared POINT birth-year, even with no
    # shared full name token. Reuses wa/wb (_birth_window, computed above); the point branch
    # is is_range False with y_min == y_max, so `not wa[2]` selects a point year and wa[0]
    # is that year. A year-range dob -- which has no point year -- thus never satisfies this,
    # faithful to the SQL's birth_year CTE (year-range excluded). name+sex needs NO clause
    # here: it is a subset of the shared-name-token check below in the uncapped model this
    # mirror represents (exactly as name+year is subsumed), and its capped-only rescue is
    # tested directly against the DB.
    if (
        wa and wb and not wa[2] and not wb[2] and wa[0] == wb[0]
        and (_first_initials(a) & _first_initials(b))
    ):
        return True
```

Update the `shares_blocking_key` docstring: add a sentence noting the `dob+first-initial` clause (shared initial + shared point year) and that `name+sex` is subsumed by the name-token check.

- [ ] **Step 4: Run the new tests to verify they pass**

Run: `cd matcher && uv run pytest tests/test_eval_generator_range.py -k "first_initial or first_char or via_first" -v`
Expected: PASS.

- [ ] **Step 5: Run the whole pure eval + generator suite for regression**

Run: `cd matcher && uv run pytest tests/test_eval_generator_range.py tests/test_eval_generator.py tests/test_eval_generator_volume.py tests/test_eval_generator_sync.py -v`
Expected: PASS — the recoverable-by-construction guarantee still holds (the new clause only ADDS recoverable pairs, matching the SQL's added pass).

- [ ] **Step 6: Commit**

```bash
git add matcher/src/cairn_matcher/eval/generator.py matcher/tests/test_eval_generator_range.py
git commit -m "feat(matcher/eval): mirror dob+first-initial in shares_blocking_key recoverability"
```

---

### Task 5: Full-suite verification + docs currency

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md` (record slice 25)
- No code changes.

**Interfaces:** none.

- [ ] **Step 1: Run the entire pure suite**

Run: `cd matcher && uv run pytest -q`
Expected: PASS — all pure tests green, ruff-clean surface (run `cd matcher && uv run ruff check .` → no findings).

- [ ] **Step 2: Run the entire DB-gated suite**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest -q`
Expected: PASS (DB-gated tests run; none skipped when `CAIRN_TEST_PG` is set). Record the exact passed/skipped counts for the commit + HANDOVER.

- [ ] **Step 3: Update HANDOVER.md**

Add a "This session (2026-07-07)" entry at the top of `docs/HANDOVER.md` summarizing matcher slice 25 (compound blocking keys `dob+first-initial` + `name+sex`; advisory eval/matcher tier only, no floor/SCHEMA/event/migration/ADR/spec change; the CTE-fragment extraction; the honest limit that name+sex is invisible to the uncapped metric and is proven by a targeted capped DB test). Demote the previous "This session" entry to "Prior session". Fold `dob+first-initial`/`name+sex` out of the "Next (B3 measurement-driven)" remaining-work list. Keep the file under 500 lines (prune the oldest condensed entries if needed).

- [ ] **Step 4: Update ROADMAP.md**

Add a slice-25 line under the matcher build entries (mirror the slice-23/24 style: one dense line — what shipped, the tier, the honest limits, the commit/PR to be filled after push).

- [ ] **Step 5: Commit the docs**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: record matcher slice 25 (compound blocking keys) in HANDOVER + ROADMAP"
```

- [ ] **Step 6: Push and open the PR**

```bash
git push -u origin HEAD
gh pr create --base main --title "matcher slice 25: compound blocking keys (dob+first-initial, name+sex)" --body "<summary + honest limits + test counts>"
```

(Branch/PR mechanics are handled by the finishing-a-development-branch flow; link no issue unless one is opened for name+sex's real-data validation follow-on.)

---

## Notes for the implementer

- **uv only**, never venv/pip (project house rule).
- The pure suite must stay green with NO database (`CAIRN_TEST_PG` unset → DB tests SKIP). Never make a pure module import psycopg.
- `blocking.py` is deliberately PURE (psycopg-free); do not import anything DB-bound into it.
- When editing the f-string SQL in `db.py`, remember: literal `{4}` regex quantifiers → `{{4}}`; CTE interpolations → single braces. A brace error surfaces at import (the pure suite catches it).
- Additive-only is the safety property: if any existing test that asserts a pair is ABSENT starts failing, you have created a false group — stop and investigate, do not "fix" the test.
