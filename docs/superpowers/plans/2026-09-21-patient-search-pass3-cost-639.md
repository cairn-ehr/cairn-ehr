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

*(Task 5 fills this in.)*
