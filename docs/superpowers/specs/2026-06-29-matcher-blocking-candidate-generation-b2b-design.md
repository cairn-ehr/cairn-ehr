# Matcher piece B2b — blocking / candidate-pair generation + batch sweep

**Date:** 2026-06-29 · **Status:** design approved, pre-implementation
**Spec home:** §5.2 (matching pipeline — blocking tier) / §5.13 (locale-pluggable comparators) ·
**ADR:** implements settled [ADR-0014](../../spec/decisions/0014-locale-pluggable-matcher-comparators.md);
**no new ADR, no spec-version bump** (same posture as B1/B2 — this is advisory, fit-for-purpose code).

## Problem

The B2 pipeline (`cairn_matcher/pipeline/`) scores **one given pair**: `propose(conn, a, b)`
loads two patients' projection rows, scores them (B1), consults the in-DB hard-veto floor
(`db/016`), bands the result, and upserts an advisory proposal (`db/017`). It has no way to
decide *which* pairs across the patient set are worth scoring — today "an external driver must
supply the pairs." Scoring all C(n,2) pairs is intractable and pointless: the vast majority
share no signal.

**B2b** is the missing front end: **blocking** (generate the small set of candidate pairs that
share a blocking key) plus a **batch sweep** that feeds each candidate through the existing
`propose()`.

## Governing constraints

- **Advisory, not safety-critical (§9 defect-blast-radius rule).** A missed candidate is a false
  split, which the §5.13 **hub duplicate-sweep is the declared backstop for**; an extra candidate
  just costs CPU — the scorer + veto still gate every pair. So the recall/cost tradeoff here is
  **pure performance, never safety**. This is why blocking is Python + an inline query, **not** a
  versioned DB-floor function: keeping advisory logic out of the floor stops deployment policy from
  forking the safety surface.
- **Safety asymmetry unchanged.** Blocking only *proposes* pairs to score. Auto-link semantics are
  unbuilt (piece C, needs the §5.7 identity event algebra). The deterministic tier's *auto-link* is
  out of scope; its *blocking key* (exact identifier) is simply one of the three passes below.
- **House rules:** TDD; pure functions where practical; inline docs for a junior dev; files < 500
  lines; AGPL-3.0; fix-or-file on review findings.

## Scope

In: candidate-pair **generator** + batch **sweep driver** (option B from brainstorming).
Out: a CLI (function-only, option A); compound blocking keys (B3); deterministic-tier auto-link
(piece C); the hub-tier aggressive sweep (B3); a `compare_address` comparator (B3).

## Components

Both live in the existing `matcher/src/cairn_matcher/pipeline/` package (beside B2). No new SQL
file, no SCHEMA-array change.

### `db.generate_candidate_pairs(conn, *, max_block_size) -> (pairs, skipped_blocks)`

The one new query, in `pipeline/db.py` (the matcher's only IO module). Read-only; the caller owns
the snapshot. A disjunction (`UNION`) of three blocking passes over the existing projections,
deduped to one canonical row per pair:

1. **Identifier pass** — patients sharing a `(system, match_key)` in `patient_identifier`,
   excluding `system = 'unknown'`. Smallest blocks, highest precision; this is the §5.2
   deterministic accelerator used purely as a blocking key here.
2. **Exact-DOB pass** — patients sharing a `patient_demographic` row with `field='dob'` and the
   same `value` (the ISO string). Medium precision; the scorer + other fields discriminate within.
3. **Name-token pass** — patients sharing a token from
   `regexp_split_to_table(lower(value), '\s+')` over `patient_name`. Highest recall (reordered /
   changed names, typos elsewhere), lowest precision — and the reason the probabilistic tier earns
   its keep (without it, fuzzy comparators only ever fire on pairs that already share an exact DOB
   or identifier). The SQL tokenizer is deliberately simple and **recall-oriented**; the Python
   adapter/scorer remains the source of truth for comparison — blocking need not agree token-exact.

**Canonical ordering & dedup.** Each pass emits `LEAST(a,b), GREATEST(a,b)` on the patient
**uuid** values (128-bit integer order = Postgres byte order = `runner.canonical_pair`'s order,
*not* text order — upper/mixed-case UUIDs would diverge), with `a <> b`; the `UNION` is
`DISTINCT`, so a pair blocking on several keys is yielded once.

**Oversized-block guard (no silent caps).** Each pass groups by its blocking value
(`HAVING count(DISTINCT patient_id) >= 2`). A group with `count > max_block_size` is **excluded
from pair generation and returned in `skipped_blocks`** — `(pass_name, key, size)`. Rationale: a
block of size *k* contributes *C(k,2)* pairs; a blocking value shared by hundreds of people is
non-discriminating, and the hub sweep is the honest backstop for whatever it drops.

**Cap default = 100, configurable.** *C(100,2) ≈ 5 000* pairs/block. Conservative but tunable;
even 100 can aggregate to a lot across many blocks — the real lever to shrink blocks is
**compound keys** (e.g. token + birth-year), deliberately deferred to B3 where the eval harness
can *measure* whether they are needed (YAGNI for v1).

Implementation note: returning both pairs and the skipped report is naturally two read queries
(one for oversized groups, one for the in-cap pairs) sharing the cap parameter — an implementation
detail for the plan, not a contract.

### `sweep.sweep(conn, *, max_block_size=100, thresholds=DEFAULT_THRESHOLDS, weights=DEFAULT_WEIGHTS) -> SweepResult`

New module `pipeline/sweep.py`. Pure orchestration over the existing `db` + `runner` seam — no new
combinatorial logic outside the SQL.

- **Phase 1 — generate & close.** Call `generate_candidate_pairs(...)`, materialize the pairs into
  a Python list, then `conn.rollback()` to close the read snapshot **before** the write loop, so a
  long sweep does not pin the xmin horizon (the same hazard `runner.propose` already guards on its
  sub-threshold path).
- **Phase 2 — score each.** Loop the materialized pairs, calling `runner.propose(conn, low, high,
  thresholds=…, weights=…)` per pair. Each `propose()` is its own transaction and is idempotent
  (human `status` preserved on re-run), so the sweep is **resumable** — a crash mid-sweep leaves
  committed proposals intact and a re-run refreshes scores without clobbering human decisions.
- **Skip-and-report errors (house rule #5).** Wrap each `propose()` in `try/except`. On failure:
  record `(pair, message)`, `conn.rollback()` to clear the aborted transaction so the connection
  stays usable, and continue. One malformed projection row never kills the batch; nothing fails
  silently.
- **Tally** into `SweepResult`.

### Value types (`sweep.py`)

Frozen dataclasses, pure:

- `SkippedBlock(pass_name: str, key: str, size: int)`
- `SweepError(pair: tuple[str, str], message: str)`
- `SweepResult(generated: int, auto_candidate: int, review: int, below_threshold: int,
  skipped_blocks: list[SkippedBlock], errors: list[SweepError])`

`generated` = pairs scored; `auto_candidate`/`review` = proposals written by band;
`below_threshold` = pairs that persisted nothing. The result is the observability surface and the
"log what was dropped" record.

## Data flow

```
patient_identifier / patient_demographic / patient_name
        │  (db.generate_candidate_pairs: 3 blocking passes, oversized excluded + reported)
        ▼
list[(low, high)]  +  skipped_blocks[]
        │  (sweep phase 1: materialize, then conn.rollback to close the read snapshot)
        ▼
for each pair:  runner.propose()  →  load → score(B1) → cairn_match_veto(db/016) → band → upsert(db/017)+commit
        │  (try/except: on error → rollback + record, continue)
        ▼
SweepResult (counts by band, skipped blocks, per-pair errors)
```

## Testing (TDD)

Integration-gated on `CAIRN_TEST_PG` (PG18 + cairn_pgx), run with `--extra pipeline`; they seed
`patient_*` projection rows directly, then assert on candidate generation and the written
proposals. Skip cleanly without `CAIRN_TEST_PG`.

**`generate_candidate_pairs`:**
- Two patients sharing an identifier `(system, match_key)` → the pair is generated.
- Two patients sharing only a name token → generated.
- Two patients sharing nothing → **not** generated.
- A pair sharing both an identifier and an exact DOB → generated **once** (dedup).
- Self-pairs excluded; `system='unknown'` never blocks.
- An oversized block (set a small cap, seed cap+1 patients sharing one DOB) → reported in
  `skipped_blocks`, generates **no** pairs from that block.
- Pairs are canonical-ordered regardless of input UUID case.

**`sweep`:**
- A strong pair → proposal written; `SweepResult` band counts reflect it.
- A no-signal pair → no proposal; `below_threshold` (or simply absent from generation).
- Re-run → idempotent; a pre-set human `status` on an existing proposal is preserved.
- A malformed projection row on one pair → `errors` records it, the sweep **continues**, other
  pairs still scored (connection still usable afterwards).

## Files & sizing

- `pipeline/db.py` ~75 → ~115 lines (one new function + its query helpers).
- `pipeline/sweep.py` new, ~100 lines.
- Tests: extend `matcher/tests/` (a `test_sweep.py` + candidate-generation cases). All well
  under 500 lines.

## Out of scope / deferred (recorded, not lost)

- **Compound blocking keys** (token + birth-year, etc.) to shrink blocks — B3, measurement-driven.
- **CLI entry point** for operational runs — modest later add once `sweep()` exists and is tested.
- **Hub-tier aggressive duplicate sweep** + proposal retraction — B3.
- **Deterministic-tier auto-link** and the proposal→`link` apply seam — piece C (needs §5.7).
- **Server-side cursor / streaming** of a huge candidate list — YAGNI; the cap bounds it for v1.
