# Matcher pipeline — piece B2 (pairwise, veto-gated, proposal-persisting)

**Date:** 2026-06-29 · **Status:** design approved, pre-implementation · **Spec home:** §5.2 / §5.13 /
§4.4 · **ADRs:** implements settled [ADR-0014](../../spec/decisions/0014-locale-pluggable-matcher-comparators.md)
(advisory matcher, config-pinned actor) — **no new ADR, no spec-version bump.**

## 0. Where this sits

The §5.2 advisory matcher is built in pieces:

| Piece | What | Tier | State |
|---|---|---|---|
| A | In-DB hard-veto + coherence floor (`db/016`) | In-DB, safety-critical | **done** |
| B1 | Pure scoring core (`cairn_matcher`, zero-dep) | Python, advisory | **done** |
| **B2** | **Data adapter + veto-gated pairwise pipeline + proposal worklist** | **Python, advisory** | **this slice** |
| B2b | Blocking / candidate-pair generation across the whole patient set | Python, advisory | deferred |
| B3 | Locale comparator packs · weight-learning · eval harness · hub duplicate-sweep | Python, advisory | deferred |
| C | Proposal → §5.7 `link` apply seam | In-DB, safety-critical | deferred (identity algebra unbuilt) |

B2 is the seam that connects B1's pure scorer to real node data and produces the durable advisory
output piece C will one day consume. It is **advisory** (§9 fit-for-purpose): a defect yields a bad
*proposal a human reviews*, never a silent record corruption. The only safety-critical part — the
hard-veto floor — already exists in `db/016`; B2 **calls** it and never re-implements it.

**Scope decision (this slice = pairwise):** B2 scores a *given patient pair*. Finding which pairs to
score (blocking) is B2b. This keeps the slice a complete, fully-testable vertical without a tuning pass.

## 1. Architecture & module layout

A new IO-bearing sub-package, kept strictly separate from B1's pure core (which is untouched and stays
zero-dependency):

```
matcher/src/cairn_matcher/
  agreement.py  comparators.py  records.py  orchestrator.py  scoring.py   # B1, unchanged
  pipeline/
    __init__.py
    adapter.py   # PURE: projection row dicts -> CandidateRecord  (no psycopg import)
    banding.py   # PURE: (MatchScore, veto findings, thresholds) -> Band | None + proposal payload
    db.py        # IO: psycopg queries — load a patient's projection rows, call cairn_match_veto, upsert proposal
    runner.py    # orchestrates: (conn, patient_a, patient_b) -> load -> adapt -> score -> veto -> band -> persist
```

- **`psycopg` is an optional extra, not a core dependency:** `[project.optional-dependencies] pipeline =
  ["psycopg>=3"]`. B1's "zero runtime deps / pure" claim stays literally true; only `db.py`/`runner.py`
  import it. **psycopg3 is LGPL-3.0-or-later → AGPL-3.0-compatible** (license-checked, house rule #1).
  This is the *only* runtime dependency the whole matcher introduces.
- The pure halves (`adapter.py`, `banding.py`) carry the bulk of the logic and are unit-tested with no
  database. `db.py` is a thin SQL shim; `runner.py` is the only place IO and pure logic meet.
- **Portability note:** because `adapter.py` + `banding.py` are pure, an eventual decision to move the
  adapter in-DB (SQL/pgrx) would replace only `db.py`/`runner.py`. This slice does not lock that in.

## 2. The adapter (`adapter.py`, pure)

Pure functions mapping projection rows (passed in as plain dicts/sequences — no DB coupling) into B1's
`CandidateRecord`. Every field degrades safely (principle 4 — absence is never disagreement); a
structurally wrong row raises `MatcherTypeError` (house rule #5 — an adapter bug, surfaced loudly).

| Field | Source projection | Mapping | Degrade rule |
|---|---|---|---|
| `dob` | `patient_demographic` (`field='dob'`) | precision-gated ISO extraction driven by `facets.precision` (`year`→`DateValue(year)`, `month`→`+month`, `day`→`+day`); `provenance_rank` carried into `FieldValue` | value not parseable as ISO at the stated precision → **omit** (→ `INSUFFICIENT_DATA`), never a wrong `DateValue` |
| `sex_at_birth` | `patient_demographic` (`field='sex-at-birth'`) | `FieldValue(value=str, provenance_rank)` verbatim (compared by `compare_exact`) | absent row → `None` |
| `names` | every `patient_name` row | one `Name` per row, tokens = `{"unspecified": tuple(value.split())}` (untagged bag — `patient_name` projects no role structure), collected into `frozenset[Name]`; provenance = **max** `provenance_rank` across the rows | empty name set → `None` |
| `identifiers` | `patient_identifier` rows | `Mapping[system, frozenset[match_key]]`, skipping `system='unknown'`; uses `match_key` (= `coalesce(normalized, value)`) to align with the veto floor's key | no rows → empty mapping |

**Why ISO extraction is not "locale date parsing":** the DOB `value` is ISO-8601 by the `cairn-event`
write convention (`1980-07-15` for day precision, `1980` for year), and `facets.precision` declares the
precision. The adapter extracts fields at the declared precision; it does not interpret a free-form
locale date string (that remains a B3/locale-pack concern). A non-ISO value from a non-conformant peer
degrades to absence, never an error or a guess.

**Untagged-name-bag rationale:** `patient_name` retains only the opaque authored display string; the
given/family structure is not projected. A single `"unspecified"` role bag is culture-neutral and needs
no schema change; `compare_name_set` compares bags per role, so a shared single role reduces cleanly to
a whole-string token-bag comparison. Projecting structured role tokens is a future refinement (B2b/B3),
explicitly not done here. **The adapter reads projections only — never the event body** (the B1 contract).

## 3. The pipeline: score → veto → band → persist

### 3.1 `db.py` (IO, thin)

- `load_candidate(conn, patient_id) -> CandidateRecord` — runs the four projection SELECTs, hands rows to
  `adapter.py`. (`cairn_agent` needs `SELECT` on the `patient_*` projections — a grant added in db/017.)
- `match_veto(conn, a, b) -> list[VetoFinding]` — calls in-DB `cairn_match_veto(a,b)`, returning rows
  verbatim (`veto_kind, severity, subject, detail`). B2 never re-implements the floor.
- `upsert_proposal(conn, proposal) -> None` — writes one `match_proposal` row (§4).

### 3.2 `banding.py` (pure)

`band(score: MatchScore, vetoes: list[VetoFinding], thresholds: Thresholds) -> Band | None`. Two shipped
conservative thresholds (`T_review < T_auto`; **illustrative/provisional — B3 learns real ones from local
adjudication data**). Logic, in priority order:

| Condition | Band |
|---|---|
| `total >= T_auto` **and** zero veto findings (any severity) | `auto_candidate` |
| `total >= T_review` (including a high score capped by **any** veto — hard_veto *or* degrade_hold) | `review` |
| `total < T_review` | **`None`** → no proposal persisted |

This honours `db/016` exactly: a veto **never auto-links and never auto-rejects** — any finding
(hard_veto *or* degrade_hold) caps the band at `review`; it never suppresses a real signal down to
no-match. Below `T_review` nothing is persisted (the noise floor).

**Declared backstop (not silently dropped):** signal below `T_review`, and false splits generally, are
the job of the **B3 hub duplicate-sweep**, not this pairwise slice. The runner documents/logs this so
"persist nothing" never reads as "covered everything."

**Threshold caveats (carried from the design dialogue):**
1. `provenance_factor` has a 0.5 floor, so every field is at least halved at unknown provenance. Max
   single-field contributions at rank 0 are modest (identifier 4.0, dob-exact 3.0, name-exact 2.5).
   `T_review`/`T_auto` must be chosen with that floor in mind.
2. B1 hardcodes identifier `provenance_rank = 0` (`orchestrator._identifiers`), so even a shared
   same-system identifier scores 4.0 → lands in `review`, not `auto_candidate`. Safe (a human confirms)
   but timid; left as-is for B2 and flagged for B3's weight-learning rather than patched mid-slice.

### 3.3 `runner.py`

`propose(conn, a, b) -> Band | None` — sort the pair, `load_candidate` both sides, `field_comparisons`
→ `score` (B1), `match_veto`, `band`; on a non-`None` band, `upsert_proposal`. **One transaction**: load,
veto, and upsert commit together. If `cairn_match_veto` errors, the transaction aborts and nothing is
written — a proposal whose veto status is unknown is never persisted.

## 4. The proposal worklist table (`db/017_match_proposal.sql`, SCHEMA-tracked)

A new advisory projection added to the `cairn-node` SCHEMA array (15→16). **Advisory infrastructure, not
a safety gate** — no validation door, no `submit_event` involvement. It is B2's durable output and the
input contract for piece C (deferred §5.7 link-apply seam) and any review UI.

```sql
CREATE TABLE IF NOT EXISTS match_proposal (
    patient_low        UUID    NOT NULL,   -- least(a,b)   } pair stored in canonical
    patient_high       UUID    NOT NULL,   -- greatest(a,b) } order => symmetric, unique
    score_total        DOUBLE PRECISION NOT NULL,
    band               TEXT    NOT NULL,   -- 'auto_candidate' | 'review'
    veto_findings      JSONB   NOT NULL,   -- the cairn_match_veto rows, verbatim (explainability)
    evidence           JSONB   NOT NULL,   -- the per-field MatchScore breakdown
    matcher_version    TEXT    NOT NULL,   -- cairn_matcher __version__ + config digest (ADR-0014 pinning)
    status             TEXT    NOT NULL DEFAULT 'pending',  -- human disposition: pending|accepted|rejected|deferred
    created_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (patient_low, patient_high),
    CHECK (patient_low < patient_high)
);
GRANT SELECT, INSERT, UPDATE ON match_proposal TO cairn_agent;
-- plus: GRANT SELECT on patient_identifier / patient_demographic / patient_name to cairn_agent (read side)
```

Design points:
- **Canonical pair order** (`patient_low < patient_high`, CHECK-enforced) makes the pair the natural key
  and the table symmetric — `propose(a,b)` and `propose(b,a)` touch one row, mirroring db/016's symmetry.
  The runner sorts the pair before writing.
- **Upsert, latest-wins** (`ON CONFLICT (patient_low, patient_high) DO UPDATE`): a re-run refreshes
  score/band/evidence/`matcher_version`/`updated_at`, **but preserves a non-`pending` `status`** (never
  clobbers a human's accept/reject). A live review queue, not an event log; per-run history is not kept
  here (the matcher is advisory and re-runnable).
- **`matcher_version`** = `cairn_matcher.__version__` + a digest of the active weights+comparator config
  — the lightweight slice of ADR-0014's "config version-pinned actor." Full §7.5 actor
  registration/signing of the matcher is deferred to B3 (noted, not dropped).
- **`veto_findings` + `evidence` as JSONB** keep every proposal self-explaining (why this band, what
  agreed, what vetoed) without re-derivation — the explainability the matcher is built around.

**Caveat:** latest-wins means a pair that drops *below* `T_review` on a later run (corrected data) does
not auto-retract an existing `pending` proposal via `propose`. Fine for the pairwise slice (you only
score pairs you choose); sweep-driven reconciliation/retraction is a B3/sweep concern — documented, not
half-built.

## 5. Error handling

- **Absent / unparseable data degrades, never errors** — missing field, non-ISO DOB, empty name set →
  `INSUFFICIENT_DATA` → 0 contribution. The pipeline always produces a verdict.
- **Structural type errors raise** `MatcherTypeError` — a wrong-shape projection row is an adapter bug.
- **The veto call is mandatory and never bypassed** — a `cairn_match_veto` error aborts the whole
  `propose` transaction; no proposal with unknown veto status is ever persisted.
- **One transaction per `propose`** — load → veto → upsert commit together; a crash leaves no
  half-written proposal.

## 6. Testing (TDD, failing-test-first)

**Pure unit tests, no DB** (the bulk):
- `adapter.py` — DOB precision-gated ISO parse + non-ISO degrade; untagged name bag; identifier grouping
  (skip `unknown`, use `match_key`); provenance passthrough (incl. max-over-names); `MatcherTypeError` on
  a malformed row.
- `banding.py` — full band table; veto caps at `review` for **both** hard_veto and degrade_hold;
  sub-threshold → `None`; pair-ordering symmetry.

**Integration tests, gated** (PG18 + cairn_pgx, `CAIRN_TEST_PG`, skipped when unset — same discipline as
the Rust DB tests):
- seed two patients via the real `submit_event`; `propose` end-to-end; assert the persisted
  `match_proposal` row (band / score / veto_findings / evidence);
- veto path: verified-DOB clash → high score **capped to `review`, never `auto`**;
- no-signal path → **no row**;
- upsert latest-wins **preserving** a human `status`.

Run: `cd matcher && uv run pytest` (pure suite); the integration suite needs the `pipeline` extra +
`CAIRN_TEST_PG`.

## 7. Out of scope (recorded, not lost)

- **B2b:** blocking / candidate-pair generation across the whole patient set; optionally projecting
  structured role-tagged name tokens.
- **B3:** locale comparator packs (phonetic/nickname + content-addressed profiles); weight-learning
  (incl. revisiting identifier provenance and the thresholds); eval harness; hub duplicate-sweep
  (the false-split backstop + sweep-driven proposal reconciliation/retraction); full §7.5 matcher actor
  registration/signing.
- **Piece C:** the proposal → `link` apply seam (needs the §5.7 identity event algebra, unbuilt).
- A `compare_address` comparator (B1 follow-up).

## 8. Footprint

- **Spec/ADR:** none — implements settled §5.2/§5.13/ADR-0014. **No new ADR, no spec-version bump.**
- **DB:** new `db/017_match_proposal.sql`; `cairn-node` SCHEMA array 15→16; grants to `cairn_agent`.
- **Python:** new `cairn_matcher/pipeline/` sub-package; one optional dependency (`psycopg`, LGPL→AGPL-ok).
- **No change** to B1's pure core, to `submit_event`, to any event format, or to existing projections.
