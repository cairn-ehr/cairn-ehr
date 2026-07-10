# Collation-independent projection winner tiebreaks (#69) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every trigger-maintained projection's winner tiebreak collation-independent, so a federation of nodes with different default collations always converges to the same display winner.

**Architecture:** Every projection winner comparison over a TEXT key (`node_origin`/`asserted_origin` and the final `value`/`display`/`use_key` total-order keys) is compared under `COLLATE "C"` (byte order of the identical-on-every-node UTF-8 bytes). One shared predicate fix (`cairn_hlc_overlay_wins`) covers the five standing-state overlays; five demographic projections + two display VIEWs get inline `COLLATE "C"`. Projection-read-side only — no wire/floor/event-format change.

**Tech Stack:** PostgreSQL 18 (PL/pgSQL + SQL functions), pgrx (`cairn_pgx`), Rust integration tests (`tokio-postgres`), mkdocs (docs).

## Global Constraints

- **License:** AGPL-3.0; every dependency AGPL-3.0-compatible. (No new dependency in this plan.)
- **TDD:** failing test first, then the code that makes it pass. Load-bearing — this is the §9 safety-critical in-DB floor.
- **Reviewer-legibility first** for all safety-critical SQL; inline comments explain *why*, for a junior contributor.
- **In-place migration edits** (pre-clinical posture): edit `db/00X.sql` directly; the test harness reloads the whole schema via `db::connect_and_load_schema`. No new migration file, **no SCHEMA-array bump** (collation is projection-read-side, not a wire change).
- **DB-gated tests** need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + `cairn_pgx`); they self-serialize via `db::test_serial_guard`. Run the cairn-node suite with that env set.
- **Mechanism note:** `COLLATE "C"` is respected inside row-tuple comparisons, `IMMUTABLE sql` bodies, and mixed `bigint`/`text`/`bytea` rows (verified 2026-07-10). Byte order is identical on every node regardless of its default collation.
- **Divergent-pair convention (all tests):** choose two byte-distinct strings whose `"C"` order and a locale (ICU) order **flip** — canonical pair `'B'` vs `'a'`: under `"C"`, `'a' > 'B'` (lowercase byte 0x61 > uppercase 0x42); under ICU/`"unicode"`, `'B' > 'a'`. A test that constructs a tie with these two and asserts the projection picks the **`"C"`-order winner** (`'a'`) proves collation-*independence*, not merely in-DB determinism. Each test also asserts `'B' COLLATE "unicode" > 'a' COLLATE "unicode"` to document that a real locale collation would have chosen the *other* winner (this is why the fix matters).

---

## File map

| File | Change |
|---|---|
| `docs/spec/decisions/0045-collation-independent-projection-tiebreaks.md` | **Create** — the ADR |
| `docs/spec/sync.md` | Modify — one-line convergence invariant |
| `docs/spec/index.md` | Modify — spec version 0.45 → 0.46 |
| `db/002_projection.sql` | Modify — `cairn_hlc_overlay_wins` origin `COLLATE "C"` + comment |
| `db/010_demographics.sql` | Modify — `patient_identifier` WHERE tuple |
| `db/011_demographics_fields.sql` | Modify — `patient_demographic_apply` WHERE tuple (superseded body) |
| `db/013_demographics_sex_gender.sql` | Modify — `patient_demographic_apply` WHERE, both CASE branches (live) |
| `db/012_demographics_names.sql` | Modify — trigger WHERE + `patient_name_current` VIEW ORDER BY + comment |
| `db/014_demographics_address.sql` | Modify — trigger WHERE + `patient_address_current` VIEW ORDER BY + comment |
| `crates/cairn-node/tests/overlay_tiebreaker.rs` | Modify — pure-predicate collation case (Family 1) |
| `crates/cairn-node/tests/projection_collation_convergence.rs` | **Create** — Family 2 convergence tests |
| `docs/HANDOVER.md`, `docs/ROADMAP.md` | Modify — reflect #69 closed |

---

## Task 1: ADR-0045 + spec bump

Docs-only, discrete reviewable deliverable. Establishes the invariant later tasks' comments reference.

**Files:**
- Create: `docs/spec/decisions/0045-collation-independent-projection-tiebreaks.md`
- Modify: `docs/spec/sync.md` (convergence / set-union projection section)
- Modify: `docs/spec/index.md` (version line)

- [ ] **Step 1: Write ADR-0045.** Model structure/headers on `docs/spec/decisions/0044-enroll-fail-closed-on-actor-id-collision.md`. Content:
  - **Status:** Accepted · **Date:** 2026-07-10 · Refines principle 1 (convergence); relates to ADR-0031, #115.
  - **Context:** Projection winners break `(rank, hlc_wall, hlc_counter)` ties on TEXT keys (`node_origin`/`asserted_origin`, and the final `value`/`display`/`use_key`) compared under the **node-local default collation**. Two nodes with different default collations can replay the identical event set and pick different *display* winners for an exact tie — a silent set-union convergence violation (principle 1). Not data loss (full history in `event_log`). The **cross-origin** `(wall,counter)` tie (two honest nodes coinciding on wall+counter) needs no misbehavior and is decided *before* #115's collation-free `content_address` tiebreak is reached.
  - **Decision:** Every projection winner tiebreak comparison over a TEXT key MUST be made under `COLLATE "C"` (byte order of the identical-on-every-node UTF-8 encoding). Applies to the shared `cairn_hlc_overlay_wins` predicate and every demographic projection trigger + display VIEW.
  - **Alternatives rejected:** (a) `convert_to(x,'UTF8')::bytea` — identical result, costlier/less idiomatic; (b) carry `content_address` into the demographic projection tables — schema change to five tables, `value`/`display` still needed for display, disproportionate.
  - **Consequences:** the invariant binds all future projection slices — a new tiebreak over a TEXT key silently reintroduces the divergence unless it uses `COLLATE "C"`. `content_address` (BYTEA) remains the final Byzantine tiebreak for the overlays; this only makes the `origin` step ahead of it collation-safe. No wire/floor/SCHEMA change.

- [ ] **Step 2: Add the spec line.** In `docs/spec/sync.md`, in the convergence / set-union projection prose, add one sentence: *"Projection winner tiebreaks compare TEXT keys under `COLLATE \"C\"` (byte order) so the display winner converges across a federation of mixed default collations ([ADR-0045](decisions/0045-collation-independent-projection-tiebreaks.md))."* Place it next to the existing convergence guarantee text (grep `sync.md` for "set-union" / "converge").

- [ ] **Step 3: Bump the spec version.** In `docs/spec/index.md` change `**Spec version:** 0.45` → `**Spec version:** 0.46`.

- [ ] **Step 4: Build the docs to confirm no broken link/syntax.**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build`
Expected: build succeeds, no warnings about the new ADR link.

- [ ] **Step 5: Commit.**

```bash
git add docs/spec/decisions/0045-collation-independent-projection-tiebreaks.md docs/spec/sync.md docs/spec/index.md
git commit -m "docs(adr): ADR-0045 collation-independent projection winner tiebreaks (#69)"
```

---

## Task 2: Family 1 — the shared overlay predicate

**Files:**
- Modify: `db/002_projection.sql` (function `cairn_hlc_overlay_wins`, ~lines 34–41, and the header comment ~lines 24–27)
- Test: `crates/cairn-node/tests/overlay_tiebreaker.rs` (extend; reuse the existing `wins()` helper)

**Interfaces:**
- Consumes: `wins(c, nw, nc, no, na, cw, cc, co, ca) -> bool` (existing helper, calls `cairn_hlc_overlay_wins`).
- Produces: nothing new for later tasks.

- [ ] **Step 1: Write the failing test.** Append to `overlay_tiebreaker.rs`:

```rust
/// #69: the origin tiebreak must be collation-INDEPENDENT. Construct a cross-origin
/// (wall, counter) tie whose two origins order OPPOSITELY under "C" vs a locale (ICU)
/// collation ('B' vs 'a'). The predicate compares origin under COLLATE "C", so the
/// winner follows byte order ('a' > 'B') on every node regardless of its default collation.
#[tokio::test]
async fn overlay_origin_tiebreak_is_collation_independent() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Sanity: a real locale collation orders the pair the OTHER way — this is why #69 matters.
    let unicode_flips: bool = c
        .query_one("SELECT 'B' COLLATE \"unicode\" > 'a' COLLATE \"unicode\"", &[])
        .await
        .unwrap()
        .get(0);
    assert!(unicode_flips, "'B' > 'a' should hold under a locale collation");

    // Same (wall, counter); origins 'B' (new) vs 'a' (current); content_address never consulted
    // because the origins differ. Under COLLATE "C", 'a' > 'B', so new='B' does NOT outrank cur='a'.
    assert!(
        !wins(&c, 7, 0, "B", vec![9], Some(7), Some(0), Some("a"), Some(vec![1])).await,
        "new origin 'B' must LOSE to current origin 'a' under C byte order"
    );
    // Symmetric: new='a' outranks cur='B'.
    assert!(
        wins(&c, 7, 0, "a", vec![1], Some(7), Some(0), Some("B"), Some(vec![9])).await,
        "new origin 'a' must WIN over current origin 'B' under C byte order"
    );
}
```

- [ ] **Step 2: Run it — expect FAIL (red).** Before the fix, `cairn_hlc_overlay_wins` compares origin under the default (ICU-like) collation, where `'B' > 'a'`, so `wins(... "B" ... "a" ...)` returns `true` and the first assertion fails.

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test overlay_tiebreaker overlay_origin_tiebreak_is_collation_independent -- --nocapture`
Expected: FAIL on the first `assert!(!wins(...))`.

- [ ] **Step 3: Apply the fix.** In `db/002_projection.sql`, change the predicate body to compare origin under `COLLATE "C"`:

```sql
CREATE OR REPLACE FUNCTION cairn_hlc_overlay_wins(
    new_wall bigint, new_counter int, new_origin text, new_addr bytea,
    cur_wall bigint, cur_counter int, cur_origin text, cur_addr bytea
) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT (new_wall, new_counter, new_origin COLLATE "C", new_addr)
         > (COALESCE(cur_wall, -1), COALESCE(cur_counter, -1),
            COALESCE(cur_origin, '') COLLATE "C", COALESCE(cur_addr, '\x'::bytea));
$$;
```

Update the header comment (the sentence in ~lines 24–27 that says content_address is "immune to the TEXT-collation concern that #69 tracks for the `origin` comparison"): replace the deferral with a note that the `origin` comparison is now itself collation-safe under `COLLATE "C"` per **ADR-0045 (#69)**, and `content_address` remains the final Byzantine same-origin tiebreak.

- [ ] **Step 4: Run it — expect PASS (green), and re-run the whole file for no regression.**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test overlay_tiebreaker -- --nocapture`
Expected: all tests PASS (the new one + the existing #115/#157 convergence tests unchanged).

- [ ] **Step 5: Commit.**

```bash
git add db/002_projection.sql crates/cairn-node/tests/overlay_tiebreaker.rs
git commit -m "fix(floor): collation-independent origin tiebreak in the shared overlay predicate (#69)"
```

---

## Task 3: Family 2 — patient_identifier (db/010) + the shared test helper

**Files:**
- Create: `crates/cairn-node/tests/projection_collation_convergence.rs`
- Modify: `db/010_demographics.sql` (the `patient_identifier` ON CONFLICT WHERE, ~lines 116–119)

**Interfaces:**
- Produces (reused by Tasks 4–6): a shared test helper module in the new file —
  - `cs() -> Option<String>`
  - `async fn setup(c: &Client) -> (SigningKey, String)` — truncate + enroll one agent, returns `(sk, kid)`.
  - `async fn submit_generic(c, sk, kid, patient: Uuid, event_type: &str, wall: i64, counter: i32, origin: &str, payload: serde_json::Value, twin: &str)` — author+sign+`submit_event` with the wire HLC set **verbatim** from `(wall, counter, origin)`. Models `demographics_fields.rs::submit_field` but parameterizes `counter` and `node_origin`.
  - `async fn locale_flips(c, hi: &str, lo: &str) -> bool` — asserts `hi COLLATE "unicode" > lo COLLATE "unicode"` (the "a real locale disagrees" guard).

- [ ] **Step 1: Write the helper + the failing identifier test.** Create `projection_collation_convergence.rs`:

```rust
//! #69 — collation-independent projection winner tiebreaks. Each test constructs an exact
//! (rank, wall, counter) tie whose remaining TEXT tiebreak key holds a pair that orders
//! OPPOSITELY under "C" vs a locale (ICU) collation ('B' vs 'a'), then asserts the projection
//! picks the "C"-order winner in BOTH arrival orders — proving convergence is collation-
//! independent, not merely in-DB deterministic. Real Postgres, gated on $CAIRN_TEST_PG,
//! serialized cluster-wide via db::test_serial_guard.
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use serde_json::json;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

async fn setup(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name, patient_address CASCADE")
        .await
        .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// True iff a locale (ICU) collation orders `hi > lo` — the guard that the chosen pair
/// really does flip vs "C" (where the projection must instead pick the byte-order winner).
async fn locale_flips(c: &Client, hi: &str, lo: &str) -> bool {
    c.query_one(
        "SELECT $1::text COLLATE \"unicode\" > $2::text COLLATE \"unicode\"",
        &[&hi, &lo],
    )
    .await
    .unwrap()
    .get(0)
}

/// Author + sign + submit one event with the wire HLC set verbatim from (wall, counter, origin).
/// submit_event stores the wire HLC as-is (db/005), so the projection sees exactly these values.
#[allow(clippy::too_many_arguments)]
async fn submit_generic(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    event_type: &str,
    wall: i64,
    counter: i32,
    origin: &str,
    payload: serde_json::Value,
    twin: &str,
) {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: event_type.into(),
        schema_version: "demographic.field/1".into(),
        hlc: Hlc {
            wall,
            counter,
            node_origin: origin.into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin.into()),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap();
}

/// A demographic.identifier.asserted payload (§4.4). One system, same normalized value on
/// both events so the (patient, system, match_key) PK collides — leaving the winner to the
/// (wall, counter, origin, value) tiebreak.
fn identifier_payload(system: &str, value: &str) -> serde_json::Value {
    json!({
        "system": system,
        "value": value,
        "use": "official",
        "provenance": "document-verified"
    })
}

/// #69: patient_identifier resolves an equal-(wall,counter) cross-origin tie by origin under
/// COLLATE "C". Origins 'B' vs 'a' flip between "C" and a locale collation; the retained
/// representative must be the byte-order winner ('a') regardless of apply order.
#[tokio::test]
async fn identifier_origin_tiebreak_is_collation_independent() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert!(locale_flips(&c, "B", "a").await, "pair must flip under a locale collation");

    // Two arrival orders → both must land on origin 'a' (the C byte-order winner).
    for (first, second) in [("B", "a"), ("a", "B")] {
        let (sk, kid) = setup(&c).await;
        let p = Uuid::now_v7();
        // Same value ("ABC123") → same match_key → same PK; same (wall,counter); origin differs.
        submit_generic(&c, &sk, &kid, p, "demographic.identifier.asserted", 5, 0, first,
            identifier_payload("ns:test", "ABC123"), "id ABC123").await;
        submit_generic(&c, &sk, &kid, p, "demographic.identifier.asserted", 5, 0, second,
            identifier_payload("ns:test", "ABC123"), "id ABC123").await;

        let origin: String = c
            .query_one(
                "SELECT asserted_origin FROM patient_identifier WHERE patient_id = $1::text::uuid",
                &[&p.to_string()],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(origin, "a", "C byte-order winner regardless of arrival order {first}->{second}");
    }
}
```

> Note: if `demographic.identifier.asserted`'s payload shape or twin requirement differs, mirror `crates/cairn-node/tests/demographics.rs` (the identifier slice test) for the exact `identifier_payload` fields and twin text. The `match_key`/`system` must be identical across both events so the PK collides.

- [ ] **Step 2: Run it — expect FAIL (red).** Before the fix, origin is compared under default (ICU-like) collation where `'B' > 'a'`, so origin `'B'` wins → `assert_eq!(origin, "a")` fails.

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test projection_collation_convergence identifier_origin_tiebreak_is_collation_independent -- --nocapture`
Expected: FAIL (origin comes back `"B"`).

- [ ] **Step 3: Apply the fix.** In `db/010_demographics.sql`, annotate the WHERE tuple keys:

```sql
    WHERE (EXCLUDED.asserted_hlc_wall, EXCLUDED.asserted_hlc_count,
           EXCLUDED.asserted_origin COLLATE "C", EXCLUDED.value COLLATE "C")
        > (patient_identifier.asserted_hlc_wall, patient_identifier.asserted_hlc_count,
           patient_identifier.asserted_origin COLLATE "C", patient_identifier.value COLLATE "C");
```

- [ ] **Step 4: Run it — expect PASS (green).**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test projection_collation_convergence identifier_origin_tiebreak_is_collation_independent -- --nocapture`
Expected: PASS (origin `"a"` in both arrival orders).

- [ ] **Step 5: Commit.**

```bash
git add crates/cairn-node/tests/projection_collation_convergence.rs db/010_demographics.sql
git commit -m "fix(floor): collation-independent patient_identifier tiebreak + shared test helper (#69)"
```

---

## Task 4: Family 2 — patient_demographic (db/013 live + db/011 superseded)

Tests the `value` tiebreak in **both** CASE branches (provenance-first via `dob`; recency-first via `gender-identity`).

**Files:**
- Modify: `db/013_demographics_sex_gender.sql` (the `patient_demographic_apply` WHERE CASE, both branches, ~lines 75–86)
- Modify: `db/011_demographics_fields.sql` (the superseded `patient_demographic_apply` WHERE, ~lines 156–159)
- Test: `crates/cairn-node/tests/projection_collation_convergence.rs` (append)

**Interfaces:**
- Consumes: `setup`, `submit_generic`, `locale_flips` from Task 3.
- Produces: `fn field_payload(field, value, provenance) -> serde_json::Value`.

- [ ] **Step 1: Write the failing test.** Append:

```rust
/// A demographic.field.asserted payload (§4.2). `facets` carries dob precision when needed.
fn field_payload(field: &str, value: &str, provenance: &str) -> serde_json::Value {
    let mut p = json!({"field": field, "value": value, "provenance": provenance});
    if field == "dob" {
        p["facets"] = json!({"precision": "day"});
    }
    p
}

/// #69: patient_demographic breaks an equal-(rank,wall,counter,origin) tie on `value` under
/// COLLATE "C", in BOTH winner-policy branches. Values 'B'/'a' flip between "C" and a locale
/// collation; the projected winner must be the byte-order winner ('a') regardless of arrival order.
#[tokio::test]
async fn demographic_value_tiebreak_is_collation_independent_both_branches() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert!(locale_flips(&c, "B", "a").await);

    // ("dob", ...) exercises the provenance-first branch; ("gender-identity", ...) the recency-first.
    for field in ["dob", "gender-identity"] {
        for (first, second) in [("B", "a"), ("a", "B")] {
            let (sk, kid) = setup(&c).await;
            let p = Uuid::now_v7();
            // Same field+provenance (→ equal rank), same (wall,counter,origin); value differs.
            submit_generic(&c, &sk, &kid, p, "demographic.field.asserted", 9, 0, "n",
                field_payload(field, first, "patient-stated"), &format!("{field} {first}")).await;
            submit_generic(&c, &sk, &kid, p, "demographic.field.asserted", 9, 0, "n",
                field_payload(field, second, "patient-stated"), &format!("{field} {second}")).await;

            let value: String = c
                .query_one(
                    "SELECT value FROM patient_demographic \
                     WHERE patient_id = $1::text::uuid AND field = $2",
                    &[&p.to_string(), &field],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(value, "a", "{field}: C byte-order winner for {first}->{second}");
        }
    }
}
```

> Note: confirm `gender-identity` has a winner policy in `cairn_demographic_field_policy` (db/013) so it projects; if the exact field name differs, use the recency-first field db/013 actually registers (grep db/013 for `recency-first`).

- [ ] **Step 2: Run it — expect FAIL (red).** Value compared under default collation → `'B'` wins.

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test projection_collation_convergence demographic_value_tiebreak_is_collation_independent_both_branches -- --nocapture`
Expected: FAIL (value `"B"`).

- [ ] **Step 3: Apply the fix.** In `db/013_demographics_sex_gender.sql`, annotate `asserted_origin` and `value` under `COLLATE "C"` in **both** CASE branches:

```sql
    WHERE CASE cairn_demographic_field_policy(pd.field)
        WHEN 'recency-first' THEN
            (EXCLUDED.asserted_hlc_wall, EXCLUDED.asserted_hlc_count,
             EXCLUDED.provenance_rank, EXCLUDED.asserted_origin COLLATE "C", EXCLUDED.value COLLATE "C")
          > (pd.asserted_hlc_wall, pd.asserted_hlc_count,
             pd.provenance_rank, pd.asserted_origin COLLATE "C", pd.value COLLATE "C")
        ELSE
            (EXCLUDED.provenance_rank, EXCLUDED.asserted_hlc_wall,
             EXCLUDED.asserted_hlc_count, EXCLUDED.asserted_origin COLLATE "C", EXCLUDED.value COLLATE "C")
          > (pd.provenance_rank, pd.asserted_hlc_wall,
             pd.asserted_hlc_count, pd.asserted_origin COLLATE "C", pd.value COLLATE "C")
    END;
```

Then in `db/011_demographics_fields.sql` (the function db/013 supersedes — fixed so no migration shows a stale pattern), annotate its WHERE tuple identically and add a one-line comment `-- COLLATE "C" tiebreak per ADR-0045 (#69); this body is superseded by db/013`:

```sql
    WHERE (EXCLUDED.provenance_rank, EXCLUDED.asserted_hlc_wall,
           EXCLUDED.asserted_hlc_count, EXCLUDED.asserted_origin COLLATE "C", EXCLUDED.value COLLATE "C")
        > (pd.provenance_rank, pd.asserted_hlc_wall,
           pd.asserted_hlc_count, pd.asserted_origin COLLATE "C", pd.value COLLATE "C");
```

- [ ] **Step 4: Run it — expect PASS (green).**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test projection_collation_convergence demographic_value_tiebreak_is_collation_independent_both_branches -- --nocapture`
Expected: PASS (value `"a"` for both fields, both arrival orders).

- [ ] **Step 5: Commit.**

```bash
git add db/013_demographics_sex_gender.sql db/011_demographics_fields.sql crates/cairn-node/tests/projection_collation_convergence.rs
git commit -m "fix(floor): collation-independent patient_demographic tiebreak, both branches (#69)"
```

---

## Task 5: Family 2 — patient_name (db/012) trigger + display VIEW

Tests the trigger's `asserted_origin` key AND the `patient_name_current` VIEW's `value` display key.

**Files:**
- Modify: `db/012_demographics_names.sql` (trigger WHERE ~lines 85–88; VIEW ORDER BY ~lines 125–129; the #69 forward-note comment ~lines 114–117)
- Test: `crates/cairn-node/tests/projection_collation_convergence.rs` (append)

**Interfaces:**
- Consumes: `setup`, `submit_generic`, `locale_flips`, `field_payload` (name uses field `"name"`; see note).

- [ ] **Step 1: Write the failing test.** Append. The name event is a `demographic.field.asserted` with `field:"name"`, `value:<the name>`, and a `use` facet. Two legal names for one patient with equal `(wall,counter,rank,origin)` but different `value` → the retained set holds both members (PK includes `value`), and `patient_name_current`'s ORDER BY falls through to `value COLLATE "C" DESC`.

```rust
/// A demographic.field.asserted name payload (§4.2). `use` selects the legal tier.
fn name_payload(value: &str, name_use: &str, provenance: &str) -> serde_json::Value {
    json!({"field": "name", "value": value, "provenance": provenance,
           "facets": {"use": name_use}})
}

/// #69: patient_name_current picks its DISPLAY name across equal-(rank,wall,counter,origin)
/// members by `value` under COLLATE "C". Values 'B'/'a' flip vs a locale collation; the
/// displayed name must be the byte-order winner ('a').
#[tokio::test]
async fn name_display_value_tiebreak_is_collation_independent() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert!(locale_flips(&c, "B", "a").await);

    for (first, second) in [("B", "a"), ("a", "B")] {
        let (sk, kid) = setup(&c).await;
        let p = Uuid::now_v7();
        // Two legal names, equal (wall,counter,provenance,origin); values differ → VIEW tiebreak.
        submit_generic(&c, &sk, &kid, p, "demographic.field.asserted", 3, 0, "n",
            name_payload(first, "legal", "patient-stated"), &format!("name {first}")).await;
        submit_generic(&c, &sk, &kid, p, "demographic.field.asserted", 3, 0, "n",
            name_payload(second, "legal", "patient-stated"), &format!("name {second}")).await;

        let value: String = c
            .query_one(
                "SELECT value FROM patient_name_current WHERE patient_id = $1::text::uuid",
                &[&p.to_string()],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(value, "a", "displayed name is the C byte-order winner for {first}->{second}");
    }
}
```

> Note: confirm the exact name field shape from `crates/cairn-node/tests/demographics_names.rs` — the `field` value (`"name"`), where `use` lives (`facets.use`), and the twin renderer. Both events must share the same `use_key='legal'` so the VIEW's `(use_key='legal') DESC` and recency keys tie, forcing the `value` tiebreak.

- [ ] **Step 2: Run it — expect FAIL (red).** VIEW `value DESC` uses default collation → `'B'` displays.

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test projection_collation_convergence name_display_value_tiebreak_is_collation_independent -- --nocapture`
Expected: FAIL (value `"B"`).

- [ ] **Step 3: Apply the fix.** In `db/012_demographics_names.sql`, the trigger WHERE:

```sql
    WHERE (EXCLUDED.last_hlc_wall, EXCLUDED.last_hlc_count,
           EXCLUDED.provenance_rank, EXCLUDED.asserted_origin COLLATE "C")
        > (pn.last_hlc_wall, pn.last_hlc_count,
           pn.provenance_rank, pn.asserted_origin COLLATE "C");
```

and the VIEW ORDER BY:

```sql
ORDER BY patient_id,
         (use_key = 'legal') DESC,
         last_hlc_wall DESC, last_hlc_count DESC,
         provenance_rank DESC, asserted_origin COLLATE "C" DESC,
         use_key COLLATE "C" DESC, value COLLATE "C" DESC;
```

Update the #69 forward-note comment (~lines 114–117) to say the `COLLATE "C"` fix is now applied here per **ADR-0045 (#69)**.

- [ ] **Step 4: Run it — expect PASS (green).**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test projection_collation_convergence name_display_value_tiebreak_is_collation_independent -- --nocapture`
Expected: PASS (value `"a"`).

- [ ] **Step 5: Commit.**

```bash
git add db/012_demographics_names.sql crates/cairn-node/tests/projection_collation_convergence.rs
git commit -m "fix(floor): collation-independent patient_name trigger + display VIEW (#69)"
```

---

## Task 6: Family 2 — patient_address (db/014) trigger + display VIEW

Tests the `patient_address_current` VIEW's `display` key.

**Files:**
- Modify: `db/014_demographics_address.sql` (trigger WHERE ~lines 177–180; VIEW ORDER BY ~lines 209–211; the #69 forward-note comment above the VIEW)
- Test: `crates/cairn-node/tests/projection_collation_convergence.rs` (append)

**Interfaces:**
- Consumes: `setup`, `submit_generic`, `locale_flips`.

- [ ] **Step 1: Write the failing test.** Append. Address event is `demographic.field.asserted` with `field:"address"`, a `display` string, and a `use` facet. Two addresses for one `use` with equal `(wall,counter,rank,origin)`, different `display` → VIEW falls to `display COLLATE "C" DESC`.

```rust
/// A demographic.field.asserted address payload (§4.3). `display` is the legibility twin core.
fn address_payload(display: &str, addr_use: &str, provenance: &str) -> serde_json::Value {
    json!({"field": "address", "value": display, "provenance": provenance,
           "facets": {"use": addr_use, "display": display}})
}

/// #69: patient_address_current picks its per-use display across equal-(rank,wall,counter,origin)
/// members by `display` under COLLATE "C". 'B'/'a' flip vs a locale collation; the shown address
/// must be the byte-order winner ('a').
#[tokio::test]
async fn address_display_tiebreak_is_collation_independent() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert!(locale_flips(&c, "B", "a").await);

    for (first, second) in [("B", "a"), ("a", "B")] {
        let (sk, kid) = setup(&c).await;
        let p = Uuid::now_v7();
        submit_generic(&c, &sk, &kid, p, "demographic.field.asserted", 4, 0, "n",
            address_payload(first, "home", "patient-stated"), &format!("addr {first}")).await;
        submit_generic(&c, &sk, &kid, p, "demographic.field.asserted", 4, 0, "n",
            address_payload(second, "home", "patient-stated"), &format!("addr {second}")).await;

        let display: String = c
            .query_one(
                "SELECT display FROM patient_address_current WHERE patient_id = $1::text::uuid",
                &[&p.to_string()],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(display, "a", "shown address is the C byte-order winner for {first}->{second}");
    }
}
```

> Note: confirm the exact address payload shape (`field`, where `display`/`use` live, `structured`/`geo` optionality, twin) from `crates/cairn-node/tests/demographics_address.rs`. Both events must share `use_key` so the retained set keeps both members and the VIEW's per-use tiebreak reaches `display`.

- [ ] **Step 2: Run it — expect FAIL (red).**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test projection_collation_convergence address_display_tiebreak_is_collation_independent -- --nocapture`
Expected: FAIL (display `"B"`).

- [ ] **Step 3: Apply the fix.** In `db/014_demographics_address.sql`, trigger WHERE:

```sql
    WHERE (EXCLUDED.last_hlc_wall, EXCLUDED.last_hlc_count,
           EXCLUDED.provenance_rank, EXCLUDED.asserted_origin COLLATE "C")
        > (pa.last_hlc_wall, pa.last_hlc_count,
           pa.provenance_rank, pa.asserted_origin COLLATE "C");
```

VIEW ORDER BY:

```sql
ORDER BY patient_id, use_key,
         last_hlc_wall DESC, last_hlc_count DESC,
         provenance_rank DESC, asserted_origin COLLATE "C" DESC,
         display COLLATE "C" DESC;
```

Update the #69 forward-note comment above the VIEW to say the `COLLATE "C"` fix is now applied per **ADR-0045 (#69)**.

- [ ] **Step 4: Run it — expect PASS (green).**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test projection_collation_convergence address_display_tiebreak_is_collation_independent -- --nocapture`
Expected: PASS (display `"a"`).

- [ ] **Step 5: Commit.**

```bash
git add db/014_demographics_address.sql crates/cairn-node/tests/projection_collation_convergence.rs
git commit -m "fix(floor): collation-independent patient_address trigger + display VIEW (#69)"
```

---

## Task 7: Whole-workspace verification + HANDOVER/ROADMAP

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Full cairn-node DB-gated suite green (no regressions).**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node -- --nocapture`
Expected: all pass, incl. `overlay_tiebreaker`, `projection_collation_convergence`, and every existing demographics/identity test.

- [ ] **Step 2: Workspace build/test + lints clean.**

Run: `cargo test --workspace` then `cargo fmt --all -- --check` then `cargo clippy --workspace --all-targets -- -D warnings`
Expected: all green.

- [ ] **Step 3: Docs build.**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build`
Expected: succeeds.

- [ ] **Step 4: Update HANDOVER + ROADMAP.** In `docs/HANDOVER.md` add a "This session" entry: #69 closed — codebase-wide `COLLATE "C"` projection-tiebreak canonicalization (shared overlay predicate + five demographic projections + two VIEWs), ADR-0045, spec v0.46. In `docs/ROADMAP.md` Phase 2, mark the #69 residual (previously "deferred to #69" on the deterministic-overlay-convergence line) as **done**. Prune both to stay concise.

- [ ] **Step 5: Commit.**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: HANDOVER/ROADMAP — collation-independent projection tiebreaks done (#69)"
```

---

## Self-review notes (author)

- **Spec coverage:** mechanism → Global Constraints + Task 2; Family 1 → Task 2; Family 2 (db/010/011/013/012/014 + two VIEWs) → Tasks 3–6; ADR + spec → Task 1; tests (collation-divergent pair, both arrival orders, per projection) → Tasks 2–6; verification → Task 7. All design sections mapped.
- **Superseded db/011:** fixed in Task 4 with an explicit comment, per the approved micro-call.
- **Both db/013 CASE branches:** covered by Task 4's two-field loop (dob = provenance-first, gender-identity = recency-first).
- **Type consistency:** `submit_generic`/`setup`/`locale_flips`/`field_payload`/`name_payload`/`address_payload`/`identifier_payload` defined in Task 3–6 and reused with matching signatures.
- **Open confirmations flagged inline** (payload shapes for identifier/name/address, `gender-identity` policy name) — the executor verifies each against the existing sibling test file named in the note before running red.
