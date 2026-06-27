# Demographic Names (slice 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the §4.2 **names** demographic field — every name retained as evidence, with a single legal-preferred, recency-first display-winner — on `cairn-node`.

**Architecture:** A name is the existing generic `demographic.field.asserted` event with `field="name"` (the slice-2 event + in-DB floor accept it unchanged). A new `db/012` adds the first **retained-set + display-winner** projection: a `patient_name` set table (most-recent-assertion-per-member representative) plus a `patient_name_current` VIEW computing the winner (legal-preferred → recency-first → any-use fallback) with no stored pointer. Pure Rust builders mirror the slice-1/2 demographics functions.

**Tech Stack:** Rust (`cairn-event`, `cairn-node`), PostgreSQL 18 + `cairn_pgx` (pgrx 0.18.1), `tokio-postgres`, `serde_json`.

## Global Constraints

- **TDD** — failing test first, then minimal code. Load-bearing on this safety-critical (§9) surface.
- **AGPL-3.0** — no new dependency is added in this plan; if that changes, the dep must be AGPL-3.0-compatible (check before adding).
- **PostgreSQL ≥ 18** + `cairn_pgx`. DB-gated tests read the connection string from `CAIRN_TEST_PG` (e.g. `host=127.0.0.1 port=5532 user=hherb dbname=cairn_test`) and **skip with a printed notice** when it is unset. They self-serialize cluster-wide via `db::test_serial_guard`.
- **Junior-readable inline comments** on every non-trivial function — *why/how it fits*, not just *what*.
- **Files < 500 lines** where feasible; `db/012` and the `demographics.rs` additions are both well under.
- **The in-DB floor is never weakened.** This slice adds **no** floor change and **no** new event type — verify nothing in `db/005`/`010`/`011` or `submit_event` is edited.
- **All tests + `cargo clippy --workspace` green before any commit** (unless the user grants an explicit exception).

---

### Task 1: `cairn-event` name builder + legibility twin (pure)

**Files:**
- Modify: `crates/cairn-event/src/demographics.rs` (add two functions + unit tests; reuses the existing `demographic_field_body` + `json!`/`Value` imports already in the file)

**Interfaces:**
- Consumes: `demographic_field_body(field: &str, value: &str, facets: Option<Value>, provenance: &str) -> Value` (already in this file).
- Produces:
  - `name_assertion_body(value: &str, use_: Option<&str>, provenance: &str) -> serde_json::Value`
  - `render_name_twin(value: &str, use_: Option<&str>, provenance: &str) -> String`

- [ ] **Step 1: Write the failing unit tests**

Add to the `#[cfg(test)] mod tests` block at the bottom of `crates/cairn-event/src/demographics.rs`:

```rust
    #[test]
    fn name_body_carries_field_value_use_and_provenance() {
        let v = name_assertion_body("田中 太郎", Some("legal"), "document-verified");
        assert_eq!(v["field"], "name");
        assert_eq!(v["value"], "田中 太郎");
        assert_eq!(v["provenance"], "document-verified");
        assert_eq!(v["facets"]["use"], "legal");
    }

    #[test]
    fn name_body_omits_absent_use_never_null() {
        let v = name_assertion_body("Ronaldinho", None, "patient-stated");
        assert_eq!(v["field"], "name");
        assert_eq!(v["value"], "Ronaldinho");
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("facets"), "absent use carries no facets bag, never null");
    }

    #[test]
    fn name_twin_uses_use_when_present_else_provenance() {
        assert_eq!(
            render_name_twin("田中 太郎", Some("legal"), "document-verified"),
            "Name (legal): 田中 太郎"
        );
        assert_eq!(
            render_name_twin("Mary", None, "patient-stated"),
            "Name (patient-stated): Mary"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cairn-event name_ -- --nocapture`
Expected: FAIL — `cannot find function name_assertion_body` / `render_name_twin`.

- [ ] **Step 3: Write the minimal implementation**

Add to `crates/cairn-event/src/demographics.rs`, after `sex_at_birth_assertion_body` (keep the demographic-field builders together):

```rust
/// One §4.2 name assertion. `value` is the authored display string, carried
/// verbatim ("田中 太郎", a mononym, a patronymic — culture-neutral as-authored;
/// the core never parses or normalises it). `use_` is the recommended-but-open
/// category (legal/maiden/alias/transliteration/…), placed in the `facets.use`
/// bag and omitted entirely when None so the in-DB floor sees exactly what was
/// asserted. Structured parts (given/family + a locale profile) are a later slice.
pub fn name_assertion_body(value: &str, use_: Option<&str>, provenance: &str) -> Value {
    let facets = use_.map(|u| json!({ "use": u }));
    demographic_field_body("name", value, facets, provenance)
}

/// Render the §4.5 legibility twin for a name. Matches the spec example
/// "Name (legal): 田中 太郎": the `use` sits in the parens when present; when it is
/// absent the parens fall back to the provenance ("Name (patient-stated): Mary")
/// so the parenthetical is never empty and the fact stays legible without a profile.
pub fn render_name_twin(value: &str, use_: Option<&str>, provenance: &str) -> String {
    let context = use_.unwrap_or(provenance);
    format!("Name ({context}): {value}")
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cairn-event name_ -- --nocapture`
Expected: PASS (3 tests). Also run `cargo test -p cairn-event` to confirm no regression.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-event/src/demographics.rs
git commit -m "feat(event): §4.2 name assertion builder and legibility twin"
```

---

### Task 2: `db/012` retained-set + display-winner projection (integration-tested)

**Files:**
- Create: `db/012_demographics_names.sql`
- Modify: `crates/cairn-node/src/db.rs:3-18` (add `012` to the `SCHEMA` array, bump its declared length `10` → `11`)
- Create/Test: `crates/cairn-node/tests/demographics_names.rs`

**Interfaces:**
- Consumes: `name_assertion_body`, `render_name_twin` (Task 1); `cairn_provenance_rank(text)→int` (db/011); the unchanged `demographic.field.asserted` event type, generic floor `cairn_check_demographic_field`, and `cairn_event_twin` twin enforcement (db/011); `db::connect_and_load_schema`, `db::test_serial_guard` (cairn-node).
- Produces (SQL objects later code/UI reads):
  - table `patient_name (patient_id, use_key, value, use_raw, provenance, provenance_rank, last_hlc_wall, last_hlc_count, asserted_origin, updated_at)`, PK `(patient_id, use_key, value)`
  - view `patient_name_current` (one display-winner row per `patient_id`)
  - trigger `patient_name_apply_trg` on `event_log`

- [ ] **Step 1: Write the failing integration test file**

Create `crates/cairn-node/tests/demographics_names.rs`. This mirrors the slice-2 harness (`demographics_fields.rs`) — same `setup`/`submit_field` shape, but the `setup` TRUNCATE list adds `patient_name`, and `submit_field` carries a `counter` so equal-`wall` ties can be ordered when needed.

```rust
//! Integration coverage for the §4.2 NAMES field: the retained-set patient_name
//! projection (most-recent-assertion-per-member) + the patient_name_current
//! display-winner VIEW (legal-preferred, recency-first, any-use fallback). Real
//! Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via
//! `db::test_serial_guard`. Matching (§5.2) is a separate subsystem, not exercised here.
use cairn_event::demographics::{name_assertion_body, render_name_twin};
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

async fn setup(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name CASCADE").await.unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    ).await.unwrap();
    (sk, kid)
}

/// Author + sign + submit one demographic.field.asserted event. `wall`/`counter`
/// set the HLC so recency ties can be ordered deterministically in tests.
async fn submit_field(
    c: &Client, sk: &SigningKey, kid: &str, patient: Uuid, wall: i64, counter: i64,
    payload: serde_json::Value, twin: Option<&str>,
) -> Result<u64, tokio_postgres::Error> {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: "demographic.field/1".into(),
        hlc: Hlc { wall, counter, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: twin.map(|t| t.to_string()),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await
}

/// The current display name for a patient (NULL-safe: returns None if no row).
async fn current_name(c: &Client, p: Uuid) -> Option<String> {
    let p_str = p.to_string();
    c.query_opt(
        "SELECT value FROM patient_name_current WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().map(|r| r.get(0))
}

/// Count retained name members for a patient.
async fn name_count(c: &Client, p: Uuid) -> i64 {
    let p_str = p.to_string();
    c.query_one(
        "SELECT count(*) FROM patient_name WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0)
}

#[tokio::test]
async fn happy_path_and_retained_set() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // A legal, a maiden, and an alias — ALL retained as evidence; current = the legal.
    for (val, use_, prov, wall) in [
        ("Mary Jones", "legal", "document-verified", 3),
        ("Mary Smith", "maiden", "patient-stated", 1),
        ("MJ", "alias", "patient-stated", 2),
    ] {
        submit_field(&c, &sk, &kid, p, wall, 0,
            name_assertion_body(val, Some(use_), prov),
            Some(&render_name_twin(val, Some(use_), prov))).await.unwrap();
    }
    assert_eq!(name_count(&c, p).await, 3, "all three names retained as evidence");
    assert_eq!(current_name(&c, p).await.as_deref(), Some("Mary Jones"), "legal name is the display winner");
    // The authored twin was carried verbatim for the legal name.
    let p_str = p.to_string();
    let twin: String = c.query_one(
        "SELECT plaintext_twin FROM event_log WHERE patient_id::text=$1 AND hlc_wall=3",
        &[&p_str]).await.unwrap().get(0);
    assert_eq!(twin, "Name (legal): Mary Jones");
}

#[tokio::test]
async fn recency_first_within_legal_diverges_from_dob() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // Old, HIGHER-provenance legal name (document-verified, wall=1).
    submit_field(&c, &sk, &kid, p, 1, 0,
        name_assertion_body("Mary Smith", Some("legal"), "document-verified"),
        Some(&render_name_twin("Mary Smith", Some("legal"), "document-verified"))).await.unwrap();
    assert_eq!(current_name(&c, p).await.as_deref(), Some("Mary Smith"), "first legal name displays");
    // Newer, LOWER-provenance legal name (patient-stated, wall=2). For NAMES, recency
    // wins — the opposite of DOB's provenance-lock. The current name she goes by shows.
    submit_field(&c, &sk, &kid, p, 2, 0,
        name_assertion_body("Mary Jones", Some("legal"), "patient-stated"),
        Some(&render_name_twin("Mary Jones", Some("legal"), "patient-stated"))).await.unwrap();
    assert_eq!(current_name(&c, p).await.as_deref(), Some("Mary Jones"),
        "newer legal name wins display (recency beats provenance for names)");
    assert_eq!(name_count(&c, p).await, 2, "the old name is retained, not overwritten");
}

#[tokio::test]
async fn no_legal_name_falls_back_to_most_recent_any_use() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // Unidentified patient: only a triage alias exists — it MUST still display (paper-parity).
    submit_field(&c, &sk, &kid, p, 1, 0,
        name_assertion_body("Unknown Male ~40", Some("alias"), "clinician-observed"),
        Some(&render_name_twin("Unknown Male ~40", Some("alias"), "clinician-observed"))).await.unwrap();
    assert_eq!(current_name(&c, p).await.as_deref(), Some("Unknown Male ~40"),
        "alias displays when no legal name exists");
}

#[tokio::test]
async fn legal_name_takes_over_from_a_newer_alias() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // A legal name asserted EARLY (wall=1), then a NEWER alias (wall=2). The legal tier
    // always outranks any non-legal, so the legal name stays the display winner even
    // though the alias is more recent.
    submit_field(&c, &sk, &kid, p, 1, 0,
        name_assertion_body("Mary Jones", Some("legal"), "patient-stated"),
        Some(&render_name_twin("Mary Jones", Some("legal"), "patient-stated"))).await.unwrap();
    submit_field(&c, &sk, &kid, p, 2, 0,
        name_assertion_body("MJ", Some("alias"), "patient-stated"),
        Some(&render_name_twin("MJ", Some("alias"), "patient-stated"))).await.unwrap();
    assert_eq!(current_name(&c, p).await.as_deref(), Some("Mary Jones"),
        "legal tier outranks a newer alias");
}

#[tokio::test]
async fn set_union_reassertion_is_idempotent() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // The same (use, value) re-asserted at a later HLC stays ONE member (its representative
    // advances); never a duplicate row.
    for wall in [1, 2] {
        submit_field(&c, &sk, &kid, p, wall, 0,
            name_assertion_body("Mary Jones", Some("legal"), "patient-stated"),
            Some(&render_name_twin("Mary Jones", Some("legal"), "patient-stated"))).await.unwrap();
    }
    assert_eq!(name_count(&c, p).await, 1, "re-assertion of the same name dedupes to one member");
    assert_eq!(current_name(&c, p).await.as_deref(), Some("Mary Jones"));
}

#[tokio::test]
async fn cross_field_isolation_with_dob() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // A name event must NOT create a patient_demographic row; a dob event must NOT create
    // a patient_name row. The two projections are blind to each other's fields.
    submit_field(&c, &sk, &kid, p, 1, 0,
        name_assertion_body("Mary Jones", Some("legal"), "patient-stated"),
        Some(&render_name_twin("Mary Jones", Some("legal"), "patient-stated"))).await.unwrap();
    submit_field(&c, &sk, &kid, p, 2, 0,
        serde_json::json!({"field":"dob","value":"1980","provenance":"patient-stated",
                           "facets":{"precision":"year"}}),
        Some("Date of birth (patient-stated): 1980 (year)")).await.unwrap();

    let p_str = p.to_string();
    let names: i64 = c.query_one(
        "SELECT count(*) FROM patient_name WHERE patient_id::text=$1", &[&p_str]).await.unwrap().get(0);
    let demos: i64 = c.query_one(
        "SELECT count(*) FROM patient_demographic WHERE patient_id::text=$1", &[&p_str]).await.unwrap().get(0);
    assert_eq!(names, 1, "only the name event projects into patient_name");
    assert_eq!(demos, 1, "only the dob event projects into patient_demographic");
}

#[tokio::test]
async fn floor_rejects_empty_name_value() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // The generic §4.2 floor rejects an empty value — for field="name" too (no name-specific
    // floor code exists; the generic invariant covers it). Nothing is appended or projected.
    let r = submit_field(&c, &sk, &kid, p, 1, 0,
        serde_json::json!({"field":"name","value":"","provenance":"patient-stated"}),
        Some("Name (patient-stated): x")).await;
    assert!(r.is_err(), "empty name value is rejected by the floor");
    let p_str = p.to_string();
    let n: i64 = c.query_one(
        "SELECT count(*) FROM event_log WHERE patient_id::text=$1", &[&p_str]).await.unwrap().get(0);
    assert_eq!(n, 0, "rejected name is not appended");
    assert_eq!(name_count(&c, p).await, 0, "rejected name is not projected");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test demographics_names -- --nocapture`
Expected: FAIL — the schema load errors or the queries error with `relation "patient_name" does not exist` (the projection isn't created yet). (If `CAIRN_TEST_PG` is unset the tests *skip*, which is not a real pass — set it.)

- [ ] **Step 3: Create the `db/012` projection**

Create `db/012_demographics_names.sql`:

```sql
-- Cairn — demographic NAMES: the retained-set + display-winner projection (spec §4.2).
--
-- Slice 3 of the demographics subsystem. Names are the first field that needs BOTH a
-- retained set (every name kept as matching evidence) AND a single display-winner
-- selected from it. A name reuses the slice-2 generic `demographic.field.asserted`
-- event with field='name'; the generic floor (db/011 cairn_check_demographic_field)
-- and the authored-twin enforcement already accept it, so this migration adds NO floor
-- change and NO new event type — only the projection. The display-winner is a VIEW
-- (a pure deterministic function of the set), so there is no winner-pointer to maintain.
-- Matching (§5.2) is a separate, later subsystem.

BEGIN;

-- The §4.2 retained set: one row per distinct (patient, use, value) name. use_key
-- folds an absent/blank `use` to 'unspecified' so it is a valid NOT-NULL key component
-- (mirrors patient_identifier.match_key). provenance_rank is cached (reuses db/011's
-- cairn_provenance_rank) so the trigger's recency/provenance test is a plain tuple compare.
CREATE TABLE IF NOT EXISTS patient_name (
    patient_id         UUID    NOT NULL,
    use_key            TEXT    NOT NULL,   -- coalesce(NULLIF(trim(use),''),'unspecified')
    value              TEXT    NOT NULL,   -- the authored display string (opaque to the core)
    use_raw            TEXT,               -- the original `use` facet (NULL when absent)
    provenance         TEXT    NOT NULL,
    provenance_rank    INT     NOT NULL,
    last_hlc_wall      BIGINT  NOT NULL,
    last_hlc_count     INTEGER NOT NULL,
    asserted_origin    TEXT    NOT NULL,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (patient_id, use_key, value)
);

-- Incremental maintenance: fold exactly the one new name event into the retained set.
-- event_log.body holds b->'payload' (see db/005 submit_event INSERT).
CREATE OR REPLACE FUNCTION patient_name_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p      jsonb := NEW.body;
    fld    text  := p ->> 'field';
    v_use  text  := NULLIF(trim(p -> 'facets' ->> 'use'), '');
    v_key  text;
    v_rank int;
BEGIN
    -- Only NAME events project here. dob/sex-at-birth (db/011) and any unknown field
    -- are ignored — names get their own multi-valued shape. (This trigger and the
    -- patient_demographic trigger both fire on demographic.field.asserted; each gates
    -- to its own fields and writes a different table, so order is irrelevant.)
    IF fld <> 'name' THEN
        RETURN NULL;
    END IF;
    v_key  := coalesce(v_use, 'unspecified');
    v_rank := cairn_provenance_rank(p ->> 'provenance');

    INSERT INTO patient_name AS pn
        (patient_id, use_key, value, use_raw, provenance, provenance_rank,
         last_hlc_wall, last_hlc_count, asserted_origin)
    VALUES
        (NEW.patient_id, v_key, p ->> 'value', v_use, p ->> 'provenance', v_rank,
         NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
    -- Per (patient, use, value) member, keep the MOST-RECENT assertion as its
    -- representative (recency-first tuple, matching the display rule). The compare is a
    -- deterministic, apply-order-independent function of the member's assertion set, so
    -- every node converges to the same row. A re-assertion that does not advance the
    -- tuple leaves the row unchanged (set-union idempotency).
    ON CONFLICT (patient_id, use_key, value) DO UPDATE SET
        use_raw         = EXCLUDED.use_raw,
        provenance      = EXCLUDED.provenance,
        provenance_rank = EXCLUDED.provenance_rank,
        last_hlc_wall   = EXCLUDED.last_hlc_wall,
        last_hlc_count  = EXCLUDED.last_hlc_count,
        asserted_origin = EXCLUDED.asserted_origin,
        updated_at      = clock_timestamp()
    WHERE (EXCLUDED.last_hlc_wall, EXCLUDED.last_hlc_count,
           EXCLUDED.provenance_rank, EXCLUDED.asserted_origin)
        > (pn.last_hlc_wall, pn.last_hlc_count,
           pn.provenance_rank, pn.asserted_origin);
    RETURN NULL;
END;
$$;

DROP TRIGGER IF EXISTS patient_name_apply_trg ON event_log;
CREATE TRIGGER patient_name_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'demographic.field.asserted')
    EXECUTE FUNCTION patient_name_apply();

-- The §4.2 display-winner: one row per patient, selected from the retained set with NO
-- stored pointer. The ORDER BY is the whole rule:
--   1) prefer use_key='legal' (a legal name always outranks any non-legal — a 2010 legal
--      beats a 2024 alias);
--   2) recency-first within the tier (newest legal name wins — recency beats provenance
--      for names, the deliberate divergence from DOB's provenance-lock);
--   3) provenance_rank then asserted_origin break exact-recency ties deterministically.
-- When no legal name exists, the newest name of ANY use wins (the unidentified-patient
-- fallback) — paper-parity: the chart header always shows something.
CREATE OR REPLACE VIEW patient_name_current AS
SELECT DISTINCT ON (patient_id)
    patient_id, use_key, value, use_raw, provenance, provenance_rank,
    last_hlc_wall, last_hlc_count, asserted_origin, updated_at
FROM patient_name
ORDER BY patient_id,
         (use_key = 'legal') DESC,
         last_hlc_wall DESC, last_hlc_count DESC,
         provenance_rank DESC, asserted_origin DESC;

GRANT SELECT ON patient_name, patient_name_current TO cairn_agent;

COMMIT;
```

- [ ] **Step 4: Register the migration in the node schema loader**

In `crates/cairn-node/src/db.rs`, bump the array length and append the entry after `011`:

```rust
const SCHEMA: [(&str, &str); 11] = [
    // ... existing entries 001..011 unchanged ...
    ("011_demographics_fields", include_str!("../../../db/011_demographics_fields.sql")),
    ("012_demographics_names",  include_str!("../../../db/012_demographics_names.sql")),
];
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test demographics_names -- --nocapture`
Expected: PASS (7 tests). If a winner test fails, re-check the view `ORDER BY` tuple order against the rule in Step 3's comment — do not change the test expectations.

- [ ] **Step 6: Full-suite + lint gate**

Run: `cargo test -p cairn-event && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all green, no clippy warnings. (Confirms slice-1/2 demographics tests still pass — no projection regressed.)

- [ ] **Step 7: Commit**

```bash
git add db/012_demographics_names.sql crates/cairn-node/src/db.rs crates/cairn-node/tests/demographics_names.rs
git commit -m "feat(db): §4.2 names retained-set projection + display-winner view"
```

---

### Task 3: ADR-0036 + §4.2 spec refinement + doc currency

**Files:**
- Create: `docs/spec/decisions/0036-demographic-name-display-recency-first.md`
- Modify: `docs/spec/demographics.md` (the §4.2 names table row + a rationale note)
- Modify: `docs/spec/decisions/README.md` (ADR index, if present) and `docs/spec/index.md` (spec version bump 0.36 → 0.37 if the file tracks it)
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md` (mark slice 3 done; trim the next-slice menu)

**Interfaces:** Documentation only — no code, no test.

- [ ] **Step 1: Write ADR-0036**

Create `docs/spec/decisions/0036-demographic-name-display-recency-first.md`, following the house ADR format (read a recent one, e.g. `0035-entities-relationships-and-provider-numbers.md`, for the exact heading/status/date structure). Content must capture:
  - **Status:** Accepted · **Date:** 2026-06-28 · refines §4.2 (and the §4.1 provenance ladder’s *application* to names).
  - **Context:** §4.2 originally read *"display = highest-provenance recent legal name"* (provenance-first, like DOB). Names are a **volatile, legitimately-changing** identity field; provenance-first pins a stale married name or a **deadname** over a current patient-stated legal name — a dignity *and* safety failure (paper-parity: you call the patient by the name they give).
  - **Decision:** the names display-winner is **recency-first within the legal tier**, provenance/origin breaking ties; when no legal name exists it **falls back to the most-recent name of any `use`** (the unidentified-patient case). **All names are retained** regardless; provenance still feeds the §5.2 matcher. The displayed name is the **legal-preferred reference point**; surfacing a preferred/chosen name as an "a.k.a." is **UI soft-policy above the floor** (principle 12), reading the same retained set.
  - **Consequences / why not provenance-first:** a verified old name no longer locks display; correctness of the *current* name is prioritised over evidence-strength for display (evidence strength is preserved in `event_log` + the matcher). DOB keeps its provenance-lock (DOB does not change); names diverge **by design** — do not "fix" them back.

- [ ] **Step 2: Refine §4.2 in `docs/spec/demographics.md`**

Change the Names row of the §4.2 table from:

```
| Names | Multi-valued set (legal, maiden, alias, transliteration) | All retained; display = highest-provenance recent legal name | Weak evidence |
```

to:

```
| Names | Multi-valued set (legal, maiden, alias, transliteration) | All retained; display = **most-recent legal name (recency-first; provenance and origin break ties)**, falling back to the most-recent name of any `use` when no legal name exists ([ADR-0036](decisions/0036-demographic-name-display-recency-first.md)) | Weak evidence |
```

Add a sentence to the §4.2 Notes block:

```
- Names are recency-first (not provenance-locked like DOB): names legitimately change
  (marriage, transition), so the current name the patient goes by displays; the old name
  is retained as evidence. A verified document does not pin a stale name or a deadname
  ([ADR-0036](decisions/0036-demographic-name-display-recency-first.md)).
```

- [ ] **Step 3: Update the ADR index + spec version**

If `docs/spec/decisions/README.md` carries an ADR table, add the `0036` row. In `docs/spec/index.md`, bump the stated spec version (0.36 → 0.37) if it is tracked there. (Grep for `0.36` to find the version string.)

- [ ] **Step 4: Update HANDOVER + ROADMAP**

In `docs/HANDOVER.md`: add a short "This session" entry for slice 3 (names retained-set + display-winner view + ADR-0036), and move names off the "next slices" menu. In `docs/ROADMAP.md` Phase 4: mark the §4.2 names field done; update the "Next:" list to drop names and lead with administrative-sex + gender-identity / §4.3 address. Keep both files concise (< 500 lines).

- [ ] **Step 5: Commit**

```bash
git add docs/
git commit -m "spec(demographics): §4.2 names recency-first display (ADR-0036) + doc currency"
```

---

## Self-Review

**1. Spec coverage** — every design-doc section maps to a task:
- Scalar display value + reuse generic event/floor → Task 1 (builder) + Task 2 (no floor/event change; verified by `floor_rejects_empty_name_value` and cross-field isolation).
- Retained set `patient_name` (recency-first per-member rep) → Task 2 SQL + `happy_path_and_retained_set`, `set_union_reassertion_is_idempotent`.
- Display-winner view (legal-preferred, recency-first, any-use fallback) → Task 2 view + `recency_first_within_legal_diverges_from_dob`, `no_legal_name_falls_back_to_most_recent_any_use`, `legal_name_takes_over_from_a_newer_alias`.
- §4.5 twin → Task 1 `render_name_twin` + twin assertion in `happy_path_and_retained_set`.
- UI "a.k.a." seam → documented in ADR-0036 (Task 3), no code (correctly out of scope).
- ADR-0036 + §4.2 refinement → Task 3.
- Cross-field isolation (ADR-0012 federation-forward) → `cross_field_isolation_with_dob`.

**2. Placeholder scan** — no TBD/TODO; all code shown in full; the one "follow the house ADR format" step points at a concrete exemplar file rather than inventing structure.

**3. Type consistency** — `name_assertion_body(value, use_, provenance)` and `render_name_twin(value, use_, provenance)` signatures match between Task 1 and Task 2's test usage. SQL object/column names (`patient_name`, `patient_name_current`, `use_key`, `last_hlc_wall`, `last_hlc_count`, `asserted_origin`) match between the DDL, the trigger, the view, and the test queries. `NEW.hlc_counter` (event_log column) → `last_hlc_count` (projection column) mapping matches the slice-2 trigger convention.
