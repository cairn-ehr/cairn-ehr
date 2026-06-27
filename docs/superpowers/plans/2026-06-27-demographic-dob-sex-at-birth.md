# Demographic DOB + sex-at-birth (slice 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the two §4.2 provenance-locked single-valued demographic fields — DOB and sex-at-birth — end-to-end: pure Rust builders/twins → an in-DB structural floor → a provenance-precedence `patient_demographic` projection.

**Architecture:** A single generic `demographic.field.asserted` event (a `field` discriminator) flows through the reused `submit_event` door; a `cairn_provenance_rank` ladder + a winner-by-`(rank, HLC)` projection select the current display value. The floor stays open (an unknown field is stored + legible but not projected — federation-forward per ADR-0012); the projection trigger holds the per-field policy. Full assertion history lives in `event_log` as the matching evidence.

**Tech Stack:** Rust (`cairn-event` pure crate, `cairn-node` integration tests with `tokio-postgres`), PostgreSQL 18 + the `cairn_pgx` extension, PL/pgSQL + SQL migrations.

**Design doc:** [`docs/superpowers/specs/2026-06-27-demographic-dob-sex-at-birth-design.md`](../specs/2026-06-27-demographic-dob-sex-at-birth-design.md)

## Global Constraints

- **AGPL-3.0**; every dependency AGPL-3.0-compatible (no new deps in this slice).
- **TDD** — failing test first, then the code that makes it pass. Load-bearing on this safety-critical surface.
- **Inline docs for a junior contributor** — every non-trivial fn/migration block explains *why* and *how it fits*, not just *what*.
- **Files < 500 lines** where feasible.
- **The in-DB floor is culture-neutral**: it enforces only structural invariants — never parses/validates a date, never enforces a sex vocabulary, never holds a profile, never rejects on validation (principle 12).
- **Provenance ladder** (the §4.1 order, encoded by `cairn_provenance_rank`): `fact-proven 70 · document-verified 60 · patient-stated 50 · third-party-stated 40 · clinician-observed 30 · imported/unknown 20 · inferred 10 · (unrecognized) 0`.
- **DB-gated integration tests** need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + `cairn_pgx`); they self-serialize cluster-wide via `db::test_serial_guard` and skip cleanly when the env var is unset.
- **`submit_event` (db/005) is reused verbatim** — never re-declared. Demographics' only write-path change is the `cairn_event_twin` hook.

## File Structure

| File | Responsibility |
|---|---|
| `crates/cairn-event/src/demographics.rs` | Pure payload builders + twin renderers for the new fields (extends the slice-1 module) |
| `db/011_demographics_fields.sql` | The rank fn, the structural floor dispatcher, the `patient_demographic` projection + trigger, and the `cairn_event_twin` re-declaration |
| `crates/cairn-node/src/db.rs:3-17` | Register `011` in the `SCHEMA` array |
| `crates/cairn-node/tests/demographics_fields.rs` | Integration tests: happy path, precedence, floor rejections, unknown-field gate, regression |
| `docs/spec/demographics.md` | §4.1 ladder prose gains `fact-proven` as the top tier |

---

### Task 1: Rust pure builders + twin renderers

**Files:**
- Modify: `crates/cairn-event/src/demographics.rs` (append new fns + unit tests; the module is 89 lines today, stays < 500)
- Test: same file (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `serde_json::{json, Value}` (already imported in the module).
- Produces (relied on by Task 2's integration test):
  - `demographic_field_body(field: &str, value: &str, facets: Option<serde_json::Value>, provenance: &str) -> serde_json::Value`
  - `dob_assertion_body(value: &str, precision: &str, basis: Option<&str>, provenance: &str) -> serde_json::Value`
  - `sex_at_birth_assertion_body(value: &str, provenance: &str) -> serde_json::Value`
  - `render_dob_twin(value: &str, precision: &str, provenance: &str) -> String`
  - `render_sex_at_birth_twin(value: &str, provenance: &str) -> String`

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block in `crates/cairn-event/src/demographics.rs`:

```rust
    #[test]
    fn dob_body_carries_field_value_provenance_and_facets() {
        let v = dob_assertion_body("1980-07-15", "day", Some("document"), "document-verified");
        assert_eq!(v["field"], "dob");
        assert_eq!(v["value"], "1980-07-15");
        assert_eq!(v["provenance"], "document-verified");
        assert_eq!(v["facets"]["precision"], "day");
        assert_eq!(v["facets"]["basis"], "document");
    }

    #[test]
    fn dob_body_omits_absent_basis_never_null() {
        let v = dob_assertion_body("1980", "year", None, "patient-stated");
        assert_eq!(v["facets"]["precision"], "year");
        let facets = v["facets"].as_object().unwrap();
        assert!(!facets.contains_key("basis"), "absent basis must be omitted, not null");
    }

    #[test]
    fn sex_at_birth_body_has_no_facets() {
        let v = sex_at_birth_assertion_body("female", "clinician-observed");
        assert_eq!(v["field"], "sex-at-birth");
        assert_eq!(v["value"], "female");
        assert_eq!(v["provenance"], "clinician-observed");
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("facets"), "sex-at-birth carries no facets bag");
    }

    #[test]
    fn twins_render_profile_independent_plaintext() {
        assert_eq!(
            render_dob_twin("1980", "year", "patient-stated"),
            "Date of birth (patient-stated): 1980 (year)"
        );
        assert_eq!(
            render_sex_at_birth_twin("female", "clinician-observed"),
            "Sex at birth (clinician-observed): female"
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cairn-event demographics`
Expected: FAIL — `cannot find function dob_assertion_body` (etc.).

- [ ] **Step 3: Write the minimal implementation**

Append these public fns to `crates/cairn-event/src/demographics.rs` (above the `#[cfg(test)]` block). Match the existing module's doc-comment density:

```rust
/// Build a generic §4.2 demographic-field assertion payload (the value of
/// `EventBody.payload`). `field` is the discriminator a node's projection keys on;
/// `facets` is an optional per-field bag (DOB's precision/basis), omitted entirely
/// when absent so the in-DB floor's key-presence checks see exactly what was asserted.
pub fn demographic_field_body(
    field: &str, value: &str, facets: Option<Value>, provenance: &str,
) -> Value {
    let mut p = json!({ "field": field, "provenance": provenance, "value": value });
    let obj = p.as_object_mut().expect("json! built an object");
    if let Some(f) = facets { obj.insert("facets".into(), f); }
    p
}

/// One §4.2 date-of-birth assertion. `precision` is mandatory (principle 4 — a date
/// must declare how precise it is; the in-DB floor rejects a dob with no precision).
/// `basis` (how the date was derived) is optional and omitted when `None`.
pub fn dob_assertion_body(
    value: &str, precision: &str, basis: Option<&str>, provenance: &str,
) -> Value {
    let mut facets = json!({ "precision": precision });
    if let Some(b) = basis {
        facets.as_object_mut().unwrap().insert("basis".into(), json!(b));
    }
    demographic_field_body("dob", value, Some(facets), provenance)
}

/// One §4.2 sex-at-birth assertion. `value` is an OPEN string — intersex /
/// indeterminate / unknown must be recordable (principle 4); never a closed enum.
pub fn sex_at_birth_assertion_body(value: &str, provenance: &str) -> Value {
    demographic_field_body("sex-at-birth", value, None, provenance)
}

/// Render the §4.5 materialised legibility twin for a date of birth:
/// `"Date of birth (<provenance>): <value> (<precision>)"`. Profile-independent —
/// readable on a node that has never seen the dob field's schema.
pub fn render_dob_twin(value: &str, precision: &str, provenance: &str) -> String {
    format!("Date of birth ({provenance}): {value} ({precision})")
}

/// Render the §4.5 legibility twin for sex-at-birth:
/// `"Sex at birth (<provenance>): <value>"`.
pub fn render_sex_at_birth_twin(value: &str, provenance: &str) -> String {
    format!("Sex at birth ({provenance}): {value}")
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cairn-event demographics`
Expected: PASS (the four new tests plus the three pre-existing slice-1 tests).

- [ ] **Step 5: Lint + commit**

```bash
cargo clippy -p cairn-event --all-targets -- -D warnings
git add crates/cairn-event/src/demographics.rs
git commit -m "feat(event): §4.2 dob + sex-at-birth assertion builders and twins"
```

---

### Task 2: The in-DB spine — rank fn, floor, projection, twin hook (happy path)

**Files:**
- Create: `db/011_demographics_fields.sql`
- Modify: `crates/cairn-node/src/db.rs:3-17` (register `011` in `SCHEMA`)
- Test: `crates/cairn-node/tests/demographics_fields.rs` (create; the happy-path test)

**Interfaces:**
- Consumes: Task 1's `dob_assertion_body` / `sex_at_birth_assertion_body` / `render_dob_twin` / `render_sex_at_birth_twin`; the existing `cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey}`, `cairn_node::db`, and the `submit_event` / `enroll_actor` SQL surface.
- Produces (relied on by Tasks 3–4): the `demographic.field.asserted` event type, `cairn_provenance_rank(text)`, `cairn_check_demographic_field(jsonb)`, the `patient_demographic` table, and a `setup` / `assert_field` test helper pair in the new test file.

- [ ] **Step 1: Write the failing happy-path test**

Create `crates/cairn-node/tests/demographics_fields.rs`:

```rust
//! Integration coverage for the §4.2 provenance-precedence fields (DOB +
//! sex-at-birth): the in-DB structural floor + the winner-by-(rank,HLC)
//! patient_demographic projection. Real Postgres, gated on `$CAIRN_TEST_PG`,
//! serialized cluster-wide via `db::test_serial_guard`. Matching (§5.2) is a
//! separate subsystem and is NOT exercised here.
use cairn_event::demographics::{
    dob_assertion_body, render_dob_twin, render_sex_at_birth_twin, sex_at_birth_assertion_body,
};
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

/// Truncate the clinical tables and enroll one agent signer. Returns (sk, kid).
async fn setup(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, patient_demographic CASCADE")
        .await.unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    ).await.unwrap();
    (sk, kid)
}

/// Author + sign + submit one demographic.field.asserted event. `payload` is the
/// already-built body (from a Task-1 builder or a raw json! for rejection tests);
/// `twin` is the authored §4.5 twin. Returns the raw submit result.
async fn submit_field(
    c: &Client, sk: &SigningKey, kid: &str, patient: Uuid, wall: i64,
    payload: serde_json::Value, twin: Option<&str>,
) -> Result<u64, tokio_postgres::Error> {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: "demographic.field/1".into(),
        hlc: Hlc { wall, counter: 0, node_origin: "n".into() },
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

#[tokio::test]
async fn happy_path_projects_dob_and_sex_with_rank_and_facets() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    submit_field(&c, &sk, &kid, p, 1,
        dob_assertion_body("1980-07-15", "day", Some("document"), "document-verified"),
        Some(&render_dob_twin("1980-07-15", "day", "document-verified"))
    ).await.expect("valid dob accepted");
    submit_field(&c, &sk, &kid, p, 2,
        sex_at_birth_assertion_body("female", "clinician-observed"),
        Some(&render_sex_at_birth_twin("female", "clinician-observed"))
    ).await.expect("valid sex-at-birth accepted");

    let p_str = p.to_string();
    // DOB row: value, cached provenance_rank (document-verified -> 60), facets.precision.
    let row = c.query_one(
        "SELECT value, provenance_rank, facets->>'precision' \
         FROM patient_demographic WHERE patient_id::text=$1 AND field='dob'",
        &[&p_str]).await.unwrap();
    let value: String = row.get(0);
    let rank: i32 = row.get(1);
    let precision: String = row.get(2);
    assert_eq!(value, "1980-07-15");
    assert_eq!(rank, 60);
    assert_eq!(precision, "day");
    // sex-at-birth row exists with the right value.
    let sex: String = c.query_one(
        "SELECT value FROM patient_demographic WHERE patient_id::text=$1 AND field='sex-at-birth'",
        &[&p_str]).await.unwrap().get(0);
    assert_eq!(sex, "female");
    // The AUTHORED dob twin was carried verbatim (cairn_event_twin demographic branch).
    let twin: String = c.query_one(
        "SELECT plaintext_twin FROM event_log WHERE patient_id::text=$1 AND hlc_wall=1",
        &[&p_str]).await.unwrap().get(0);
    assert_eq!(twin, "Date of birth (document-verified): 1980-07-15 (day)");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test demographics_fields happy_path -- --nocapture`
Expected: FAIL — the schema load errors (`relation "patient_demographic" does not exist` / unknown event type), because `db/011` does not exist yet.

- [ ] **Step 3: Write the migration `db/011_demographics_fields.sql`**

```sql
-- Cairn — demographic provenance-precedence fields: DOB + sex-at-birth (spec §4.1/§4.2/§4.5).
--
-- Slice 2 of the demographics subsystem. Adds the generic `demographic.field.asserted`
-- event type, the culture-neutral §4.2 structural floor (no date parsing, no sex
-- vocabulary — those are advisory, above the floor), the §4.1 provenance ladder as a
-- rank function, and the winner-by-(rank, HLC) `patient_demographic` projection. The
-- §4.5 authored twin is carried via the cairn_event_twin hook (NOT by re-declaring the
-- validated submit_event door). Matching (§5.2) is a separate, later subsystem.

BEGIN;

-- Additive registration of the new event type (fail-closed registry, ADR-0010).
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('demographic.field.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- The §4.1 provenance ladder as a total order. fact-proven (70) is a new top tier
-- above document-verified (60): laboratory/scientifically-established truth (a
-- karyotype, a confirmed assay) can override what an official document merely
-- attests. An UNRECOGNIZED string ranks 0 (below inferred) — the safe default: a
-- term from a newer ladder, or a typo, can never DISPLACE a known-provenance value,
-- and a node that doesn't know a peer's newer term degrades to "lowest", never
-- "highest" (federation-safe). IMMUTABLE so it is index/trigger-safe.
CREATE OR REPLACE FUNCTION cairn_provenance_rank(p text)
RETURNS int LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE p
        WHEN 'fact-proven'        THEN 70
        WHEN 'document-verified'  THEN 60
        WHEN 'patient-stated'     THEN 50
        WHEN 'third-party-stated' THEN 40
        WHEN 'clinician-observed' THEN 30
        WHEN 'imported'           THEN 20
        WHEN 'unknown'            THEN 20
        WHEN 'inferred'           THEN 10
        ELSE 0
    END;
$$;

-- The §4.2 structural floor for a generic demographic field assertion. Enforces ONLY
-- culture-neutral invariants; never parses a date, never validates a sex vocabulary,
-- never rejects on validation (principle 12). Per-field structural checks apply only
-- to fields THIS node knows — an unknown field passes the generic checks (it is still
-- stored in event_log and legible via its twin; the PROJECTION, not the floor, is what
-- is gated to known fields). Each violation is a distinct legible exception.
CREATE OR REPLACE FUNCTION cairn_check_demographic_field(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p   jsonb := b -> 'payload';
    fld text;
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'demographic field assertion: missing payload';
    END IF;
    -- field: the discriminator the projection keys on (§4.2).
    IF jsonb_typeof(p -> 'field') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'field')) = 0 THEN
        RAISE EXCEPTION 'demographic field assertion: field must be a non-empty string';
    END IF;
    -- provenance: the §4.1 ladder term — required-present, value-open.
    IF jsonb_typeof(p -> 'provenance') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'provenance')) = 0 THEN
        RAISE EXCEPTION 'demographic field assertion: provenance must be a non-empty string (§4.1)';
    END IF;
    -- value: the core scalar (§4.2). Open string — never a closed enum.
    IF jsonb_typeof(p -> 'value') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'value')) = 0 THEN
        RAISE EXCEPTION 'demographic field assertion: value must be a non-empty string';
    END IF;

    fld := p ->> 'field';
    -- Per-field structural dispatch (known fields only).
    IF fld = 'dob' THEN
        -- precision is mandatory: a date must declare how precise it is (principle 4 —
        -- never an unqualified exact date by default). The floor does NOT parse the
        -- date value — a half-recalled "1980, year-only" must record.
        IF jsonb_typeof(p -> 'facets' -> 'precision') IS DISTINCT FROM 'string'
           OR length(trim(p -> 'facets' ->> 'precision')) = 0 THEN
            RAISE EXCEPTION 'demographic field assertion: dob requires a non-empty facets.precision (principle 4)';
        END IF;
        -- basis is optional; when present it must be non-empty text.
        IF (p -> 'facets' ? 'basis') AND (p -> 'facets' -> 'basis') IS DISTINCT FROM 'null'::jsonb THEN
            IF jsonb_typeof(p -> 'facets' -> 'basis') IS DISTINCT FROM 'string'
               OR length(trim(p -> 'facets' ->> 'basis')) = 0 THEN
                RAISE EXCEPTION 'demographic field assertion: dob facets.basis must be non-empty text when present';
            END IF;
        END IF;
    END IF;
    -- sex-at-birth: no extra structural requirement (value-open).
    -- unknown field: generic checks only — carried, legible, not projected.
END;
$$;

-- The §4.2 provenance-precedence projection: one row per (patient, field) holding the
-- current DISPLAY winner. Full assertion history (the matching evidence) stays in
-- event_log — this is the projected current truth, an overlay, never an edit
-- (principle 2). provenance_rank is cached so the trigger's winner test is a plain
-- tuple compare. `value` is the core scalar; `facets` carries field-specific extras.
CREATE TABLE IF NOT EXISTS patient_demographic (
    patient_id         UUID    NOT NULL,
    field              TEXT    NOT NULL,   -- 'dob' | 'sex-at-birth' (known fields only)
    value              TEXT    NOT NULL,
    facets             JSONB,
    provenance         TEXT    NOT NULL,
    provenance_rank    INT     NOT NULL,
    asserted_hlc_wall  BIGINT  NOT NULL,
    asserted_hlc_count INTEGER NOT NULL,
    asserted_origin    TEXT    NOT NULL,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (patient_id, field)
);

-- Incremental maintenance: fold exactly the one new field event into the projection.
-- event_log.body holds b->'payload' (see db/005 submit_event INSERT).
CREATE OR REPLACE FUNCTION patient_demographic_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p      jsonb := NEW.body;
    fld    text  := p ->> 'field';
    v_rank int   := cairn_provenance_rank(p ->> 'provenance');
BEGIN
    -- Projection gate: only known single-valued fields project. An unknown field
    -- (e.g. a newer node's gender-identity) is already in event_log and legible via
    -- its twin; it simply has no projection policy here. Required for set-union
    -- federation (ADR-0012) — never reject (that is the floor's job and it doesn't),
    -- never project a field we have no winner-policy for.
    IF fld NOT IN ('dob', 'sex-at-birth') THEN
        RETURN NULL;
    END IF;

    INSERT INTO patient_demographic AS pd
        (patient_id, field, value, facets, provenance, provenance_rank,
         asserted_hlc_wall, asserted_hlc_count, asserted_origin)
    VALUES
        (NEW.patient_id, fld, p ->> 'value', p -> 'facets', p ->> 'provenance', v_rank,
         NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
    -- Winner = max (provenance_rank, then HLC recency, then node_origin). Provenance
    -- beats recency (rank leads the tuple), so a later lower-provenance assertion
    -- cannot displace an earlier higher-provenance one ("verified value locks"); a
    -- later EQUAL-provenance assertion wins on HLC. node_origin is the final
    -- deterministic tiebreak, so every node converges to the same winner regardless
    -- of apply order. The WHERE gates the overlay: if the incoming row does not
    -- outrank the incumbent, the row is left unchanged.
    ON CONFLICT (patient_id, field) DO UPDATE SET
        value              = EXCLUDED.value,
        facets             = EXCLUDED.facets,
        provenance         = EXCLUDED.provenance,
        provenance_rank    = EXCLUDED.provenance_rank,
        asserted_hlc_wall  = EXCLUDED.asserted_hlc_wall,
        asserted_hlc_count = EXCLUDED.asserted_hlc_count,
        asserted_origin    = EXCLUDED.asserted_origin,
        updated_at         = clock_timestamp()
    WHERE (EXCLUDED.provenance_rank, EXCLUDED.asserted_hlc_wall,
           EXCLUDED.asserted_hlc_count, EXCLUDED.asserted_origin)
        > (pd.provenance_rank, pd.asserted_hlc_wall,
           pd.asserted_hlc_count, pd.asserted_origin);
    RETURN NULL;
END;
$$;

DROP TRIGGER IF EXISTS patient_demographic_apply_trg ON event_log;
CREATE TRIGGER patient_demographic_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'demographic.field.asserted')
    EXECUTE FUNCTION patient_demographic_apply();

GRANT SELECT ON patient_demographic TO cairn_agent;

-- Demographics' ONLY change to the write path: extend the twin hook (NOT submit_event)
-- to dispatch BOTH demographic event types through their structural floor, then a
-- single shared §4.5 authored-twin enforcement. This supersedes db/010's definition
-- (latest-loaded wins — the standard additive-migration pattern); the identifier
-- branch behaves identically. Legacy types fall back to the derived skeleton twin.
CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
    v_twin text;
BEGIN
    IF p_type = 'demographic.identifier.asserted' THEN
        PERFORM cairn_check_identifier_assertion(b);
    ELSIF p_type = 'demographic.field.asserted' THEN
        PERFORM cairn_check_demographic_field(b);
    ELSE
        RETURN cairn_twin_skeleton(p_type, b);
    END IF;
    -- Shared §4.5 authored-twin enforcement for every demographic assertion (written
    -- once, not duplicated per branch): the twin is materialised at authoring, so an
    -- empty/absent twin on a demographic event is refused.
    v_twin := b ->> 'plaintext_twin';
    IF v_twin IS NULL OR length(trim(v_twin)) = 0 THEN
        RAISE EXCEPTION 'submit_event: demographic assertion requires a non-empty authored twin (§4.5)';
    END IF;
    RETURN v_twin;
END;
$$;

COMMIT;
```

- [ ] **Step 4: Register the migration in `SCHEMA`**

In `crates/cairn-node/src/db.rs`, change the array length on line 3 from `9` to `10`, and add the `011` line after the `010_demographics` line (line 16):

```rust
const SCHEMA: [(&str, &str); 10] = [
```
```rust
    ("010_demographics",  include_str!("../../../db/010_demographics.sql")),
    ("011_demographics_fields", include_str!("../../../db/011_demographics_fields.sql")),
];
```

- [ ] **Step 5: Run the happy-path test to verify it passes**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test demographics_fields happy_path -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Lint + commit**

```bash
cargo clippy -p cairn-node --all-targets -- -D warnings
git add db/011_demographics_fields.sql crates/cairn-node/src/db.rs crates/cairn-node/tests/demographics_fields.rs
git commit -m "feat(db): §4.2 dob+sex-at-birth floor, provenance ladder, precedence projection"
```

---

### Task 3: Provenance-precedence + recency tests

**Files:**
- Test: `crates/cairn-node/tests/demographics_fields.rs` (append two tests)

**Interfaces:**
- Consumes: the `setup` / `submit_field` helpers and `dob_assertion_body` / `render_dob_twin` from Task 2.
- Produces: nothing new — these tests verify the Task-2 trigger's winner logic. (If they fail, the bug is in Task 2's `patient_demographic_apply` winner tuple — fix it there.)

- [ ] **Step 1: Write the failing precedence tests**

Append to `crates/cairn-node/tests/demographics_fields.rs`:

```rust
/// Helper: read the current projected dob value for a patient.
async fn dob_value(c: &Client, p: Uuid) -> String {
    let p_str = p.to_string();
    c.query_one(
        "SELECT value FROM patient_demographic WHERE patient_id::text=$1 AND field='dob'",
        &[&p_str]).await.unwrap().get(0)
}

#[tokio::test]
async fn provenance_beats_recency_and_verified_locks() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // 1) An early patient-stated dob (rank 50).
    submit_field(&c, &sk, &kid, p, 1,
        dob_assertion_body("1979", "year", None, "patient-stated"),
        Some(&render_dob_twin("1979", "year", "patient-stated"))).await.unwrap();
    assert_eq!(dob_value(&c, p).await, "1979");

    // 2) A later document-verified dob (rank 60) — higher provenance wins.
    submit_field(&c, &sk, &kid, p, 2,
        dob_assertion_body("1980-07-15", "day", Some("document"), "document-verified"),
        Some(&render_dob_twin("1980-07-15", "day", "document-verified"))).await.unwrap();
    assert_eq!(dob_value(&c, p).await, "1980-07-15", "higher provenance wins");

    // 3) An EVEN LATER patient-stated dob (rank 50) — must NOT displace the verified
    //    value. "Verified value locks vs. lower provenance."
    submit_field(&c, &sk, &kid, p, 3,
        dob_assertion_body("1981", "year", None, "patient-stated"),
        Some(&render_dob_twin("1981", "year", "patient-stated"))).await.unwrap();
    assert_eq!(dob_value(&c, p).await, "1980-07-15", "verified value locks vs lower provenance");
}

#[tokio::test]
async fn recency_breaks_ties_among_equal_provenance() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // Two document-verified dobs (equal rank) — the HLC-later one wins.
    submit_field(&c, &sk, &kid, p, 1,
        dob_assertion_body("1980-01-01", "day", Some("document"), "document-verified"),
        Some(&render_dob_twin("1980-01-01", "day", "document-verified"))).await.unwrap();
    submit_field(&c, &sk, &kid, p, 2,
        dob_assertion_body("1980-07-15", "day", Some("document"), "document-verified"),
        Some(&render_dob_twin("1980-07-15", "day", "document-verified"))).await.unwrap();
    assert_eq!(dob_value(&c, p).await, "1980-07-15", "later HLC wins among equal provenance");
}
```

- [ ] **Step 2: Run to verify pass (or surface a Task-2 bug)**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test demographics_fields -- --nocapture`
Expected: PASS. If `provenance_beats_recency_and_verified_locks` or `recency_breaks_ties_among_equal_provenance` FAILS, the winner tuple in `patient_demographic_apply` (db/011) is wrong — fix it there and re-run.

- [ ] **Step 3: Commit**

```bash
git add crates/cairn-node/tests/demographics_fields.rs
git commit -m "test(db): §4.2 provenance-precedence + recency winner semantics"
```

---

### Task 4: Floor rejections, the unknown-field gate, and regression

**Files:**
- Test: `crates/cairn-node/tests/demographics_fields.rs` (append three tests + one helper)

**Interfaces:**
- Consumes: `setup` / `submit_field` (Task 2); `cairn_event::{generate_key, sign, EventBody, Hlc}` already imported.
- Produces: nothing new — adversarial coverage of the floor and the projection gate.

- [ ] **Step 1: Write the failing rejection / gate / regression tests**

Append to `crates/cairn-node/tests/demographics_fields.rs`:

```rust
/// Assert the floor REJECTS the payload (submit errors) AND nothing was written —
/// neither to event_log nor to the patient_demographic projection.
async fn assert_rejected_and_empty(
    c: &Client, sk: &SigningKey, kid: &str, p: Uuid,
    payload: serde_json::Value, twin: Option<&str>, label: &str,
) {
    let r = submit_field(c, sk, kid, p, 1, payload, twin).await;
    assert!(r.is_err(), "{label}: must be rejected by the floor");
    let p_str = p.to_string();
    let n: i64 = c.query_one(
        "SELECT count(*) FROM event_log WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0);
    assert_eq!(n, 0, "{label}: nothing appended to event_log");
    let m: i64 = c.query_one(
        "SELECT count(*) FROM patient_demographic WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0);
    assert_eq!(m, 0, "{label}: nothing projected");
}

#[tokio::test]
async fn floor_rejects_each_invariant_violation() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let good = Some("Date of birth (document-verified): 1980 (year)");

    // value empty
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"dob","value":"","provenance":"document-verified",
                           "facets":{"precision":"year"}}), good, "value-empty").await;
    // provenance missing
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"dob","value":"1980","facets":{"precision":"year"}}),
        good, "provenance-missing").await;
    // field missing
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"value":"1980","provenance":"document-verified",
                           "facets":{"precision":"year"}}), good, "field-missing").await;
    // dob missing precision — principle 4
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"dob","value":"1980","provenance":"document-verified"}),
        good, "dob-missing-precision").await;
    // empty authored twin — §4.5
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"sex-at-birth","value":"female","provenance":"clinician-observed"}),
        Some(""), "empty-twin").await;
}

#[tokio::test]
async fn unknown_field_is_carried_but_not_projected() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // A well-formed assertion for a field this node has no projection policy for.
    // The floor ACCEPTS it (generic checks pass), it lands in event_log and is legible
    // via its twin — but it is NOT projected. This is the federation-forward contract:
    // an older node must store a newer node's field, just not project it (ADR-0012).
    submit_field(&c, &sk, &kid, p, 1,
        serde_json::json!({"field":"eye-color","value":"brown","provenance":"clinician-observed"}),
        Some("Eye color (clinician-observed): brown")
    ).await.expect("well-formed unknown field accepted (carried, legible)");

    let p_str = p.to_string();
    let in_log: i64 = c.query_one(
        "SELECT count(*) FROM event_log WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0);
    assert_eq!(in_log, 1, "unknown field is stored in event_log (legible evidence)");
    let projected: i64 = c.query_one(
        "SELECT count(*) FROM patient_demographic WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0);
    assert_eq!(projected, 0, "unknown field has no projection policy — not projected");
}

#[tokio::test]
async fn regression_identifier_and_legacy_patient_created_still_work() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // Slice-1 identifier assertion still projects through the (now re-declared) twin hook.
    let id_body = serde_json::json!({
        "field":"identifier","value":"943 476 5919","system":"nhs-number",
        "provenance":"document-verified","normalized":"9434765919","profile":"nhs-number@b3-abc"});
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(), patient_id: p.to_string(),
        event_type: "demographic.identifier.asserted".into(),
        schema_version: "demographic.identifier/1".into(),
        hlc: Hlc { wall: 1, counter: 0, node_origin: "n".into() }, t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: id_body, attachments: vec![],
        plaintext_twin: Some("nhs-number, document-verified: 943 476 5919".into()),
    };
    let signed = sign(&body, &sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await
        .expect("slice-1 identifier still accepted via re-declared twin hook");
    let p_str = p.to_string();
    let id_rows: i64 = c.query_one(
        "SELECT count(*) FROM patient_identifier WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0);
    assert_eq!(id_rows, 1, "identifier still projects");

    // Legacy patient.created (no authored twin) still gets the derived skeleton twin.
    let p2 = Uuid::now_v7();
    let body2 = EventBody {
        event_id: Uuid::now_v7().to_string(), patient_id: p2.to_string(),
        event_type: "patient.created".into(), schema_version: "demo/1".into(),
        hlc: Hlc { wall: 1, counter: 0, node_origin: "n".into() }, t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"name":"A B","dob":"1980","sex":"x"}),
        attachments: vec![], plaintext_twin: None,
    };
    let signed2 = sign(&body2, &sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed2.signed_bytes]).await
        .expect("legacy event still accepted");
    let twin: String = c.query_one(
        "SELECT plaintext_twin FROM event_log WHERE event_id::text=$1", &[&body2.event_id])
        .await.unwrap().get(0);
    assert!(twin.starts_with("[patient.created]"), "legacy still derives the skeleton twin");
}
```

- [ ] **Step 2: Run the full test file to verify it passes**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test demographics_fields -- --nocapture`
Expected: PASS (all tests across Tasks 2–4).

- [ ] **Step 3: Commit**

```bash
git add crates/cairn-node/tests/demographics_fields.rs
git commit -m "test(db): §4.2 floor rejections, unknown-field gate, slice-1/legacy regression"
```

---

### Task 5: Extend the canonical §4.1 provenance ladder

**Files:**
- Modify: `docs/spec/demographics.md` (the §4.1 "Provenance ladder" line)

**Interfaces:** none (documentation).

- [ ] **Step 1: Update the ladder prose**

In `docs/spec/demographics.md`, find the §4.1 line:

```
**Provenance ladder:** document-verified > patient-stated > third-party-stated > clinician-observed > imported/unknown > inferred. Capturing provenance must cost the registrar one tap.
```

Replace it with (adds the `fact-proven` top tier + a one-clause gloss; keeps the rest verbatim):

```
**Provenance ladder:** fact-proven (laboratory/scientifically-established truth — a karyotype, a confirmed assay — that can override what an official document merely *attests*) > document-verified > patient-stated > third-party-stated > clinician-observed > imported/unknown > inferred. Capturing provenance must cost the registrar one tap. The ladder is value-open: an unrecognized provenance term ranks lowest, so it can never displace a known-provenance value ([principle 4](index.md#founding-principles-the-lens-for-every-decision)).
```

- [ ] **Step 2: Verify the mkdocs build is clean**

Run: `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build`
Expected: build completes with no error referencing `demographics.md`.

- [ ] **Step 3: Commit**

```bash
git add docs/spec/demographics.md
git commit -m "spec(demographics): §4.1 ladder gains fact-proven top tier"
```

---

## Self-Review

**Spec coverage:**
- Generic `demographic.field.asserted` event + `field` discriminator → Task 1 (builders) + Task 2 (registration).
- `cairn_provenance_rank` ladder incl. `fact-proven` → Task 2 (fn) + Task 5 (canonical §4.1).
- Floor (generic + per-field dob precision; open value; never validates) → Task 2 (`cairn_check_demographic_field`) + Task 4 (rejection tests).
- Open floor / gated projection (unknown field carried, not projected) → Task 2 (trigger gate) + Task 4 (`unknown_field_is_carried_but_not_projected`).
- Provenance-precedence + "verified locks" + recency-among-equals → Task 2 (winner tuple) + Task 3 (tests).
- Winner-only `patient_demographic`; history in event_log → Task 2 (table, no retained-set).
- Twin hook re-decl, single shared authored-twin enforcement, submit_event reused verbatim → Task 2.
- §4.5 authored twin carried + empty-twin rejected → Task 2 (hook) + Task 4 (empty-twin case).
- Slice-1 + legacy regression → Task 4.
- `EventBody.plaintext_twin` reused unchanged (no new field) → no task needed (slice-1 already shipped it).

**Placeholder scan:** none — every code/SQL/test step shows complete content.

**Type consistency:** builder signatures (`dob_assertion_body(value, precision, basis: Option<&str>, provenance)`, `sex_at_birth_assertion_body(value, provenance)`, `render_dob_twin(value, precision, provenance)`, `render_sex_at_birth_twin(value, provenance)`) are defined in Task 1 and used identically in Tasks 2–4. `submit_field(c, sk, kid, patient, wall, payload, twin)` defined in Task 2, used in Tasks 3–4. `provenance_rank` is `INT` in the table and read as `i32` in the happy-path test. Event type string `"demographic.field.asserted"` and schema_version `"demographic.field/1"` consistent throughout.

## Session wrap (after Task 5, per the project's working rules)

- Update `docs/HANDOVER.md` + `docs/ROADMAP.md` to reflect slice 2 complete (concise; prune to < 500 lines).
- Run the full suite once green: `cargo test --workspace` (with `CAIRN_TEST_PG` set) + `cargo clippy --workspace --all-targets -- -D warnings`.
- Commit, push the branch, open a PR to `main` with the design + plan links; bundle the HANDOVER/ROADMAP currency edits into the work PR.
