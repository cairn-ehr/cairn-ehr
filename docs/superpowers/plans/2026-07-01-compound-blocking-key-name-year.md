# Compound Blocking Key (name-token + birth-year) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an additive `name+year` compound blocking pass to the §5.2 matcher so over-broad single-name-token blocks are partitioned by birth-year and survive the oversized-block cap, recovering true-match pairs that are otherwise dropped.

**Architecture:** A single new `UNION ALL` branch (plus one CTE) in `_GROUPS_SQL` in `matcher/src/cairn_matcher/pipeline/db.py`. Each blocking pass still emits `(pass_name, key, members)`, so the uniform cap/skip logic, `skipped_blocks` reporting, and the pure `_pairs_from_members` expander are unchanged. Pairs are deduped by canonical uuid pair across all passes, making the union strictly recall-non-decreasing versus the current three passes.

**Tech Stack:** Python 3 (uv, never venv/pip), psycopg (optional `pipeline` extra), PostgreSQL ≥ 18 + cairn_pgx, pytest.

## Global Constraints

- AGPL-3.0; no new dependency may be added (the change is pure SQL inside the existing `pipeline` extra). — copied from spec
- Advisory tier (§9 fit-for-purpose): **no `db/` floor file, no SCHEMA bump, no spec/ADR change** (implements settled §5.2 / §5.13 / ADR-0014). — copied from spec
- TDD: failing test first, then code. House rule #2.
- Culture-neutral honest degrade (principle 4): birth-year derived **only** via `left(value, 4)` guarded by `value ~ '^[0-9]{4}'`; no date parsing, no assumed calendar. — copied from spec
- Junior-legible inline comments on the new SQL. House rule #3.
- `db.py` stays well under 500 lines. House rule #4.
- DB-gated tests run with: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest`. They SKIP cleanly when `CAIRN_TEST_PG` is unset.

---

### Task 1: Add the `name+year` compound blocking pass

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/db.py` (the `_GROUPS_SQL` string near line 59, its preceding comment, and the module/function docstrings that say "three blocking passes")
- Test: `matcher/tests/test_candidate_generation.py` (append)

**Interfaces:**
- Consumes: `generate_candidate_pairs(conn, *, max_block_size=100) -> (pairs, skipped_blocks)` and `canonical_pair(a, b)` (both already exist; signatures unchanged).
- Produces: no new Python symbol. A new blocking pass whose rows carry `pass_name='name+year'`, `key='<token>|<year>'`. `generate_candidate_pairs`'s return shape is unchanged; the `name+year` pass simply contributes more `members` groups, expanded and deduped by the existing code path.

- [ ] **Step 1: Write the failing test**

Append to `matcher/tests/test_candidate_generation.py`:

```python
def test_name_year_rescues_pair_from_oversized_name_block(pg_conn):
    # Three patients share the name token "smith" -> the single-token 'name' block is
    # size 3. At cap=2 that block is oversized and skipped today, dropping every pair in
    # it. PA and PB also share a birth-year (1980) but NOT an exact DOB, so only the new
    # 'name+year' compound pass can rescue their pair.
    seed_patient(pg_conn, PA, dob=("1980-01-01", 20), names=[("Smith", 20)])
    seed_patient(pg_conn, PB, dob=("1980-06-06", 20), names=[("Smith", 20)])
    seed_patient(pg_conn, PC, dob=("1991-01-01", 20), names=[("Smith", 20)])
    pairs, skipped = _gen(pg_conn, max_block_size=2)
    # The oversized single-token block is still reported as skipped...
    assert any(pn == "name" and sz == 3 for pn, _key, sz in skipped)
    # ...but the same-year sub-block (smith|1980) survives and yields PA-PB.
    assert canonical_pair(PA, PB) in pairs
    # The different-year patient (PC, 1991) is alone in its sub-block -> no pair with it.
    assert canonical_pair(PA, PC) not in pairs
    assert canonical_pair(PB, PC) not in pairs
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_candidate_generation.py::test_name_year_rescues_pair_from_oversized_name_block -v`
Expected: FAIL — `canonical_pair(PA, PB)` is not in `pairs` (the oversized "smith" block is skipped and no `name+year` pass exists yet, so `pairs == []`).

- [ ] **Step 3: Add the compound pass to `_GROUPS_SQL`**

In `matcher/src/cairn_matcher/pipeline/db.py`, replace the `_GROUPS_SQL` definition (currently the `name_tokens` CTE + three `UNION ALL` passes) with the version below — it adds a `birth_year` CTE and a fourth `name+year` pass, leaving the three existing passes byte-for-byte unchanged:

```python
# Each pass yields rows of (pass_name, key, members) so the cap can be applied uniformly:
# a group is kept (pairs generated) iff cardinality(members) <= cap, else reported skipped.
# Blocking is RECALL-oriented and advisory: the SQL name tokenizer is deliberately simple
# (lower + whitespace split); the Python scorer remains the source of truth for comparison.
#
# The 'name+year' pass is a COMPOUND key (name token + birth-year). It is ADDITIVE: the
# single-token 'name' pass is retained, and pairs are deduped by canonical uuid pair across
# passes, so adding this pass can only RAISE recall (it rescues pairs from an oversized
# single-token block, which the cap would otherwise drop wholesale). Birth-year is taken as
# the leading 4 digits of the stored DOB value ONLY when the value begins with 4 digits
# (`value ~ '^[0-9]{4}'`) -- an honest, culture-neutral degrade that parses no date and
# assumes no calendar (principle 4); a record with a null/non-ISO DOB simply does not join
# this pass and stays covered by the single-token 'name' pass. Because left() truncates,
# this pass also groups precision-mismatched true matches ("1990" vs "1990-05-12") that the
# exact-DOB pass never groups.
_GROUPS_SQL = """
WITH name_tokens AS (
    SELECT DISTINCT patient_id, token
    FROM patient_name, regexp_split_to_table(lower(value), '\\s+') AS token
    WHERE token <> ''
),
birth_year AS (
    SELECT patient_id, left(value, 4) AS year
    FROM patient_demographic
    WHERE field = 'dob' AND value ~ '^[0-9]{4}'
)
SELECT 'identifier' AS pass_name, system || ':' || match_key AS key,
       array_agg(patient_id) AS members
FROM patient_identifier WHERE system <> 'unknown'
GROUP BY system, match_key HAVING count(DISTINCT patient_id) >= 2
UNION ALL
SELECT 'dob', value, array_agg(patient_id)
FROM patient_demographic WHERE field = 'dob'
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

Also update the wording that now undercounts the passes:
- The module docstring line `loads a patient's projection rows, calls the in-DB veto floor` is unaffected; but the `generate_candidate_pairs` docstring says "three blocking passes". Change "three blocking passes" to "four blocking passes (identifier / exact-DOB / name-token / name-token+birth-year)".

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_candidate_generation.py::test_name_year_rescues_pair_from_oversized_name_block -v`
Expected: PASS.

- [ ] **Step 5: Run the full candidate-generation file to confirm no regression**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_candidate_generation.py -v`
Expected: PASS (all prior tests + the new one). The existing oversized-block and dedup tests still hold because the three original passes are unchanged.

- [ ] **Step 6: Commit**

```bash
git add matcher/src/cairn_matcher/pipeline/db.py matcher/tests/test_candidate_generation.py
git commit -m "feat(matcher): additive name+year compound blocking pass (B3)

Partition over-broad single-name-token blocks by birth-year so sub-blocks
survive the oversized-block cap, recovering true-match pairs the cap drops.
Additive UNION ALL branch -> recall-non-decreasing by dedup. Honest-degrade
ISO-prefix year guard (no date parsing); also rescues precision-mismatched
DOBs. Advisory: no db/ floor, no SCHEMA bump.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Lock in honest-degrade, precision-mismatch, and cross-pass dedup

**Files:**
- Test: `matcher/tests/test_candidate_generation.py` (append)

**Interfaces:**
- Consumes: `generate_candidate_pairs` / `canonical_pair` / `seed_patient` (unchanged). These are characterization tests for the Task 1 SQL; they add no production code.

- [ ] **Step 1: Write the three guard tests**

Append to `matcher/tests/test_candidate_generation.py`:

```python
def test_name_year_honest_degrade_no_recall_regression(pg_conn):
    # PB has no DOB, so it cannot join the 'name+year' pass. The shared "jones" token must
    # still group PA-PB via the single-token 'name' pass -> coverage never regresses for a
    # record with a missing (or non-ISO) DOB. (A non-ISO value like "07/15/80" fails the
    # `^[0-9]{4}` guard identically.)
    seed_patient(pg_conn, PA, dob=("1985-03-03", 20), names=[("Jones", 20)])
    seed_patient(pg_conn, PB, names=[("Jones", 20)], identifiers=[("mrn:a", "2", "2")])
    assert canonical_pair(PA, PB) in _pairs(pg_conn)


def test_name_year_rescues_precision_mismatched_dob(pg_conn):
    # Year-precision "1990" vs day-precision "1990-05-12": left(value,4) = "1990" for both,
    # so they share the 'name|1990' sub-block -- though the exact-DOB pass never groups them.
    # A different-year decoy (PC) oversizes the single "garcia" token block at cap=2, so only
    # the compound pass can produce PA-PB.
    seed_patient(pg_conn, PA, dob=("1990", 20, "year"), names=[("Garcia", 20)])
    seed_patient(pg_conn, PB, dob=("1990-05-12", 20, "day"), names=[("Garcia", 20)])
    seed_patient(pg_conn, PC, dob=("2000-01-01", 20), names=[("Garcia", 20)])
    pairs, skipped = _gen(pg_conn, max_block_size=2)
    assert any(pn == "name" and sz == 3 for pn, _key, sz in skipped)
    assert canonical_pair(PA, PB) in pairs


def test_name_and_name_year_pair_is_emitted_once(pg_conn):
    # PA and PB share BOTH a name token and a birth-year, so the 'name' and 'name+year'
    # passes both surface the pair. After canonical-pair dedup it appears exactly once.
    seed_patient(pg_conn, PA, dob=("1975-08-08", 20), names=[("Patel", 20)])
    seed_patient(pg_conn, PB, dob=("1975-08-08", 20), names=[("Patel", 20)])
    pairs = _pairs(pg_conn)
    assert pairs.count(canonical_pair(PA, PB)) == 1
```

- [ ] **Step 2: Run the three tests**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_candidate_generation.py -k "honest_degrade or precision_mismatched or emitted_once" -v`
Expected: PASS for all three against the Task 1 SQL. (If `test_name_year_rescues_precision_mismatched_dob` fails, the `left(value,4)` guard or join is wrong — fix the SQL, not the test.)

- [ ] **Step 3: Run the whole matcher suite with DB**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest`
Expected: PASS, no skips (DB present). Confirms the blocking eval (`test_eval_blocking.py`) and sweep tests still pass — they call the real `generate_candidate_pairs`, now with the extra pass.

- [ ] **Step 4: Run the pure suite to confirm the pure path is untouched**

Run: `cd matcher && uv run pytest`
Expected: PASS with the DB-gated tests SKIPPED (no `CAIRN_TEST_PG`). Confirms the change added no import-time DB dependency to the pure core.

- [ ] **Step 5: Commit**

```bash
git add matcher/tests/test_candidate_generation.py
git commit -m "test(matcher): name+year honest-degrade, precision-mismatch, cross-pass dedup

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Optional manual harness check + HANDOVER/ROADMAP update

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md` (record the compound-blocking slice; carry the ISO-guard limitation forward)

**Interfaces:** none (docs only). This task has no test; it is the session wrap-up gate.

- [ ] **Step 1: Eyeball the harness report on the gold set (optional sanity check)**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline python -m cairn_matcher.eval --max-block-size 100`
Expected: the blocking report prints; `pair_completeness` is ≥ its previous value on `gold_v1` (the gold set has no oversized blocks, so the number should be unchanged — this confirms additivity did not lower recall). Note the value; it is a sanity check, not an assertion.

- [ ] **Step 2: Update HANDOVER.md**

Add a top-of-file session entry (2026-07-01) summarizing: additive `name+year` compound blocking pass landed in `pipeline/db.py`; recall-non-decreasing by dedup; honest-degrade ISO-prefix year guard with the **known limitation** that the `'^[0-9]{4}'` guard's real-world adequacy is an empirical bet to revisit on richer data; advisory (no `db/`, no SCHEMA, no spec/ADR); deferred still: synthetic volume generator, further compound keys (`dob+first-initial`, `name+sex`), weight-learning. Move "compound blocking keys" out of the "Next (now unblocked)" list since it is now built.

- [ ] **Step 3: Update ROADMAP.md**

In the matcher slice list (Phase 4), record compound blocking keys as built; update the "Remaining matcher pieces / Next" line to drop compound blocking keys and lead with weight-learning + piece C.

- [ ] **Step 4: Commit**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: record name+year compound blocking slice in HANDOVER + ROADMAP

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## After implementation

- Request a whole-branch code review (superpowers:requesting-code-review or the project's opus review) before opening the PR, per the house workflow.
- Open a PR to `main` describing the additive compound pass, the recall-non-decreasing argument, and the deferred items. No issue is currently open for this slice; reference the B3 thread in HANDOVER/ROADMAP.

## Notes for the implementer

- The `seed_patient` fixture's `dob` arg is `(value, provenance_rank)` or `(value, provenance_rank, precision)`; precision defaults to `"day"`. Identifiers are `(system, match_key, value)`. Names are `(value, provenance_rank)`. See `matcher/tests/conftest.py`.
- `_gen` returns `(pairs, skipped_blocks)`; `_pairs` returns just `pairs`. Both helpers already exist at the top of `test_candidate_generation.py`.
- Do NOT change `_pairs_from_members`, `generate_candidate_pairs`'s signature, the cap logic, `runner.py`, `sweep.py`, or any `db/*.sql` file.
- The SQL string already escapes the backslash in `'\\s+'` for Python; keep that exact form in the `name_tokens` CTE.
```
