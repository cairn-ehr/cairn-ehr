# Matcher piece B2b — blocking / candidate-pair generation + batch sweep — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a blocking candidate-pair generator and a batch sweep driver to the advisory matcher, so it can decide *which* patient pairs to score across the whole projection set and feed each through the existing `propose()` — closing the "an external driver must supply the pairs" gap.

**Architecture:** Two additions to the existing `matcher/src/cairn_matcher/pipeline/` package. `db.generate_candidate_pairs()` runs one read-only blocking query (a `UNION` of three group-based passes over `patient_identifier` / `patient_demographic` / `patient_name`, deduped canonically, with an oversized-block guard) and lives in the only IO module. `sweep.sweep()` is pure orchestration: generate candidates, close the read snapshot, then loop the existing `runner.propose()` per pair with skip-and-report error handling, returning a `SweepResult` summary. No DB-floor migration, no SCHEMA bump, no spec/ADR change — advisory, fit-for-purpose Python (§9).

**Tech Stack:** Python ≥ 3.11, psycopg 3 (optional `pipeline` extra), PostgreSQL ≥ 18 + cairn_pgx (integration tests only), pytest, uv.

## Global Constraints

- **AGPL-3.0-only**; psycopg is the only allowed runtime dep, and only under the optional `pipeline` extra. The pure core (`agreement`/`comparators`/`records`/`orchestrator`/`scoring`/`adapter`/`banding`) must stay psycopg-free.
- **Advisory tier, not the safety floor.** No new SQL file in `db/`, no change to the `cairn-node` SCHEMA array, no change to `submit_event`, `db/016`, or `db/017`. Blocking logic lives in the matcher package only.
- **TDD** — failing test first, then minimal code. **Integration tests** are gated on `CAIRN_TEST_PG` and skip cleanly without it; run them with `--extra pipeline`.
- **Canonical pair order** is by patient **uuid value** (Postgres uuid byte order == `uuid.UUID` 128-bit integer order == `runner.canonical_pair`), never text order.
- **Inline docs for a junior dev**; pure functions where practical; files < 500 lines.
- Run all commands from the `matcher/` directory. Pure suite: `uv run pytest`. Integration: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest`.

## File Structure

- **Modify** `matcher/src/cairn_matcher/pipeline/db.py` (~75 → ~120 lines) — add `generate_candidate_pairs()` (the one new query). Returns plain tuples; defines no value types (keeps `db` free of any `sweep` import → no circular import).
- **Create** `matcher/src/cairn_matcher/pipeline/sweep.py` (~110 lines) — the `SkippedBlock` / `SweepError` / `SweepResult` value types and the `sweep()` orchestration.
- **Create** `matcher/tests/test_candidate_generation.py` — integration tests for `generate_candidate_pairs`.
- **Create** `matcher/tests/test_sweep.py` — integration tests for `sweep()`.

---

### Task 1: `generate_candidate_pairs` — the three blocking passes (no cap yet)

Group-based candidate generation: each pass groups patients by a blocking value, keeps groups with ≥ 2 members, and emits every within-group pair. `UNION`ed and deduped to one canonical row per pair. The `max_block_size` parameter is accepted but not yet enforced (Task 2 adds the guard), and `skipped_blocks` is returned empty.

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/db.py`
- Test: `matcher/tests/test_candidate_generation.py` (create)

**Interfaces:**
- Consumes: an open `psycopg` connection with the `patient_*` projections populated (same fixtures as B2 tests).
- Produces: `generate_candidate_pairs(conn, *, max_block_size: int = 100) -> tuple[list[tuple[str, str]], list[tuple[str, str, int]]]` — returns `(pairs, skipped_blocks)`. `pairs` is a list of canonical `(patient_low, patient_high)` lowercase-uuid-text tuples, each unique. `skipped_blocks` is a list of `(pass_name, key, size)` (empty until Task 2). Read-only: opens a read transaction the caller is responsible for closing.

- [ ] **Step 1: Write the failing tests**

Create `matcher/tests/test_candidate_generation.py`:

```python
# matcher/tests/test_candidate_generation.py
"""Integration tests for db.generate_candidate_pairs (blocking).

Seed patient_* projection rows directly, then assert which canonical pairs the three
blocking passes (identifier / exact-DOB / name-token) generate. Gated on CAIRN_TEST_PG.
"""

from cairn_matcher.pipeline.runner import canonical_pair
from tests.conftest import seed_patient

PA = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
PB = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
PC = "cccccccc-cccc-cccc-cccc-cccccccccccc"


def _pairs(conn, **kw):
    from cairn_matcher.pipeline.db import generate_candidate_pairs
    pairs, _skipped = generate_candidate_pairs(conn, **kw)
    return pairs


def test_shared_identifier_generates_the_pair(pg_conn):
    seed_patient(pg_conn, PA, identifiers=[("mrn:a", "111", "111")])
    seed_patient(pg_conn, PB, identifiers=[("mrn:a", "111", "111")])
    assert canonical_pair(PA, PB) in _pairs(pg_conn)


def test_shared_name_token_generates_the_pair(pg_conn):
    # Only a shared token "alex"; distinct identifiers, no DOB.
    seed_patient(pg_conn, PA, names=[("Alex Smith", 20)], identifiers=[("mrn:a", "1", "1")])
    seed_patient(pg_conn, PB, names=[("Alex Jones", 20)], identifiers=[("mrn:a", "2", "2")])
    assert canonical_pair(PA, PB) in _pairs(pg_conn)


def test_shared_exact_dob_generates_the_pair(pg_conn):
    seed_patient(pg_conn, PA, dob=("1980-07-15", 20))
    seed_patient(pg_conn, PB, dob=("1980-07-15", 20))
    assert canonical_pair(PA, PB) in _pairs(pg_conn)


def test_no_shared_block_does_not_generate(pg_conn):
    seed_patient(pg_conn, PA, dob=("1980-07-15", 20), names=[("Alex Smith", 20)],
                 identifiers=[("mrn:a", "1", "1")])
    seed_patient(pg_conn, PB, dob=("1991-02-02", 20), names=[("Robin Jones", 20)],
                 identifiers=[("mrn:a", "2", "2")])
    assert canonical_pair(PA, PB) not in _pairs(pg_conn)


def test_pair_sharing_two_keys_is_emitted_once(pg_conn):
    # Same identifier AND same DOB -> two passes hit -> still one row after DISTINCT.
    for p in (PA, PB):
        seed_patient(pg_conn, p, dob=("1980-07-15", 20), identifiers=[("mrn:a", "9", "9")])
    pairs = _pairs(pg_conn)
    assert pairs.count(canonical_pair(PA, PB)) == 1


def test_unknown_system_never_blocks(pg_conn):
    seed_patient(pg_conn, PA, identifiers=[("unknown", "x", "x")])
    seed_patient(pg_conn, PB, identifiers=[("unknown", "x", "x")])
    assert canonical_pair(PA, PB) not in _pairs(pg_conn)


def test_pairs_are_canonical_and_self_excluded(pg_conn):
    # Three patients all sharing one identifier -> C(3,2)=3 pairs, all low<high, none self.
    for p in (PA, PB, PC):
        seed_patient(pg_conn, p, identifiers=[("mrn:a", "7", "7")])
    pairs = _pairs(pg_conn)
    assert len(pairs) == 3
    for low, high in pairs:
        assert low < high
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_candidate_generation.py -v`
Expected: FAIL — `ImportError: cannot import name 'generate_candidate_pairs'` (or `AttributeError`).

- [ ] **Step 3: Implement `generate_candidate_pairs` (no cap)**

Append to `matcher/src/cairn_matcher/pipeline/db.py`:

```python
# The three blocking passes share one shape: group patients by a blocking value, keep
# groups with >= 2 members, and emit every within-group pair. Group-based (not direct
# self-joins) because Task 2's oversized-block guard needs each group's member count.
#
# Canonical order is enforced in SQL by m1 < m2 on the uuid VALUES (Postgres uuid byte
# order == uuid.UUID 128-bit order == runner.canonical_pair), so a pair is one stable row.
# Blocking is RECALL-oriented and advisory: the SQL name tokenizer is deliberately simple
# (lower + whitespace split); the Python scorer remains the source of truth for comparison.
_CANDIDATE_SQL = """
WITH ident_groups AS (
    SELECT array_agg(patient_id) AS members
    FROM patient_identifier
    WHERE system <> 'unknown'
    GROUP BY system, match_key
    HAVING count(DISTINCT patient_id) >= 2
),
dob_groups AS (
    SELECT array_agg(patient_id) AS members
    FROM patient_demographic
    WHERE field = 'dob'
    GROUP BY value
    HAVING count(DISTINCT patient_id) >= 2
),
name_tokens AS (
    SELECT DISTINCT patient_id, token
    FROM patient_name, regexp_split_to_table(lower(value), '\\s+') AS token
    WHERE token <> ''
),
name_groups AS (
    SELECT array_agg(patient_id) AS members
    FROM name_tokens
    GROUP BY token
    HAVING count(*) >= 2
),
all_groups AS (
    SELECT members FROM ident_groups
    UNION ALL SELECT members FROM dob_groups
    UNION ALL SELECT members FROM name_groups
)
SELECT DISTINCT m1::text AS patient_low, m2::text AS patient_high
FROM all_groups g, unnest(g.members) m1, unnest(g.members) m2
WHERE m1 < m2
"""


def generate_candidate_pairs(conn, *, max_block_size: int = 100):
    """Generate the canonical candidate pairs worth scoring, via three blocking passes.

    Returns (pairs, skipped_blocks): `pairs` is a list of unique canonical
    (patient_low, patient_high) lowercase-uuid-text tuples; `skipped_blocks` reports
    oversized blocks excluded from generation (empty until the cap is wired in).

    Read-only — opens a read transaction the CALLER must close (sweep does conn.rollback
    before its write loop, so a long sweep does not pin the xmin horizon).
    """
    with conn.cursor() as cur:
        cur.execute(_CANDIDATE_SQL)
        pairs = [(low, high) for low, high in cur.fetchall()]
    skipped_blocks: list[tuple[str, str, int]] = []
    return pairs, skipped_blocks
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_candidate_generation.py -v`
Expected: PASS (7 passed).

- [ ] **Step 5: Confirm the pure suite still skips cleanly (no DB)**

Run: `uv run pytest tests/test_candidate_generation.py -v`
Expected: 7 skipped ("CAIRN_TEST_PG not set").

- [ ] **Step 6: Commit**

```bash
git add matcher/src/cairn_matcher/pipeline/db.py matcher/tests/test_candidate_generation.py
git commit -m "feat(matcher): blocking candidate-pair generation (B2b, 3 passes)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: oversized-block guard (`max_block_size` + `skipped_blocks`)

Enforce the cap: a blocking-value group with more than `max_block_size` members is excluded from pair generation and reported in `skipped_blocks` as `(pass_name, key, size)`. The cap is per-group, not global, so an in-cap block in the same run still generates its pairs.

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/db.py`
- Test: `matcher/tests/test_candidate_generation.py`

**Interfaces:**
- Consumes: the Task 1 `generate_candidate_pairs` signature (unchanged).
- Produces: same signature; now `max_block_size` is enforced and `skipped_blocks` is populated with `(pass_name, key, size)` tuples where `pass_name ∈ {"identifier","dob","name"}`, `key` is the human-readable blocking value (`system:match_key`, the dob string, or the token), and `size` is the group's member count.

- [ ] **Step 1: Write the failing tests**

Append to `matcher/tests/test_candidate_generation.py`:

```python
PD = "dddddddd-dddd-dddd-dddd-dddddddddddd"


def test_oversized_block_is_skipped_and_reported(pg_conn):
    # cap=2: three patients share one DOB -> group size 3 > 2 -> skipped, no pairs from it.
    for p in (PA, PB, PC):
        seed_patient(pg_conn, p, dob=("1980-07-15", 20))
    pairs, skipped = __import__(
        "cairn_matcher.pipeline.db", fromlist=["generate_candidate_pairs"]
    ).generate_candidate_pairs(pg_conn, max_block_size=2)
    assert pairs == []
    assert any(pn == "dob" and sz == 3 for pn, _key, sz in skipped)


def test_cap_is_per_group_not_global(pg_conn):
    # An oversized DOB block (PA,PB,PC) is skipped, but an in-cap identifier block
    # (PA,PD) in the SAME run is still generated.
    for p in (PA, PB, PC):
        seed_patient(pg_conn, p, dob=("1980-07-15", 20))
    seed_patient(pg_conn, PD)
    with pg_conn.cursor() as cur:
        cur.execute("INSERT INTO patient_identifier (patient_id, system, match_key, value, "
                    "normalized, profile, use_type, provenance, asserted_hlc_wall, "
                    "asserted_hlc_count, asserted_origin) VALUES "
                    "(%s,'mrn:a','55','55','55',NULL,NULL,'seed',0,0,'seed'),"
                    "(%s,'mrn:a','55','55','55',NULL,NULL,'seed',0,0,'seed')", (PA, PD))
    pg_conn.commit()
    pairs, skipped = __import__(
        "cairn_matcher.pipeline.db", fromlist=["generate_candidate_pairs"]
    ).generate_candidate_pairs(pg_conn, max_block_size=2)
    assert canonical_pair(PA, PD) in pairs
    assert any(pn == "dob" and sz == 3 for pn, _key, sz in skipped)
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_candidate_generation.py -k "oversized or per_group" -v`
Expected: FAIL — `assert pairs == []` fails (cap not yet enforced; the oversized block still emits its 3 pairs) and `skipped` is empty.

- [ ] **Step 3: Implement the cap**

In `matcher/src/cairn_matcher/pipeline/db.py`, replace the `_CANDIDATE_SQL` group CTEs and the function body so each pass carries a `pass_name`/`key`/member array, kept groups (`cardinality <= cap`) generate pairs, and oversized groups are reported. Replace `_CANDIDATE_SQL` and `generate_candidate_pairs` with:

```python
# Each pass yields rows of (pass_name, key, members) so the cap can be applied uniformly:
# a group is kept (pairs generated) iff cardinality(members) <= cap, else reported skipped.
_GROUPS_SQL = """
WITH name_tokens AS (
    SELECT DISTINCT patient_id, token
    FROM patient_name, regexp_split_to_table(lower(value), '\\s+') AS token
    WHERE token <> ''
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
"""


def _pairs_from_members(members: list[str]) -> set[tuple[str, str]]:
    """Every canonical within-group pair (uuid value order), as lowercase-uuid-text.

    Pure: the same uuid ordering as runner.canonical_pair, so a pair has one identity no
    matter which group (or pass) surfaces it. Self-pairs are excluded by the strict order.
    """
    ordered = [str(uuid.UUID(str(m))) for m in members]
    out: set[tuple[str, str]] = set()
    for i, a in enumerate(ordered):
        for b in ordered[i + 1:]:
            out.add((a, b) if uuid.UUID(a) < uuid.UUID(b) else (b, a))
    return out


def generate_candidate_pairs(conn, *, max_block_size: int = 100):
    """Generate canonical candidate pairs via three blocking passes, capping huge blocks.

    Returns (pairs, skipped_blocks). `pairs`: unique canonical (low, high) lowercase-uuid
    tuples from every group with <= max_block_size members. `skipped_blocks`: the
    (pass_name, key, size) of each group EXCLUDED for exceeding the cap — a block shared
    by hundreds of people is non-discriminating (a group of size k contributes C(k,2)
    pairs), and the §5.13 hub duplicate-sweep is the declared backstop for what it drops.

    Read-only — opens a read transaction the CALLER must close.
    """
    pairs: set[tuple[str, str]] = set()
    skipped_blocks: list[tuple[str, str, int]] = []
    with conn.cursor() as cur:
        cur.execute(_GROUPS_SQL)
        for pass_name, key, members in cur.fetchall():
            size = len(members)
            if size > max_block_size:
                skipped_blocks.append((pass_name, key, size))
            else:
                pairs.update(_pairs_from_members(members))
    return sorted(pairs), skipped_blocks
```

Ensure `import uuid` is present at the top of `db.py` (add it to the existing imports if missing).

- [ ] **Step 4: Run the full candidate-generation suite**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_candidate_generation.py -v`
Expected: PASS (9 passed) — the Task 1 cases still pass under the rewritten body, plus the two cap cases.

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/pipeline/db.py matcher/tests/test_candidate_generation.py
git commit -m "feat(matcher): oversized-block cap + skipped-block reporting (B2b)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: `sweep()` batch driver + result value types

The orchestration: generate candidates, close the read snapshot, loop `runner.propose()` per pair with skip-and-report error handling, and tally into a `SweepResult`.

**Files:**
- Create: `matcher/src/cairn_matcher/pipeline/sweep.py`
- Test: `matcher/tests/test_sweep.py` (create)

**Interfaces:**
- Consumes: `db.generate_candidate_pairs(conn, *, max_block_size)` (Task 2); `runner.propose(conn, a, b, *, thresholds, weights) -> Band | None` (existing); `Band` from `banding`; `Thresholds`/`DEFAULT_THRESHOLDS` from `banding`; `Weights`/`DEFAULT_WEIGHTS` from `scoring`.
- Produces:
  - `SkippedBlock(pass_name: str, key: str, size: int)` (frozen dataclass)
  - `SweepError(pair: tuple[str, str], message: str)` (frozen dataclass)
  - `SweepResult(generated: int, auto_candidate: int, review: int, below_threshold: int, skipped_blocks: list[SkippedBlock], errors: list[SweepError])` (frozen dataclass)
  - `sweep(conn, *, max_block_size: int = 100, thresholds: Thresholds = DEFAULT_THRESHOLDS, weights: Weights = DEFAULT_WEIGHTS) -> SweepResult`

- [ ] **Step 1: Write the failing tests**

Create `matcher/tests/test_sweep.py`:

```python
# matcher/tests/test_sweep.py
"""Integration tests for sweep(): generate candidates -> propose() each -> SweepResult.

Gated on CAIRN_TEST_PG (skips cleanly without a database).
"""

from cairn_matcher.pipeline.runner import canonical_pair
from tests.conftest import seed_patient

PA = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
PB = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
PC = "cccccccc-cccc-cccc-cccc-cccccccccccc"
PD = "dddddddd-dddd-dddd-dddd-dddddddddddd"


def _proposal_status(conn, low, high):
    with conn.cursor() as cur:
        cur.execute("SELECT status FROM match_proposal WHERE patient_low=%s AND patient_high=%s",
                    (low, high))
        row = cur.fetchone()
        return row[0] if row else None


def test_sweep_proposes_a_strong_candidate(pg_conn):
    from cairn_matcher.pipeline.sweep import sweep
    for p in (PA, PB):
        seed_patient(pg_conn, p, names=[("Alex Smith", 20)],
                     identifiers=[("mrn:a", "12345", "12345")])
    result = sweep(pg_conn)
    assert result.generated >= 1
    assert result.review >= 1                      # the strong pair lands in REVIEW
    low, high = canonical_pair(PA, PB)
    assert _proposal_status(pg_conn, low, high) == "pending"


def test_sweep_writes_nothing_for_a_no_signal_population(pg_conn):
    # No shared blocking key at all -> no candidates -> no proposals.
    seed_patient(pg_conn, PA, names=[("Alex Smith", 20)], identifiers=[("mrn:a", "1", "1")])
    seed_patient(pg_conn, PB, names=[("Robin Jones", 20)], identifiers=[("mrn:a", "2", "2")])
    result = sweep(pg_conn)
    assert result.generated == 0
    assert result.review == 0 and result.auto_candidate == 0


def test_sweep_is_idempotent_and_preserves_human_status(pg_conn):
    from cairn_matcher.pipeline.sweep import sweep
    for p in (PA, PB):
        seed_patient(pg_conn, p, names=[("Alex Smith", 20)],
                     identifiers=[("mrn:a", "12345", "12345")])
    sweep(pg_conn)
    low, high = canonical_pair(PA, PB)
    with pg_conn.cursor() as cur:                  # a reviewer accepts it
        cur.execute("UPDATE match_proposal SET status='accepted' WHERE patient_low=%s", (low,))
    pg_conn.commit()
    sweep(pg_conn)                                  # re-run
    assert _proposal_status(pg_conn, low, high) == "accepted"


def test_sweep_reports_oversized_blocks(pg_conn):
    from cairn_matcher.pipeline.sweep import sweep
    for p in (PA, PB, PC):
        seed_patient(pg_conn, p, dob=("1980-07-15", 20))
    result = sweep(pg_conn, max_block_size=2)
    assert any(sb.pass_name == "dob" and sb.size == 3 for sb in result.skipped_blocks)


def test_sweep_skips_and_reports_a_failing_pair(pg_conn, monkeypatch):
    # Two independent strong pairs. propose() is forced to raise on ONE; the sweep must
    # record the error, recover the connection, and still score the other pair.
    from cairn_matcher.pipeline import sweep as sweep_mod
    from cairn_matcher.pipeline.runner import propose as real_propose

    for p in (PA, PB):
        seed_patient(pg_conn, p, identifiers=[("mrn:a", "111", "111")])
    for p in (PC, PD):
        seed_patient(pg_conn, p, identifiers=[("mrn:b", "222", "222")])

    failing = canonical_pair(PA, PB)

    def flaky(conn, a, b, **kw):
        if canonical_pair(a, b) == failing:
            raise RuntimeError("boom")
        return real_propose(conn, a, b, **kw)

    monkeypatch.setattr(sweep_mod, "propose", flaky)
    result = sweep_mod.sweep(pg_conn)
    assert len(result.errors) == 1
    assert result.errors[0].pair == failing
    assert "boom" in result.errors[0].message
    # the other pair was still scored and persisted
    assert _proposal_status(pg_conn, *canonical_pair(PC, PD)) == "pending"
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_sweep.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'cairn_matcher.pipeline.sweep'`.

- [ ] **Step 3: Implement `sweep.py`**

Create `matcher/src/cairn_matcher/pipeline/sweep.py`:

```python
# matcher/src/cairn_matcher/pipeline/sweep.py
"""Batch driver (piece B2b): generate candidate pairs, score each via propose().

This is the front end B2 lacked — it decides WHICH pairs to score (db.generate_candidate_pairs,
the blocking passes) and feeds each through the existing pairwise propose(). Pure
orchestration over the db + runner seam; no scoring/banding logic lives here.

Two phases. Phase 1 generates the candidates, then closes the read snapshot BEFORE the
write loop so a long sweep does not pin the xmin horizon (the hazard runner.propose
already guards on its own sub-threshold path). Phase 2 loops propose() per pair: each is
its own transaction and idempotent (human status preserved on re-run), so the sweep is
resumable, and a failing pair is recorded and skipped (house rule #5) rather than aborting
the batch.

Requires the optional `pipeline` extra (psycopg) at CALL time, because it drives db/runner.
"""

from dataclasses import dataclass, field

from cairn_matcher.pipeline.banding import DEFAULT_THRESHOLDS, Band, Thresholds
from cairn_matcher.pipeline.runner import propose
from cairn_matcher.scoring import DEFAULT_WEIGHTS, Weights


@dataclass(frozen=True)
class SkippedBlock:
    """A blocking-value group excluded from pair generation for exceeding the cap."""

    pass_name: str   # 'identifier' | 'dob' | 'name'
    key: str         # the human-readable blocking value (system:match_key, dob, or token)
    size: int        # number of patients sharing it


@dataclass(frozen=True)
class SweepError:
    """One candidate pair whose propose() raised — recorded, never silently dropped."""

    pair: tuple[str, str]
    message: str


@dataclass(frozen=True)
class SweepResult:
    """Summary of one sweep: the observability surface and the 'log what was dropped' record."""

    generated: int                                   # candidate pairs scored
    auto_candidate: int                              # proposals written in the AUTO_CANDIDATE band
    review: int                                      # proposals written in the REVIEW band
    below_threshold: int                             # pairs that persisted nothing
    skipped_blocks: list[SkippedBlock] = field(default_factory=list)
    errors: list[SweepError] = field(default_factory=list)


def sweep(
    conn,
    *,
    max_block_size: int = 100,
    thresholds: Thresholds = DEFAULT_THRESHOLDS,
    weights: Weights = DEFAULT_WEIGHTS,
) -> SweepResult:
    """Score every blocking candidate pair and return a SweepResult summary.

    Generates candidates (closing the read snapshot before writing), then proposes on each
    surviving pair. A pair whose propose() raises is recorded in `errors` and skipped; the
    connection is rolled back so it stays usable for the next pair.
    """
    # Imported lazily so this module is importable without the optional `pipeline` extra;
    # only an actual sweep() call needs psycopg (mirrors runner.propose's lazy db import).
    from cairn_matcher.pipeline import db

    pairs, skipped_raw = db.generate_candidate_pairs(conn, max_block_size=max_block_size)
    # Close the read transaction the SELECTs opened before the per-pair write loop.
    conn.rollback()

    skipped_blocks = [SkippedBlock(*s) for s in skipped_raw]
    auto = review = below = 0
    errors: list[SweepError] = []
    for low, high in pairs:
        try:
            result = propose(conn, low, high, thresholds=thresholds, weights=weights)
        except Exception as exc:  # noqa: BLE001 — batch must survive one bad pair (house rule #5)
            # Clear the aborted transaction so the connection is usable for the next pair.
            conn.rollback()
            errors.append(SweepError((low, high), f"{type(exc).__name__}: {exc}"))
            continue
        if result is Band.AUTO_CANDIDATE:
            auto += 1
        elif result is Band.REVIEW:
            review += 1
        else:
            below += 1
    return SweepResult(
        generated=len(pairs),
        auto_candidate=auto,
        review=review,
        below_threshold=below,
        skipped_blocks=skipped_blocks,
        errors=errors,
    )
```

- [ ] **Step 4: Run the sweep tests to verify they pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_sweep.py -v`
Expected: PASS (5 passed).

- [ ] **Step 5: Run the full matcher suite (with and without DB)**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest -q`
Expected: PASS — all prior B1/B2 tests plus the new B2b tests green.

Run: `uv run pytest -q`
Expected: PASS — the pure suite green; all integration tests (B2 + B2b) skipped cleanly.

- [ ] **Step 6: Commit**

```bash
git add matcher/src/cairn_matcher/pipeline/sweep.py matcher/tests/test_sweep.py
git commit -m "feat(matcher): batch sweep driver over blocking candidates (B2b)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage:**
- Blocking 3 passes (identifier / exact-DOB / name-token) → Task 1. ✓
- Canonical uuid-order dedup, self/unknown exclusion → Task 1. ✓
- Oversized-block cap (default 100) + `skipped_blocks` reporting → Task 2. ✓
- Two-phase sweep, close snapshot before write loop → Task 3 (`sweep`). ✓
- Per-pair `propose()` reuse, idempotent / human-status preserved → Task 3 tests. ✓
- Skip-and-report error handling → Task 3 (`sweep` + `test_sweep_skips_and_reports_a_failing_pair`). ✓
- `SweepResult` observability summary → Task 3. ✓
- No DB-floor migration / no SCHEMA bump / advisory-only → no `db/` file touched in any task. ✓
- Tests integration-gated, skip cleanly → all tests use `pg_conn` fixture. ✓

**2. Placeholder scan:** none — every step carries real test code, real SQL, real implementation, exact commands.

**3. Type consistency:** `generate_candidate_pairs(conn, *, max_block_size)` returns `(list[tuple[str,str]], list[tuple[str,str,int]])` in both Task 1 and Task 2; `sweep.py` wraps the skipped tuples into `SkippedBlock(pass_name, key, size)` — order matches the SQL's `(pass_name, key, members→size)`. `canonical_pair` reused from `runner` in tests and matches the SQL `m1 < m2` uuid ordering. `Band.AUTO_CANDIDATE`/`REVIEW` match `banding.Band`. `_pairs_from_members` returns lowercase-uuid-text tuples consistent with `canonical_pair`'s `str(uuid.UUID(...))` output.

Note for the implementer: Task 2 **replaces** the Task 1 `_CANDIDATE_SQL` + `generate_candidate_pairs` body wholesale (group-based with the cap); the Task 1 tests must still pass against the rewritten body (they do — same blocking semantics, just cap-aware).
