# Plan — pass 3's fixed per-row cost, and the ~1500 ms floor under it (#639)

**Issue:** [#639](https://github.com/cairn-ehr/cairn-ehr/issues/639) ·
**Slice 1 design:** `2026-09-21-patient-search-fragment-matching-design.md` ·
**Slice 1 plan:** `2026-09-21-patient-search-fragment-matching-636.md`

**No ADR. No spec bump. No migration, no `SCHEMA_GENERATION` bump, no signed-body change, no wire
change.** `db/046_patient_search.sql` is edited in place and replayed by `CREATE OR REPLACE` on every
connect, exactly as #638's fix was. **Nothing about WHICH candidates come back may change** — that is
the whole claim of this slice, and the first task is the test that would catch it if it did.

## What is wrong

#636 slice 1 widened pass 3 (stored token PARTS, and a 3+ byte PREFIX arm) and made the search
**5.6× slower**: a 200k-row `patient_name`, single query token `mich`, went from **411 ms** to
**2301 ms** — *with the prefix arm never firing once*.

The Pi-class measurement that followed found the sharper version of the same thing: on a Raspberry
Pi 5 at the §8.1 population of 50,000 patients, **every search costs ~1500 ms, including one that
finds nothing**, and the whole spread from that floor to the worst case is only ~1000 ms. The 5 s
§1.2 ceiling for *find an existing chart* is met at 2413 ms worst case; **§5.11's other limb —
"type a few chars and enter, no spinner" — is not.** A re-run against a real Australian name
distribution (50,378 rows, 965,260 distinct surnames) moved every timing by under 5%, so the floor
is a property of the scan and not of the fixture.

**Failure scenario, which is why this is not a tuning nicety:** slice 2's funnel UI re-searches in
the background as the clerk types. At a 1.5 s floor per keystroke-driven search, a clerk typing a
hyphenated surname on a facility Pi waits, abandons the search, and creates the duplicate chart the
whole search-before-create funnel (ADR-0061) exists to prevent.

## The diagnosis, and the one #637 got wrong

#637 blamed the prefix arm: *"`starts_with` runs against ~1.2 million generated tokens regardless of
match count, and costs more per token for a longer query string."* **That is false, and it matters,
because #637's remedy follows from it** — a materialised token table with its own reprojection cost,
a heavy fix aimed at the wrong place. The regression reproduces with `starts_with` never matching,
and `starts_with` short-circuits on first byte mismatch, so a *longer* prefix is cheaper to reject.

The measured drivers are fixed per-row work that no query can avoid:

1. **1a's second `regexp_split_to_table`, plus the lateral's `UNION` dedup sort**, run for every
   `patient_name` row. This is the bulk.
2. **`lower(normalize(t, NFC))` evaluated three times per (stored token × query token) pair** on the
   non-matching path — once for the equality, once for `octet_length`, once for `starts_with` — and
   `normalize`'s cost scales with string length. *This* is why the long compound name looked like a
   prefix-arm problem.

## The three changes, each already measured

Same 200k table, long single token `fyodorowksi-eschenbacher`, **3121 ms** baseline:

| change | result |
|---|---|
| hoist the query-token normalisation behind an `OFFSET 0` optimisation fence | 1896 ms |
| `UNION ALL` in the lateral instead of `UNION` | −30% on its own |
| skip the parts branch when the value carries no punctuation | — |
| **all three** | **637 ms (−80%)** |

**A plain subquery does not work for the first** — the planner pulls it back up and re-inlines it,
and that dead end was measured too. The `OFFSET 0` fence is required.

### Why each is semantically neutral — the argument a reviewer must check

- **The fence** changes only *how often* `lower(normalize(t, NFC))` is evaluated, never its value.
  `normalize` and `lower` are IMMUTABLE, so one evaluation per query token is the same string as one
  per pair.
- **`UNION ALL`** lets one `patient_name` row yield the same token twice (an unpunctuated single word
  is both a whole token and its own alphanumeric part). Those duplicates reach the branch's own
  `SELECT DISTINCT pn.patient_id, 'name'::text`, which collapses them. db/046's dedup block already
  argues that the outer `UNION` alone would suffice and the per-branch `DISTINCT` alone would
  suffice; the lateral's `UNION` was the third layer and is the only one being spent.
- **Skipping the parts branch on an unpunctuated value** is a subset argument, and it turns on the
  test being applied to *the string the splitter actually sees*: `normalize(pn.value, NFC)`. If that
  string contains nothing outside `[[:alnum:][:space:]]`, then splitting it on `[^[:alnum:]]+`
  (parts) and on `\s+` (whole) produce the *same* token list — Postgres's `\s` is exactly
  `[[:space:]]` — except that parts additionally drops length-1 tokens and excludes callsigns. So
  parts ⊆ whole for such a value, and the branch contributes nothing. Testing the *raw* value
  instead would be merely conservative rather than exact: a decomposed `e`+U+0301 reads as
  non-alnum before NFC and alnum after.

## Tasks

Each task is a commit. TDD: the guard tests in task 1 go in **first** and must pass against the
**shipped** SQL before any of the three changes land — a test that cannot fail before the change is
not evidence.

### Task 1 — the equivalence guard (test only, green on shipped code)

`crates/cairn-node/tests/patient_search_equivalence.rs`, DB-gated. Seed one corpus that exercises
every path the three changes touch, then assert the candidate set for a table of query tokens is
**exactly** what pass 3 returns today:

- an unpunctuated multi-word name (the skipped-parts case), including a **length-1 word** (`A`) that
  the parts branch would have dropped and the whole-token branch keeps;
- a punctuated compound (`Fyodorowksi-Eschenbacher`) and a comma-form (`Smith, John`) — the parts
  branch must still run for these;
- a decomposed-Unicode value (the NFC path) and a CJK value (the byte-gate path, #638);
- a callsign row (both callsign guards must survive all three changes);
- a value with leading/trailing whitespace (the `tok <> ''` guards).

The assertion is **set equality on `(patient_id, matched_pass)`**, per query token, both directions —
nothing lost *and* nothing gained. The slice-1 reviewer's `EXCEPT`-both-ways method, made a standing
test rather than a one-off.

Run it red by mutation before trusting it: temporarily delete the parts branch and confirm the
punctuated cases fail.

### Task 2 — hoist the query-token normalisation behind an `OFFSET 0` fence

`db/046`: replace the bare `unnest(...)` with a fenced subquery projecting `lower(normalize(t, NFC))
AS qt`, and use `q.qt` at all three sites. The comment must say *why* `OFFSET 0` is there, or the
next reader deletes it as noise — including that a plain subquery was measured and does not work.

### Task 3 — `UNION ALL` in the lateral

`db/046`. Update the file's "DELIBERATELY REDUNDANT DEDUPLICATION" block: its `#636` update
paragraph currently says the lateral's `UNION` removes a within-pass-3 duplicate as "a third dedup
layer". That sentence becomes false with this change and must be rewritten to say the duplicate now
reaches the branch `DISTINCT`, which is where it was always also caught.

### Task 4 — skip the parts branch when the value carries no punctuation

`db/046`, with the subset argument above written into the comment.

### Task 5 — measure, on the same rig as slice 1

Re-run the exact five queries of the slice-1 Pi run against the same Raspberry Pi 5 at the same
50,378-row real-name population, and record before/after in this plan. **If the Pi is unreachable,
measure on the Mac against the same corpus, say so in as many words, and leave the Pi row owed** —
a measurement on the wrong hardware reported as the budget is the error #637 made and #637's own
correction names.

### Task 6 — docs currency

HANDOVER, ROADMAP, and the slice-1 plan's measurement section (which points forward at #639).

## Paper-parity benchmark (§1.2)

**Paper counterpart:** the alphabetical index drawer at the registration desk — the clerk thumbs to
the surname and reads the cards under it. Unchanged from slice 1; this slice does not add or remove
a human act, it changes what the machine costs while the clerk waits.

**Steps:** paper 3 → architecture-forced 2 → UI bundling target 2 (slice 2). `M ≤ N`, unchanged by
this slice — no step is added or removed, because nothing about the returned candidate set changes.

**Time + cognitive load:** the budget is the **5 s to find an existing chart** already stated in
`db/046`, which slice 1 measured as met (2413 ms worst case on a Pi 5 at 50,378 rows). The budget
this slice owes is the *other* limb, **§5.11's "type a few chars and enter, no spinner"**: the
measured **~1500 ms floor on every search, including one that finds nothing**, is the number to
move. Target from the issue's measurement: **−80% of the regression**, i.e. a floor materially under
1 s on Pi-class hardware. Measured by Task 5 on the same rig and population as slice 1, recorded
below; a measurement on faster hardware does not discharge it. Cognitive load is untouched.

## Measurement results

Rig: `scripts/measure_patient_search.py` (new with this slice — slice 1's equivalent was never
committed, so its Pi numbers could be believed but not re-derived). Pure parts tested by
`scripts/tests/measure_patient_search_test.py`, wired into `rust.yml`.

### On the target hardware, which is the only run that judges the budget

Raspberry Pi 5, aarch64, 4 cores, 8 GB, PostgreSQL 18.4, `cairn_pgx` 0.3.0, reached by
`ssh -J dgx`. **50,000 `patient_name` rows drawn from the maintainer's real Australian name pool**
(the same source slice 1 used), all 53 migrations loaded on ARM. Median of 5 runs after a warm-up.
**Before and after differ in `db/046` and nothing else** — same machine, same database, same seeded
rows, the function replaced in place between the two runs.

| Search | Before (`main`) | After | Change |
|---|---|---|---|
| `fyodorowksi-eschenbacher` — long compound | 2509.5 ms | **862.4 ms** | **−66%** |
| `mich` — selective fragment | 1669.7 ms | **870.9 ms** | −48% |
| `smi` — unselective fragment | 1630.9 ms | **871.1 ms** | −47% |
| `李小` — CJK 2-character prefix | 1607.0 ms | **866.6 ms** | −46% |
| `wu` — exact short, below the byte gate | 1528.8 ms | **856.7 ms** | −44% |

**The floor — the number §5.11 is about — falls from 1528.8 ms to 856.7 ms, under 1 s.** The worst
case falls from 2509.5 ms to 862.4 ms against a 5000 ms budget.

**The spread collapses from 981 ms to 14 ms, and that is the finding, not a footnote.** Before, a
search cost measurably more for a longer query string; after, every gesture costs the same. That is
the signature of per-pair `normalize` work being hoisted out, and it is #639's diagnosis confirmed
against #637's: had the prefix arm been the driver, the long compound would still stand out.

The `main` column reproduces slice 1's hand-run to within 4% (`wu` 1528.8 vs 1479, the compound
2509.5 vs 2413), which is what licenses comparing the two runs at all.

**Honest limits.** Rows are seeded straight into the `patient_name` projection, not authored as
signed events — the same caveat slice 1 recorded; `cairn_search_candidates` reads nothing else, so
this is the intended read path, and it says nothing about registration throughput. Each timing
includes one `psql` process start, measured on that Pi at **31.9 ms**, ~4% of the post-change
figure and ~2% of the pre-change one.

### The neutrality claim, checked at 50k scale rather than argued

The shipped function was installed a second time under a second name and the two were compared
directly over the full seeded corpus — **394 query tokens** (25 hand-chosen gestures, ~300 real
surnames straight out of the data, and ~100 three-byte prefixes of them), **14,447 candidate rows**:

```
LOST   (in old, not new): 0
GAINED (in new, not old): 0
```

This is the reviewer's `EXCEPT`-both-ways method from slice 1, at 15× the token count, and it is
evidence the standing test in `patient_search_equivalence.rs` cannot give on its own — that suite
pins the CONTRACT on a seven-chart corpus, this compares the two implementations on a real one.
Neither replaces the other: the differential dies with the branch, the contract test outlives it.

### Dev-hardware run, for the record

Apple Silicon M3 Max, Postgres 18 on :5532, the same 50,000 real names: floor **560.4 → 174.1 ms**,
worst case **836.5 → 175.3 ms**. A larger proportional gain than the Pi's, which is why the Pi run
is the one quoted against the budget.

### What is left, and what would be needed to move it

The remaining ~860 ms is the whole-token `regexp_split_to_table` over every `patient_name` row —
the scan pass 3 has always been, and the one cost none of these three changes touches. Cutting it
needs a materialised token table with its own reprojection cost, which is what
[#637](https://github.com/cairn-ehr/cairn-ehr/issues/637) proposed for the wrong reason; it is now
the right remaining candidate, on the right diagnosis, and it is a slice of its own.

### Filed from this slice

[#641](https://github.com/cairn-ehr/cairn-ehr/issues/641) — found while checking task 4's subset
argument against the server rather than against the documentation: `[^[:alnum:]]+` treats a Unicode
combining mark as a separator, so slice 1a's parts branch cuts a Devanagari name at its first vowel
sign (`अमित` → `अम`) and a Thai name at its tone marks. Nothing becomes unfindable — the whole-token
source keeps the name intact — so it is precision, not recall, and it does not block this slice.
But it is ADR-0014's cultural-capture shape one level below #638: that was the *gate* encoding a
Latin selectivity model, this is the *separator class* encoding a Latin orthographic one. Fixing it
changes which candidates come back, which is exactly what this slice claims not to do.
