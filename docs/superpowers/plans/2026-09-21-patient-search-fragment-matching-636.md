# Patient search matches fragments, not only whole tokens — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A clerk can find a chart by typing part of a name — `Eschenbacher` finds `Fyodorowksi-Eschenbacher`, `mich` finds `Michaelowski` — without ever making a short name unfindable.

**Architecture:** Two widenings of `cairn_search_candidates`' pass 3, both additive and both in the safe direction (search finds strictly more). 1a projects the alphanumeric *parts* of stored punctuated tokens, mirroring what `SearchQuery::new` already does on the query side, with callsign-derived names excluded. 1b widens the join predicate from exact equality to exact-or-prefix with a 3-character minimum. No new table, no new index, no schema-generation bump.

**Tech Stack:** PostgreSQL 18 (SQL function in `db/046_patient_search.sql`), Rust integration tests (`crates/cairn-node/tests/`), `tokio-postgres`.

**Spec:** `docs/superpowers/specs/2026-09-21-patient-search-fragment-matching-design.md`

## Global Constraints

- **AGPL-3.0**; no new dependencies in this plan.
- **TDD**: the failing test is written and *seen to fail* before implementation.
- **`db/*.sql` is `include_str!`d** — a change to it needs a **rebuild**, not just a re-run (#593).
- **Migrations replay on every connect**: `db/046` is `CREATE OR REPLACE`, so it is replay-safe as-is. Do **not** add a `CREATE INDEX` in this plan.
- **No `SCHEMA_GENERATION` bump**: no projection output changes, so no reprojection is owed.
- **`matched_pass` stays `'name'`** for every name branch. Do not invent a new label (YAGNI — nothing consumes the distinction yet), but see Task 3: the file's dedup argument must be corrected because an overlapping label now exists.
- **DB-gated tests** need `CAIRN_TEST_PG`. Locally use **port 5532** (`:5432` is the legacy PG16 cluster and cannot load the schema). A DB-free run needs `CAIRN_ALLOW_DB_SKIP=1`.
- **Never `cmd | tail`** when judging a gate — it reports the last stage's status.

## File structure

| File | Responsibility | Change |
|---|---|---|
| `db/046_patient_search.sql` | advisory candidate generation | modify pass 3's lateral + predicate; correct two comments |
| `crates/cairn-node/tests/patient_search.rs` | behavioural tests for the above | add tests |
| `crates/cairn-node/tests/patient_search_drift.rs` | the sweep ⊆ search invariant, made executable | create |

---

### Task 1: 1a — stored-side part projection, callsigns excluded

**Files:**
- Modify: `db/046_patient_search.sql` (pass 3, the name branch)
- Test: `crates/cairn-node/tests/patient_search.rs`

**Interfaces:**
- Consumes: `cairn_search_candidates(p_name_tokens text[], p_birth_date text, p_identifiers jsonb) RETURNS TABLE (patient_id uuid, matched_pass text)` — unchanged signature.
- Produces: pass 3 now also matches a query token against the alphanumeric parts of a stored punctuated name. `matched_pass` is still `'name'`.

- [ ] **Step 1: Write the failing tests**

Add to `crates/cairn-node/tests/patient_search.rs`. The helpers below all already exist in that
file — `setup`, `EXTRA_TABLES`, `submit_registration`, `submit_field`, `name_assertion_body`,
`render_name_twin`, `search_candidates`, `cs`. Do **not** invent new ones; read
`the_name_token_pass_finds_a_chart_by_one_shared_token` (around line 211) for the idiom.

Note `search_candidates` returns `Vec<(Uuid, String)>` of `(patient_id, matched_pass)`, and the
existing tests assert with `assert_eq!` on the whole vector. These new tests use `.iter().any(..)`
instead, because a fixture that matches on more than one route would otherwise make an
order-dependent assertion.

```rust
/// A hyphenated compound surname is findable by EITHER half (#636, slice 1a).
///
/// `SearchQuery::new` already emits the parts of a punctuated word on the QUERY side; the stored
/// side split on whitespace only, so `Fyodorowksi-Eschenbacher` was one token and typing either
/// half found nothing. A clerk will not type the whole compound.
#[tokio::test]
async fn either_half_of_a_hyphenated_surname_finds_the_chart() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 0).await;
    let full = "Fyodorowksi-Eschenbacher";
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        1,
        name_assertion_body(full, Some("legal"), "patient-stated"),
        render_name_twin(full, Some("legal"), "patient-stated"),
    )
    .await
    .expect("name assertion accepted");

    for typed in ["eschenbacher", "fyodorowksi", "fyodorowksi-eschenbacher"] {
        let rows = search_candidates(&c, Some(&[typed]), None, None).await;
        assert!(
            rows.iter().any(|(id, pass)| *id == p && pass == "name"),
            "typing {typed:?} must find a chart stored as {full:?}; got {rows:?}"
        );
    }
}

/// A stored CALLSIGN is never fragmented — the anti-vacuity control for 1a.
///
/// The query side keeps whole words and drops single characters precisely so a John Doe callsign
/// (`Unknown-<class>-<site>-<date>-<tail>`) cannot fragment into pieces matching every John Doe
/// ever registered. Splitting the STORED side reintroduces that hazard from the other direction:
/// without the `use_key <> 'callsign'` guard, a clerk typing `unknown` surfaces every John Doe on
/// the node. A naive 1a passes every other test in this file and fails this one.
#[tokio::test]
async fn a_stored_callsign_is_not_fragmented_into_common_parts() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let (pid, call, _ord) = john_doe::register_john_doe(
        &mut c,
        &sk,
        &kid,
        "n",
        "ED",
        "site1",
        "2026-07-03",
        "unconscious ED arrival, no ID",
    )
    .await
    .expect("john doe registration accepted by the floor");

    // "unknown" is the callsign's leading part. It must not reach the chart.
    let fragmented = search_candidates(&c, Some(&["unknown"]), None, None).await;
    assert!(
        !fragmented.iter().any(|(id, _)| *id == pid),
        "typing 'unknown' must not surface a John Doe by fragmenting its callsign — that is \
         every John Doe on the node in one advisory list; got {fragmented:?}"
    );

    // The callsign must still be findable AS PRINTED. This is the half that makes the
    // exclusion safe rather than merely restrictive, and it duplicates the claim of
    // `a_john_doe_callsign_chart_is_returned_by_its_callsign_token` on purpose: this test
    // would otherwise pass if 1a broke callsign search entirely.
    let token = call.to_lowercase();
    let whole = search_candidates(&c, Some(&[token.as_str()]), None, None).await;
    assert!(
        whole.iter().any(|(id, _)| *id == pid),
        "the intact callsign must still find the chart; got {whole:?}"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
export CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$(whoami) dbname=cairn_test"
cargo test -p cairn-node --test patient_search either_half_of_a_hyphenated
```
Expected: FAIL — `typing "Eschenbacher" must find a chart stored as Fyodorowksi-Eschenbacher`.

The callsign test is expected to **pass** at this point (nothing fragments yet). That is correct: it is a regression guard for Step 3, not a driver. Say so in the commit message rather than deleting it.

- [ ] **Step 3: Widen pass 3's lateral**

In `db/046_patient_search.sql`, replace pass 3's `FROM`/`CROSS JOIN LATERAL` with a lateral that emits both token sources. Keep the existing `SELECT DISTINCT pn.patient_id, 'name'::text` head and the `JOIN unnest(...)` predicate exactly as they are:

```sql
    SELECT DISTINCT pn.patient_id, 'name'::text
      FROM patient_name pn
      CROSS JOIN LATERAL (
            -- The whole whitespace-delimited token, as before: this is what matches a
            -- punctuated name typed back exactly as printed, and an intact callsign.
            SELECT w AS tok
              FROM regexp_split_to_table(lower(normalize(pn.value, NFC)), '\s+') AS w
             WHERE w <> ''
            UNION
            -- Its alphanumeric PARTS (#636, slice 1a) — the mirror of what
            -- SearchQuery::new already emits on the query side, so a clerk typing one half
            -- of "Fyodorowksi-Eschenbacher" finds the chart. Single characters are dropped
            -- for the query side's reason: they cannot narrow a search and only inflate the
            -- advisory set.
            --
            -- CALLSIGNS ARE EXCLUDED, and this is not optional. A callsign is
            -- "Unknown-<class>-<site>-<date>-<tail>"; fragmenting it would project parts
            -- like 'unknown' and 'ed', so one typed word would surface every John Doe on
            -- the node. The query side keeps whole words for exactly this reason; this is
            -- the same guard on the other side. Pinned by
            -- `a_stored_callsign_is_not_fragmented_into_common_parts`.
            SELECT p
              FROM regexp_split_to_table(lower(normalize(pn.value, NFC)),
                                         '[^[:alnum:]]+') AS p
             WHERE length(p) > 1
               AND pn.use_key <> 'callsign'
      ) AS toks
      JOIN unnest(COALESCE(p_name_tokens, ARRAY[]::text[])) t
        ON toks.tok = lower(normalize(t, NFC))
     WHERE toks.tok <> ''
```

- [ ] **Step 4: Rebuild and run the tests**

```bash
cargo test -p cairn-node --test patient_search
```
Expected: PASS, all tests in the file. The rebuild matters — `db/046` is `include_str!`d, so a stale binary silently tests the old SQL (#593).

- [ ] **Step 5: Commit**

```bash
git add db/046_patient_search.sql crates/cairn-node/tests/patient_search.rs
git commit -m "feat(#636): stored names project their alphanumeric parts, callsigns excluded"
```

---

### Task 2: 1b — exact-or-prefix, with a 3-character minimum

**Files:**
- Modify: `db/046_patient_search.sql` (pass 3's join predicate)
- Test: `crates/cairn-node/tests/patient_search.rs`

**Interfaces:**
- Consumes: Task 1's `toks` lateral.
- Produces: pass 3 matches a query token that is a *prefix* of a projected token, when the query token is ≥ 3 characters. `matched_pass` remains `'name'`.

- [ ] **Step 1: Write the failing tests**

Same helpers as Task 1. A small local helper keeps these two readable; put it beside the other
private helpers near the top of the file, not in `tests/common/` (a new `pub fn` there must also be
registered in `identity_scaffolding_shared.rs`'s expected-helper array, which this does not need).

```rust
/// Seed one chart carrying `name`. Returns its patient id.
async fn chart_named(c: &Client, sk: &SigningKey, kid: &str, wall: i64, name: &str) -> Uuid {
    let p = Uuid::now_v7();
    submit_registration(c, sk, kid, p, wall).await;
    submit_field(
        c,
        sk,
        kid,
        p,
        wall + 1,
        name_assertion_body(name, Some("legal"), "patient-stated"),
        render_name_twin(name, Some("legal"), "patient-stated"),
    )
    .await
    .expect("name assertion accepted");
    p
}

/// A three-character fragment finds a longer name (#636, slice 1b).
#[tokio::test]
async fn a_three_character_fragment_finds_a_longer_name() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;
    let p = chart_named(&c, &sk, &kid, 0, "Samantha Michaelowski").await;

    let hit = search_candidates(&c, Some(&["mich"]), None, None).await;
    assert!(
        hit.iter().any(|(id, _)| *id == p),
        "'mich' must find Michaelowski; got {hit:?}"
    );

    // Prefix, NOT infix: 'chael' sits mid-token. Pinning this makes a later move to trigram
    // search a deliberate decision rather than a silent drift.
    let miss = search_candidates(&c, Some(&["chael"]), None, None).await;
    assert!(
        !miss.iter().any(|(id, _)| *id == p),
        "a mid-token fragment must NOT match — this slice ships prefix matching only; got {miss:?}"
    );
}

/// The 3-character minimum gates PREFIXES, never short NAMES (#636).
///
/// This is the distinction most likely to be implemented wrongly: gating short *tokens* instead of
/// short *prefixes* passes every other test in this file and would make a two-character surname
/// unfindable. Exact matching has no length rule.
#[tokio::test]
async fn a_two_character_surname_is_still_found_by_exact_match() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let wu = chart_named(&c, &sk, &kid, 0, "Wu").await;
    let ng = chart_named(&c, &sk, &kid, 10, "Ng Wei").await;
    let li = chart_named(&c, &sk, &kid, 20, "Li-Wong").await;
    let wuang = chart_named(&c, &sk, &kid, 30, "Wuang").await;

    let by_wu = search_candidates(&c, Some(&["wu"]), None, None).await;
    assert!(
        by_wu.iter().any(|(id, _)| *id == wu),
        "'Wu' must find 'Wu' by EXACT match — the minimum gates prefixes, not names; got {by_wu:?}"
    );
    // The one refusal, and it is the unselective case the minimum exists for.
    assert!(
        !by_wu.iter().any(|(id, _)| *id == wuang),
        "a two-character PREFIX of a longer token must not match; got {by_wu:?}"
    );

    let by_ng = search_candidates(&c, Some(&["ng"]), None, None).await;
    assert!(
        by_ng.iter().any(|(id, _)| *id == ng),
        "'Ng' must find the whitespace-split token 'ng'; got {by_ng:?}"
    );

    let by_li = search_candidates(&c, Some(&["li"]), None, None).await;
    assert!(
        by_li.iter().any(|(id, _)| *id == li),
        "'Li' must find 1a's part 'li' of 'Li-Wong'; got {by_li:?}"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p cairn-node --test patient_search a_three_character_fragment
cargo test -p cairn-node --test patient_search a_two_character_surname
```
Expected: the first FAILS on `'mich' must find Michaelowski`. The second is expected to **pass** already (exact matching is untouched) — it is the guard that Step 3 must not break.

- [ ] **Step 3: Widen the join predicate**

Replace pass 3's `ON` clause from Task 1 with:

```sql
      JOIN unnest(COALESCE(p_name_tokens, ARRAY[]::text[])) t
        ON toks.tok = lower(normalize(t, NFC))
        -- PREFIX matching (#636, slice 1b), because a clerk types a fragment and picks from
        -- a list rather than typing a compound surname in full.
        --
        -- `starts_with`, NOT `LIKE lower(...) || '%'`: SearchQuery::new trims only the EDGE
        -- punctuation of a word, so an internal '%' or '_' survives into a query token and
        -- would be read by LIKE as a wildcard. starts_with has no escaping surface and is
        -- what LIKE 'x%' optimises to anyway.
        --
        -- MINIMUM 3 CHARACTERS, and note WHAT it gates: the PREFIX arm only. Exact matching
        -- above has no length rule, so a two-character surname ("Wu", "Ng") stays findable —
        -- only a two-character prefix OF A LONGER token is refused, which is the unselective
        -- case this exists for. A one- or two-character prefix matches a large fraction of
        -- any population; worse, if such a search preceded a registration, that whole
        -- candidate list would be written into a permanent signed attestation (ADR-0061).
        -- Pinned by `a_two_character_surname_is_still_found_by_exact_match`.
        OR (length(lower(normalize(t, NFC))) >= 3
            AND starts_with(toks.tok, lower(normalize(t, NFC))))
     WHERE toks.tok <> ''
```

- [ ] **Step 4: Run the whole file**

```bash
cargo test -p cairn-node --test patient_search
```
Expected: PASS, every test. Pay attention to the pre-existing tests around `matched_pass` collisions (roughly lines 420–470) — if any now fail, stop and read Task 3 before changing them.

- [ ] **Step 5: Commit**

```bash
git add db/046_patient_search.sql crates/cairn-node/tests/patient_search.rs
git commit -m "feat(#636): pass 3 matches a 3+ character prefix, not only a whole token"
```

---

### Task 3: correct the two comments the widening invalidates

**Files:**
- Modify: `db/046_patient_search.sql` (the `DRIFT NOTE` and the `DELIBERATELY REDUNDANT DEDUPLICATION` blocks)

Neither is cosmetic. Both are arguments a future reader will act on, and both are now false.

- [ ] **Step 1: Correct the dedup argument**

The existing block argues that per-branch `DISTINCT` alone would suffice because *"`matched_pass` is a per-branch LITERAL … two rows can only ever collide when they came from the SAME branch"*, and it names its own expiry: *"stops being safe the moment … a fourth pass is added with an overlapping label."* Task 1's parts source is inside pass 3's branch, but the `UNION` inside the lateral means one `pn` row can now yield the same `tok` twice (a single-word name is both a whole token and its own part).

Append to that block:

```sql
-- UPDATE (#636): the expiry named above has arrived, in the mild form. Pass 3's lateral now
-- UNIONs two token sources, so one patient_name row can yield the same token twice (a
-- single unpunctuated word is both a whole token and its own alphanumeric part). The
-- lateral's own UNION removes that, and the outer UNION removes anything it misses. What
-- is no longer true is the claim that "every possible duplicate is a within-branch
-- duplicate" holds for pass 3 INTERNALLY — so the per-branch DISTINCT is now doing real
-- work rather than being redundant belt. Keep all three dedups.
```

- [ ] **Step 2: Correct the DRIFT NOTE**

The note says the blocking keys mirror the matcher's and that *"if you change a key here, check the matcher."* A reader must not conclude the two have drifted by accident. Append:

```sql
-- UPDATE (#636): search is now DELIBERATELY WIDER than the matcher on the name key — it
-- matches stored token PARTS and 3+ character PREFIXES; the matcher's blocking keys are
-- unchanged. This is safe in exactly one direction and only that one: the invariant this
-- note protects is "a chart the sweep would pair is a chart this search finds"
-- (sweep-paired ⊆ search-found), and widening search preserves it. NARROWING search, or
-- widening the matcher without widening search, would break it. Made executable by
-- crates/cairn-node/tests/patient_search_drift.rs. Widening the matcher to match is a
-- separate question with its own recall/precision and sweep-cost evaluation (#353).
```

- [ ] **Step 3: Verify nothing else in the file contradicts the change**

```bash
grep -n "exact\|Exact\|LIKE\|index" db/046_patient_search.sql
```
The pass-3 comment asserting *"Exact equality, NOT `LIKE '%token%'`"* and *"keeps the door open to an expression index"* must be reconciled: exact equality is no longer the whole story, and there is still no index (nor can there be one on a set-returning function's output without materialising a token table). Rewrite that paragraph to say what is now true — prefix via `starts_with` is still not a leading wildcard, the pass has always been a scan, and an index would require a materialised token table.

- [ ] **Step 4: Commit**

```bash
git add db/046_patient_search.sql
git commit -m "docs(#636): the dedup argument and the DRIFT NOTE say what is now true"
```

---

### Task 4: the drift invariant, made executable

**Files:**
- Create: `crates/cairn-node/tests/patient_search_drift.rs`

This is the test that turns Task 3's "widening is safe" from an argument into a checked claim. Without it the DRIFT NOTE is prose asserting a property nothing verifies.

**Interfaces:**
- Consumes: `cairn_search_candidates`; the matcher's blocking-key extraction as expressed in `matcher/pipeline/db.py`'s `_GROUPS_SQL`.
- Produces: nothing; a guard.

- [ ] **Step 1: Write the test**

This test lives in its own file, so it needs its own copies of the module scaffolding. Follow
`patient_search.rs`'s header exactly — the same `#[path = "common/mod.rs"] mod common;` style, the
same `use` list, and the same `cs()` helper. Reuse `chart_named` by copying it (a `pub fn` in
`tests/common/` would have to be registered in `identity_scaffolding_shared.rs`'s expected-helper
array; this test does not need that coupling).

```rust
//! #636 — the one-directional invariant db/046's DRIFT NOTE protects.
//!
//! The sweep and the search share key EXTRACTION but not their queries: the sweep blocks
//! all-by-all, the search maps one query to a set. The property that must hold is
//! **sweep-paired ⊆ search-found** — any two charts the background duplicate sweep would put in
//! one block must both be reachable by a search for the key that blocked them.
//!
//! Slice 1 widened search and left the matcher alone, which preserves this. The test exists
//! because the NEXT change might not, and prose cannot fail CI.

#[tokio::test]
async fn every_sweep_block_key_is_still_reachable_by_search() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // One shape per way the two sides tokenise differently: plain, short, multi-word,
    // hyphenated, apostrophised, long compound, and an accented multi-word name.
    let fixtures = [
        "Smith",
        "Wu",
        "Ng Wei",
        "Li-Wong",
        "O'Brien-Smith",
        "Fyodorowksi-Eschenbacher",
        "Samantha Michaelowski",
        "García Pérez",
    ];

    let mut seeded: Vec<(&str, Uuid)> = Vec::new();
    for (i, name) in fixtures.iter().enumerate() {
        seeded.push((name, chart_named(&c, &sk, &kid, (i as i64) * 10, name).await));
    }

    for (name, id) in &seeded {
        // The sweep's blocking keys for this stored name, extracted exactly as
        // matcher/pipeline/db.py's _GROUPS_SQL does: whitespace split of the NFC-normalised,
        // lower-cased value. Asked of the SERVER so the extraction cannot drift from the
        // matcher's by being re-implemented in Rust here.
        let keys: Vec<String> = c
            .query(
                "SELECT tok FROM regexp_split_to_table(lower(normalize($1, NFC)), '\\s+') AS tok \
                 WHERE tok <> ''",
                &[name],
            )
            .await
            .unwrap()
            .iter()
            .map(|r| r.get::<_, String>(0))
            .collect();

        assert!(
            !keys.is_empty(),
            "fixture {name:?} produced no blocking key — this test would be vacuous for it"
        );

        for k in keys {
            let rows = search_candidates(&c, Some(&[k.as_str()]), None, None).await;
            assert!(
                rows.iter().any(|(found, _)| found == id),
                "the sweep would block {name:?} on key {k:?}, but a search for {k:?} does not \
                 find it — sweep-paired is no longer a subset of search-found (db/046 DRIFT \
                 NOTE). Either search was narrowed, or the matcher was widened without it."
            );
        }
    }
}
```

- [ ] **Step 2: Run it**

```bash
cargo test -p cairn-node --test patient_search_drift
```
Expected: PASS. If it fails, the widening broke the invariant and Task 1 or 2 is wrong — do not weaken this test to make it pass.

- [ ] **Step 3: Commit**

```bash
git add crates/cairn-node/tests/patient_search_drift.rs
git commit -m "test(#636): sweep-paired is still a subset of search-found"
```

---

### Task 5: the §1.2 measurement

**Files:**
- Modify: this plan (record the numbers below)

- [ ] **Step 1: Seed a realistic population**

Use the matcher's existing synthetic volume generator (`matcher/` — find it before writing anything new) to load the largest population a Pi-class node is expected to hold. Record the row count of `patient_name`.

- [ ] **Step 2: Measure four searches**

For each of: a selective fragment (`mich`), an unselective one (`smi`), an exact short name (`Wu`), and a full compound (`Fyodorowksi-Eschenbacher`), run `cairn_search_candidates` and record wall-clock time. Take the median of 5 runs after one warm-up.

```sql
EXPLAIN (ANALYZE, BUFFERS) SELECT * FROM cairn_search_candidates(ARRAY['mich'], NULL, '[]'::jsonb);
```

- [ ] **Step 3: Record the result against the budget**

The ceiling is **5 s to find an existing chart**, stated in db/046 itself. Write the four medians and the population size into the section below. If any exceeds 5 s, **stop and report** — do not tune silently. The likely remedy is a materialised token table, which is a separate slice with its own reprojection cost, not an edit to this one.

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/plans/2026-09-21-patient-search-fragment-matching-636.md
git commit -m "docs(#636): the §1.2 measurement"
```

---

## Paper-parity benchmark (§1.2)

**Paper counterpart:** flipping to a section of the alphabetical patient index drawer — you read the first few letters on the card edge, you do not read whole names.

**Steps:** paper 3 human acts (ask a name, flip to the letters, pull the card) → architecture-forced 2 (type a fragment, pick from the list) → UI bundling target 2, delivered by slice 2. `M ≤ N`, so there is no architecture defect to file. Today's M is effectively unbounded for a compound surname, because the clerk must type it exactly or get nothing — which is why this slice exists.

**Time + cognitive load:** budget is the **5 s to find an existing chart** already stated in `db/046`. Measured by Task 5 at Pi-class population; numbers recorded there. Cognitive load falls — the clerk no longer has to reproduce punctuation and spelling exactly. This slice exposes no clinician-facing surface of its own; the end-to-end measurement is owed by slice 2, which does.

## Measurement results

Measured 2026-09-21 against `cairn_perf_636` (dropped afterward — not `cairn_test`), seeded via
the matcher's existing synthetic generator (`cairn_matcher.eval.generator.generate_dataset`,
300,000 entities) plus five named fixture patients so each query's match count is reasoned-about
rather than accidental (3 Michaelowski-shaped names, a 3,000-strong Smith cohort at ~1% of
population, one `Wu`, one `Fyodorowksi-Eschenbacher`). Full method, all 5 raw runs per query, and
`EXPLAIN` plans in `.superpowers/sdd/2026-09-21-patient-search-fragment-matching-636/task-5-report.md`.

| Search | Median (5 runs) | Budget |
|---|---|---|
| `mich` (selective fragment) | 1177.6 ms | 5 s |
| `smi` (unselective fragment) | 1075.8 ms | 5 s |
| `Wu` (exact, short) | 680.1 ms | 5 s |
| `Fyodorowksi-Eschenbacher` (exact, compound) | 3374.7 ms | 5 s |

Population: `patient_name` rows = 605,392 (603,005 distinct patients). **Budget held for all four**,
but with real margin erosion on the exact-compound case (3.37 s of the 5 s budget, ~67% consumed) —
see the report for why the longest query string, not the widest fragment, turned out to be the
worst case, and an honest caveat on how this population size compares to a documented Pi-class
ceiling (none exists in the spec).

**Three caveats that must travel with these numbers (also recorded in #637 — cited, not deferred to
it: restated here because this file, not the gitignored task report, is what survives).**

1. **Hardware.** This was measured on Apple Silicon (Postgres.app), **not** Pi-class ARM. The 5 s
   ceiling in `db/046_patient_search.sql` is stated for a Pi-class node, so what this run shows is
   *"no breach on dev hardware,"* not *"the budget holds on the hardware the budget is about."* A
   Pi5 re-run is the natural follow-on — the project already keeps a Pi5 rig as its
   performance-floor smoke test (spike 0001).
2. **Population provenance.** 605,392 rows / 603,005 patients was a figure **chosen** as a
   generous stress level, not one sourced from any requirement. No documented Pi-class population
   target exists anywhere in `docs/spec/` — searched `topology.md`, `deployment.md`, `vision.md`,
   ADR-0001/0002/0016, and found only qualitative framing ("a handful of workstations," "a busy ED
   runs on a department server, not a Pi"). Do not read 605k as a spec figure; it is this task's
   stress choice.
3. **The shape of the cost is counter-intuitive — and the first diagnosis written here was wrong,
   corrected by a later reviewer's measurement.** The worst case is the **long exact name**
   (`Fyodorowksi-Eschenbacher`, 3.37 s / 67% of budget), not the unselective fragment (`smi`,
   1.08 s / 22%). This paragraph originally attributed that to pass 3's `starts_with` prefix arm
   running against ~1.2 million generated tokens and costing more per token for a longer query
   string. **That diagnosis is false.** A follow-up measurement reproduced the regression on a
   query where `starts_with` never fires at all (`mich` against a 200k-row unpunctuated table:
   411 ms before this slice, 2301 ms after — a 5.6× slowdown with the prefix arm never matching),
   and `starts_with` short-circuits on the first byte mismatch, so a *longer* prefix is cheaper to
   reject, not costlier — the opposite of what the original paragraph claimed.

   The measured drivers are instead:
   - **1a's second `regexp_split_to_table` plus the lateral's `UNION` dedup sort**, executed per
     `patient_name` row — this is the bulk of the cost;
   - **`lower(normalize(t, NFC))` re-evaluated three times per (stored-token × query-token) pair**
     on the non-matching path (once for equality, once for `length(...)`, once for
     `starts_with(...)`), and `normalize`'s cost scales with string length — which is why the long
     compound name looked like a prefix-arm problem when the real cost was repeated normalisation
     of a long string, not the prefix arm itself.

   Three semantically-neutral changes were measured to recover about 80% of the regression
   (3121 ms → 637 ms on the long-token case): hoisting the query-token normalisation behind an
   `OFFSET 0` optimisation fence (a plain subquery does **not** work — the planner re-inlines it);
   `UNION ALL` instead of `UNION` in the lateral, since the outer `SELECT DISTINCT patient_id`
   already collapses duplicates; and skipping the parts branch entirely for a value containing no
   punctuation, where the parts split is provably a subset of the whitespace split. None of these
   three were applied in this slice — they are follow-on work, tracked separately — this paragraph
   only corrects the diagnosis so a Pi5 follow-on, or any future optimisation slice, is not planned
   against the wrong cause (the original text pointed at a heavy fix — a materialised token table
   with its own reprojection cost — when three one-line changes recover most of the regression).

## Pi-class measurement — the real target hardware, at the real target size (2026-09-21)

Run on **Raspberry Pi 5 Model B Rev 1.0**, aarch64, 4 cores, 8 GB, PostgreSQL 18.4, `cairn_pgx`
0.3.0 — reached from this machine by `ssh -J dgx hherb@192.168.68.81`. All 53 migrations load
cleanly on ARM. Population **50,000** patients, the figure pinned in
[spec §8.1](../../spec/deployment.md). Median of 5 runs after a warm-up.

| Search | Median | Rows found |
|---|---|---|
| `fyodorowksi-eschenbacher` — exact long compound | **2525 ms** | 2500 |
| `mich` — selective Latin fragment | 1655 ms | 2500 |
| `smi` — unselective Latin fragment | 1639 ms | 35000 |
| `李小` — CJK 2-character prefix (#638) | 1593 ms | 2500 |
| `wu` — exact short, below the byte gate | 1512 ms | 0 |

**The 5 s ceiling holds: worst case 2525 ms, about 50% of budget.** This supersedes the Apple
Silicon run for the purpose of judging §1.2 — that one measured the wrong machine at 12× the wrong
population.

### The number that matters is not the worst case, it is the FLOOR

**Every search costs at least ~1500 ms, including one that finds nothing.** That is the scan: pass 3
reads all of `patient_name` and runs a lateral `regexp_split_to_table` over every value, so the cost
is paid before selectivity is even consulted. The spread from floor to worst case is only ~1000 ms,
and it tracks query *length*, exactly as [#639](https://github.com/cairn-ehr/cairn-ehr/issues/639)
found.

So the "budget held" headline is true and slightly misleading. §1.2's 5 s ceiling is for *find an
existing chart* and it is met. But [§5.11](../../spec/identity.md)'s other limb — *"type a few chars
and enter, no spinner"* — is **not** met at 1.5 s: that is spinner territory on every keystroke-driven
search, and slice 2's UI re-searches in the background as the clerk types. #639's three measured
optimisations cut the floor, not just the tail, which makes them materially more valuable than the
worst-case figure alone suggests.

### Honest caveats about this seeding

- Rows were inserted **directly into the `patient_name` projection**, not authored through the event
  log. `cairn_search_candidates` reads only that table, so this measures the intended read path —
  but it is not an end-to-end write-then-read test, and authoring 50k signed events on a Pi would
  have measured the write path, which is not what is budgeted here.
- **`wu` found 0 rows**, so that row does not verify short-name findability — the fixture generates
  `Wu0`…`Wu96` as single tokens, which `wu` does not equal. It is still a useful **floor** reading
  (the cost of a search that matches nothing), and short-name findability is pinned by
  `a_two_character_surname_is_still_found_by_exact_match` instead.
- **`smi` found 35,000 of 50,000** because the fixture makes most names `Smith John<i>`. A real
  population is not 70% one surname, so treat that row as a deliberate worst-case stress on result
  volume rather than a realistic query.

### Re-run with a REAL Australian name distribution (2026-09-21)

Same Pi 5, same 5 s budget, 50,378 rows drawn from the maintainer's synthetic-population pool
(`~/src/SyntheticHealthData/synthetic_demographics.sqlite3`): **965,260 distinct surnames**,
commonest surname 0.18% of the pool. Every query value below is **real, taken from the data**.

| Search | Median | Found | Earlier synthetic run |
|---|---|---|---|
| `fitzherbert-brockholes` — real long compound, exact | **2413 ms** | **0** | 2525 ms |
| `mich` — fragment | 1629 ms | 438 | 1655 ms |
| `smi` — fragment | 1586 ms | 191 | 1639 ms |
| `欧阳` — real CJK surname, 2 chars (#638) | 1561 ms | 5 | 1593 ms |
| `wu` — exact short surname | 1479 ms | 16 | 1513 ms |

**Budget held: worst case 2413 ms.**

**Every timing landed within ~5% of the synthetic run**, despite 50,378 distinct values replacing
about a dozen. A hypothesis stated before this run — that real token diversity would make the
lateral's `UNION` dedup materially worse — is **disproved**.

What the re-run *does* establish, which the synthetic run could only suggest:

- **Result size is irrelevant to cost.** `smi` matched **191** rows here against **35,000** in the
  synthetic run — a 180× change — and the time moved by 3%. The earlier run could only hint at this
  because its fixture was ~70% one surname; now it is measured.
- **The worst case found NOTHING.** `fitzherbert-brockholes` returned **zero rows** and was still
  ~800 ms slower than everything else. So the cost is per-token comparison work against the query
  string, with no relationship to output at all — [#639](https://github.com/cairn-ehr/cairn-ehr/issues/639)
  confirmed twice over, on real data.
- **The ~1500 ms floor is a property of the scan, not of the fixture.** It survived a complete change
  of data shape.

That makes #639's three optimisations target exactly the right thing: fixed per-row work that no
query can avoid and no data distribution changes.

**Caveats.** Rows were seeded directly into the `patient_name` projection (read path only, as
before). The pool carries only **516 CJK-script surnames (0.008%)**, far below Australia's real
Chinese-ancestry share, so a random 50k sample contained **exactly one** — CJK rows were topped up
to 379 deliberately so the #638 query had something to find; that row is an injected cohort, not a
natural-distribution result. And the labels "selective"/"unselective" proved backwards in real data:
`mich` matched *more* than `smi` (438 vs 191), because `mich` also matches the very common given
names Michael/Michelle while `smi` mostly reaches the surname Smith.

**This pool may not be the current version** — the maintainer believes the complete database is
ABS-modelled with gender/age/ethnicity distribution per region and carries diagnoses, allergies and
medications; it is on an offline archive reachable from ~2026-10-05. Re-run then if a published
figure is needed.
