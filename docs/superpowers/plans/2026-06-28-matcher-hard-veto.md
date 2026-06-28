# §4.4/§5.2 In-DB Hard-Veto + Coherence-Check Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the safety-critical in-DB `cairn_match_veto(patient_a, patient_b)` function — the closed set of hard vetoes (same-system identifier mismatch, verified-DOB clash, verified-sex-at-birth clash) that forces a human decision, never an auto-link, never an auto-reject.

**Architecture:** One new additive SQL file (`db/016_match_veto.sql`) composing three pure helper functions over the existing `patient_identifier` (db/010) and `patient_demographic` (db/011) projections. No event-format change, no `submit_event` change, no new projection table. The file is registered in the `cairn-node` `SCHEMA` array; coverage is cairn-node integration tests on real PG18 + cairn_pgx, gated on `$CAIRN_TEST_PG` and serialized via `db::test_serial_guard`.

**Tech Stack:** PostgreSQL ≥ 18 (PL/pgSQL + SQL functions), `cairn_pgx` pgrx extension, Rust integration tests (`tokio_postgres`, `cairn-event`, `cairn-node::db`).

## Global Constraints

- **AGPL-3.0** for all code; every dependency AGPL-3.0-compatible (no new deps in this plan).
- **TDD** — failing test first, then the SQL that makes it pass. Load-bearing: this is safety-critical (§9) in-DB code.
- **Reviewer-legibility** — every function carries a comment explaining *why* it exists and *how* it fits, for a junior contributor (house rule 3). Pure, composable functions over cleverness (house rule 4).
- **PostgreSQL ≥ 18**; the integration boundary is the DB boundary.
- **Files under ~500 lines** where feasible; `db/016_match_veto.sql` is well under.
- **The veto never auto-acts.** It returns a verdict; it never writes, links, demotes, or rejects.
- **DB-gated test connection string:** `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + cairn_pgx). Tests `return` early (skip) when `$CAIRN_TEST_PG` is unset — match the existing demographics tests exactly.
- **Verdict vocabulary (verbatim):** `veto_kind ∈ {identifier, dob, sex-at-birth}`; `severity ∈ {hard_veto, degrade_hold}`. Verified provenance = `provenance_rank >= 60` (`document-verified` = 60, `fact-proven` = 70).

---

### Task 1: The SQL file — helpers + entry points (`db/016_match_veto.sql`)

This task writes the complete SQL. It is verified end-to-end by the integration tests in Tasks 3–5, but we commit the SQL first as one reviewable unit (it is one file with one responsibility), then register it (Task 2), then drive behaviour with tests. Per house-rule TDD the *behavioural* red-first cycle is Tasks 3–5; this task is the scaffolding those tests exercise. Do **not** skip running Task 2 before the tests.

**Files:**
- Create: `db/016_match_veto.sql`

**Interfaces:**
- Consumes: `patient_identifier(patient_id uuid, system text, value text, normalized text, …)` (db/010); `patient_demographic(patient_id uuid, field text, value text, facets jsonb, provenance_rank int, …)` (db/011); role `cairn_agent` (db/004).
- Produces:
  - `cairn_identifier_veto(p_a uuid, p_b uuid) RETURNS TABLE(veto_kind text, severity text, subject text, detail text)`
  - `cairn_field_clash(p_a uuid, p_b uuid, p_field text) RETURNS TABLE(veto_kind text, severity text, subject text, detail text)`
  - `cairn_match_veto(p_a uuid, p_b uuid) RETURNS TABLE(veto_kind text, severity text, subject text, detail text)`
  - `cairn_has_hard_veto(p_a uuid, p_b uuid) RETURNS boolean`

- [ ] **Step 1: Write the SQL file**

Create `db/016_match_veto.sql` with exactly this content:

```sql
-- db/016_match_veto.sql
-- §4.4/§5.2 in-DB hard-veto + coherence-check (the matching pipeline's safety floor).
--
-- WHAT: given two patient candidates, return the closed set of HARD VETOES between
-- them — strong evidence AGAINST a link. A veto FORCES A HUMAN DECISION: it never
-- auto-links and never auto-rejects (an auto-reject is itself a silent false split,
-- identity §5.2/§5.13). This function only COMPUTES a verdict; it never writes,
-- links, demotes, or queues anything.
--
-- WHY HERE (not in the Python matcher): the matcher is advisory and only *proposes*
-- (identity §5.2 NOTE). The hard-veto floor is safety-critical (§9) — it must be
-- deterministic, in-database, and parse nothing culture-specific. This is the floor
-- every future matcher proposal must pass.
--
-- Reads only the existing projections patient_identifier (db/010) and
-- patient_demographic (db/011). Additive: no event-format change, no submit_event
-- change, no new table. Reuses cairn_provenance_rank's output (the cached
-- patient_demographic.provenance_rank column).
--
-- Two verdict levels (the §4.4 honest-degradation nuance):
--   hard_veto    — a TRUSTWORTHY clash; blocks auto-link AND (once linking exists)
--                  may demote an existing link to under-review.
--   degrade_hold — an UNTRUSTWORTHY basis (a profile-less node can't tell a real
--                  identifier mismatch from formatting noise); blocks auto-link and
--                  surfaces to a human, but must NOT demote an existing link.
-- Both stay on the safe side of false-merge >> false-split: neither auto-rejects.

-- ---------------------------------------------------------------------------
-- Helper: the §4.4 identifier veto over patient_identifier.
--
-- A patient may legitimately hold MULTIPLE identifiers in one `system` (the
-- projection PK is (patient_id, system, match_key)), so the comparison is
-- SET-BASED per system, not value-to-value. A clash exists for a shared system
-- only when the two patients share NO common identifier — sharing even one value
-- is positive evidence (a match signal), never a veto.
--   * `system = 'unknown'` (the §4.4 sentinel) NEVER participates in a veto.
--   * Trustworthy comparison is possible only over the materialised `normalized`
--     form. If the two sides share a normalized value -> no finding. Else if both
--     sides carry at least one non-null normalized -> the trustworthy sets are
--     disjoint -> hard_veto. Else (>=1 side is profile-less, normalized absent) ->
--     fall back to the raw `value`: shared value -> no finding; disjoint -> the
--     difference may be pure formatting noise -> degrade_hold.
--   * `9434765919` vs `943 476 5919` share one `normalized` -> no finding.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_identifier_veto(p_a uuid, p_b uuid)
RETURNS TABLE(veto_kind text, severity text, subject text, detail text)
LANGUAGE sql STABLE AS $$
    WITH a AS (
        SELECT system, value, normalized FROM patient_identifier
        WHERE patient_id = p_a AND system <> 'unknown'
    ),
    b AS (
        SELECT system, value, normalized FROM patient_identifier
        WHERE patient_id = p_b AND system <> 'unknown'
    ),
    shared_system AS (
        SELECT system FROM a INTERSECT SELECT system FROM b
    ),
    per_sys AS (
        SELECT
            s.system,
            -- the two sides share at least one non-null normalized value
            EXISTS (
                SELECT 1 FROM a JOIN b ON a.system = b.system
                WHERE a.system = s.system
                  AND a.normalized IS NOT NULL
                  AND a.normalized = b.normalized
            ) AS shared_norm,
            -- both sides carry at least one non-null normalized for this system
            EXISTS (SELECT 1 FROM a WHERE a.system = s.system AND a.normalized IS NOT NULL)
            AND
            EXISTS (SELECT 1 FROM b WHERE b.system = s.system AND b.normalized IS NOT NULL)
                AS both_have_norm,
            -- the two sides share at least one raw value string
            EXISTS (
                SELECT 1 FROM a JOIN b ON a.system = b.system
                WHERE a.system = s.system AND a.value = b.value
            ) AS shared_val
        FROM shared_system s
    )
    SELECT
        'identifier'::text,
        CASE WHEN both_have_norm THEN 'hard_veto'::text ELSE 'degrade_hold'::text END,
        system,
        CASE WHEN both_have_norm
             THEN format('same system %L, no shared normalized identifier (trustworthy mismatch)', system)
             ELSE format('same system %L, values differ but a profile is absent — held for human review', system)
        END
    FROM per_sys
    WHERE NOT shared_norm
      AND NOT shared_val;
$$;

-- ---------------------------------------------------------------------------
-- Helper: the verified DOB / sex-at-birth coherence clash over
-- patient_demographic (one winner row per (patient_id, field)).
--
-- Fires hard_veto IFF: both patients have a winner for `p_field`, BOTH winners
-- are VERIFIED (provenance_rank >= 60: document-verified | fact-proven — the
-- "verified value locks" property of the db/011 projection means a node's winner
-- already reflects its verified value when one exists), the winners carry the SAME
-- precision facet, and the `value` strings differ.
--
-- PARSES NO DATES. The floor never parses the open `value` string (db/011) — date
-- parsing is locale-specific, profile-dependent logic that belongs in the advisory
-- Python matcher, not the safety floor.
--   * Different precision -> NO finding. `1980` (year) vs `1980-03-15` (day) are a
--     consistent coarsening; principle 4: imprecision is partial agreement, never
--     disagreement. (IS NOT DISTINCT FROM treats both-null precision as equal, so
--     sex-at-birth — which carries no precision facet — reduces to "both verified +
--     values differ".)
--   * Known conservative residual: same precision, different format/coding
--     (`15/03/1980` vs `1980-03-15`, or `M` vs `male`) -> a false hard_veto. Safe
--     side (routes to human review, never auto-rejects/merges); rare within one
--     node's own data; resolved by the advisory matcher's locale comparators.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_field_clash(p_a uuid, p_b uuid, p_field text)
RETURNS TABLE(veto_kind text, severity text, subject text, detail text)
LANGUAGE sql STABLE AS $$
    SELECT
        p_field,
        'hard_veto'::text,
        p_field,
        format('verified %s clash (precision %s): %L vs %L',
               p_field,
               coalesce(x.facets ->> 'precision', 'none'),
               x.value, y.value)
    FROM patient_demographic x
    JOIN patient_demographic y ON y.field = x.field
    WHERE x.patient_id = p_a
      AND y.patient_id = p_b
      AND x.field = p_field
      AND x.provenance_rank >= 60
      AND y.provenance_rank >= 60
      AND x.value IS DISTINCT FROM y.value
      AND (x.facets ->> 'precision') IS NOT DISTINCT FROM (y.facets ->> 'precision');
$$;

-- ---------------------------------------------------------------------------
-- The public entry point: the union of the closed hard-veto set between two
-- patient candidates. Empty set = no veto (clear to auto-link, subject to the
-- matcher's own conservative threshold — not this function's concern). Symmetric,
-- deterministic; a = b yields empty naturally (identical identifier sets share a
-- normalized; identical demographic winners are value-equal).
--
-- DECEASED-STATUS CONFLICT (§5.13 closed set) IS DEFERRED — no deceased field is
-- projected yet (patient_demographic projects only dob + sex-at-birth). When a
-- deceased projection lands, add a fourth branch:
--     UNION ALL SELECT * FROM cairn_field_clash(p_a, p_b, 'deceased')
-- (or a bespoke helper if deceased needs different clash semantics). See the
-- design doc §6 and HANDOVER. Do NOT silently drop it.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_match_veto(p_a uuid, p_b uuid)
RETURNS TABLE(veto_kind text, severity text, subject text, detail text)
LANGUAGE sql STABLE AS $$
    SELECT * FROM cairn_identifier_veto(p_a, p_b)
    UNION ALL
    SELECT * FROM cairn_field_clash(p_a, p_b, 'dob')
    UNION ALL
    SELECT * FROM cairn_field_clash(p_a, p_b, 'sex-at-birth');
$$;

-- ---------------------------------------------------------------------------
-- Scalar convenience: the matcher's auto-link gate. True iff any HARD_VETO-severity
-- finding exists. A lone degrade_hold does NOT trip this gate (the caller still
-- surfaces such a pair to a human, but it is not a trustworthy veto).
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_has_hard_veto(p_a uuid, p_b uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM cairn_match_veto(p_a, p_b) WHERE severity = 'hard_veto'
    );
$$;

GRANT EXECUTE ON FUNCTION cairn_identifier_veto(uuid, uuid) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_field_clash(uuid, uuid, text) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_match_veto(uuid, uuid) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_has_hard_veto(uuid, uuid) TO cairn_agent;
```

- [ ] **Step 2: Commit the SQL file**

```bash
git add db/016_match_veto.sql
git commit -m "feat(matcher): add the §4.4/§5.2 in-DB hard-veto SQL (db/016)"
```

---

### Task 2: Register `db/016` in the cairn-node SCHEMA array

**Files:**
- Modify: `crates/cairn-node/src/db.rs:3-22`

**Interfaces:**
- Consumes: `db/016_match_veto.sql` (Task 1).
- Produces: the loaded-schema path now includes `cairn_match_veto` / `cairn_has_hard_veto` for every test DB built by `db::connect_and_load_schema`.

- [ ] **Step 1: Bump the array length and append the entry**

In `crates/cairn-node/src/db.rs`, change the array length from `14` to `15`:

```rust
const SCHEMA: [(&str, &str); 15] = [
```

Then add the new entry immediately after the `015_globalise_twin` line, before the closing `];`:

```rust
    ("015_globalise_twin", include_str!("../../../db/015_globalise_twin.sql")),
    ("016_match_veto",    include_str!("../../../db/016_match_veto.sql")),
];
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p cairn-node`
Expected: builds clean (the `include_str!` resolves; array length matches the 15 entries: 001–007, 009–016).

- [ ] **Step 3: Commit**

```bash
git add crates/cairn-node/src/db.rs
git commit -m "feat(matcher): load db/016 in the cairn-node SCHEMA array (14->15)"
```

---

### Task 3: Integration test harness + the identifier vetoes

**Files:**
- Create: `crates/cairn-node/tests/match_veto.rs`

**Interfaces:**
- Consumes: `cairn_event::demographics::{IdentifierAssertion, identifier_assertion_body, render_identifier_twin}`; `cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey}`; `cairn_node::db`; the SQL `cairn_match_veto` / `cairn_has_hard_veto`.
- Produces: the test module + the `setup` / `submit_identifier` helpers reused by Tasks 4–5.

- [ ] **Step 1: Write the harness + the first failing tests (identifier vetoes)**

Create `crates/cairn-node/tests/match_veto.rs`:

```rust
//! Integration coverage for the §4.4/§5.2 in-DB hard-veto + coherence-check
//! (db/016): cairn_match_veto / cairn_has_hard_veto over the patient_identifier
//! and patient_demographic projections. Real Postgres, gated on `$CAIRN_TEST_PG`,
//! serialized cluster-wide via `db::test_serial_guard`. The advisory probabilistic
//! matcher (§5.2 piece B, Python) and the §5.7 link-apply seam are separate
//! subsystems and are NOT exercised here.
use cairn_event::demographics::{
    dob_assertion_body, identifier_assertion_body, render_dob_twin, render_identifier_twin,
    render_sex_at_birth_twin, sex_at_birth_assertion_body, IdentifierAssertion,
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

/// Sign + submit one demographic event of any field through the real submit_event door.
async fn submit(
    c: &Client, sk: &SigningKey, kid: &str, patient: Uuid, wall: i64,
    event_type: &str, schema_version: &str,
    payload: serde_json::Value, twin: Option<&str>,
) -> Result<u64, tokio_postgres::Error> {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: event_type.into(),
        schema_version: schema_version.into(),
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

/// Submit one §4.4 identifier assertion for `patient`.
async fn submit_identifier(
    c: &Client, sk: &SigningKey, kid: &str, patient: Uuid, wall: i64, a: &IdentifierAssertion<'_>,
) {
    submit(c, sk, kid, patient, wall, "demographic.identifier.asserted",
           "demographic.identifier/1",
           identifier_assertion_body(a), Some(&render_identifier_twin(a)))
        .await.expect("valid identifier accepted");
}

/// Collect cairn_match_veto rows as (veto_kind, severity, subject) tuples, ordered.
async fn veto_rows(c: &Client, a: Uuid, b: Uuid) -> Vec<(String, String, String)> {
    let a_s = a.to_string();
    let b_s = b.to_string();
    let rows = c.query(
        "SELECT veto_kind, severity, subject FROM cairn_match_veto($1::uuid, $2::uuid) \
         ORDER BY veto_kind, subject",
        &[&a_s, &b_s]).await.unwrap();
    rows.iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect()
}

async fn has_hard_veto(c: &Client, a: Uuid, b: Uuid) -> bool {
    let a_s = a.to_string();
    let b_s = b.to_string();
    c.query_one("SELECT cairn_has_hard_veto($1::uuid, $2::uuid)", &[&a_s, &b_s])
        .await.unwrap().get(0)
}

/// Build an IdentifierAssertion borrowing the given strings (helper for readability).
fn idassert<'a>(system: &'a str, value: &'a str, normalized: Option<&'a str>) -> IdentifierAssertion<'a> {
    IdentifierAssertion {
        value, system, provenance: "patient-stated",
        normalized, profile: None, use_: None,
    }
}

#[tokio::test]
async fn no_veto_when_no_shared_system() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_identifier(&c, &sk, &kid, a, 1, &idassert("medicare-au", "1111", Some("1111"))).await;
    submit_identifier(&c, &sk, &kid, b, 2, &idassert("nhs-number", "2222", Some("2222"))).await;
    assert!(veto_rows(&c, a, b).await.is_empty(), "disjoint systems raise no veto");
    assert!(!has_hard_veto(&c, a, b).await);
}

#[tokio::test]
async fn identifier_hard_veto_when_normalized_present_and_disjoint() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_identifier(&c, &sk, &kid, a, 1, &idassert("medicare-au", "9434765919", Some("9434765919"))).await;
    submit_identifier(&c, &sk, &kid, b, 2, &idassert("medicare-au", "5000000000", Some("5000000000"))).await;
    let rows = veto_rows(&c, a, b).await;
    assert_eq!(rows, vec![("identifier".into(), "hard_veto".into(), "medicare-au".into())]);
    assert!(has_hard_veto(&c, a, b).await);
}

#[tokio::test]
async fn identifier_same_normalized_is_no_veto() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    // Same identifier, formatted differently, identical normalized -> match signal, NOT a veto.
    submit_identifier(&c, &sk, &kid, a, 1, &idassert("medicare-au", "9434765919", Some("9434765919"))).await;
    submit_identifier(&c, &sk, &kid, b, 2, &idassert("medicare-au", "943 476 5919", Some("9434765919"))).await;
    assert!(veto_rows(&c, a, b).await.is_empty(), "shared normalized = positive evidence");
}

#[tokio::test]
async fn identifier_degrade_hold_when_profile_absent_and_values_differ() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    // No normalized on either side (profile-less node); raw values differ -> cannot trust
    // (may be formatting noise) -> degrade_hold, never a hard veto.
    submit_identifier(&c, &sk, &kid, a, 1, &idassert("local-mrn", "00123", None)).await;
    submit_identifier(&c, &sk, &kid, b, 2, &idassert("local-mrn", "123", None)).await;
    let rows = veto_rows(&c, a, b).await;
    assert_eq!(rows, vec![("identifier".into(), "degrade_hold".into(), "local-mrn".into())]);
    assert!(!has_hard_veto(&c, a, b).await, "degrade_hold does not trip the auto-link gate");
}

#[tokio::test]
async fn unknown_system_never_vetoes() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_identifier(&c, &sk, &kid, a, 1, &idassert("unknown", "AAA", Some("AAA"))).await;
    submit_identifier(&c, &sk, &kid, b, 2, &idassert("unknown", "BBB", Some("BBB"))).await;
    assert!(veto_rows(&c, a, b).await.is_empty(), "system 'unknown' never participates in a veto");
}

#[tokio::test]
async fn multi_valued_shared_value_is_no_veto() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    // A holds {X, Y}; B holds {Y} in one system -> they share Y -> no veto (set-based).
    submit_identifier(&c, &sk, &kid, a, 1, &idassert("medicare-au", "1000000000", Some("1000000000"))).await;
    submit_identifier(&c, &sk, &kid, a, 2, &idassert("medicare-au", "2000000000", Some("2000000000"))).await;
    submit_identifier(&c, &sk, &kid, b, 3, &idassert("medicare-au", "2000000000", Some("2000000000"))).await;
    assert!(veto_rows(&c, a, b).await.is_empty(), "one shared normalized in the set = no veto");
}
```

- [ ] **Step 2: Run the tests**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test match_veto`
Expected: with Tasks 1–2 already committed, all 6 tests PASS, with no unused-item warnings. If `cairn_match_veto` is missing you would see `function cairn_match_veto(uuid, uuid) does not exist` — that confirms Task 2 wiring is required. (If `$CAIRN_TEST_PG` is unset every test prints "skipped" and passes vacuously — that is NOT a real run; set the env var.)

- [ ] **Step 3: Commit**

```bash
git add crates/cairn-node/tests/match_veto.rs
git commit -m "test(matcher): identifier hard-veto + degrade-hold + set-based no-veto cases"
```

---

### Task 4: The verified DOB / sex-at-birth coherence clashes

**Files:**
- Modify: `crates/cairn-node/tests/match_veto.rs` (append tests + a `submit_dob`/`submit_sex` helper)

**Interfaces:**
- Consumes: the `submit` helper + `dob_assertion_body` / `sex_at_birth_assertion_body` / `render_dob_twin` / `render_sex_at_birth_twin` (already imported in Task 3).
- Produces: DOB/sex clash coverage.

- [ ] **Step 1: Append the field-clash helpers + failing tests**

Add to `crates/cairn-node/tests/match_veto.rs`:

```rust
/// Submit one §4.2 DOB assertion.
async fn submit_dob(
    c: &Client, sk: &SigningKey, kid: &str, patient: Uuid, wall: i64,
    value: &str, precision: &str, provenance: &str,
) {
    submit(c, sk, kid, patient, wall, "demographic.field.asserted", "demographic.field/1",
           dob_assertion_body(value, precision, Some("document"), provenance),
           Some(&render_dob_twin(value, precision, provenance)))
        .await.expect("valid dob accepted");
}

/// Submit one §4.2 sex-at-birth assertion.
async fn submit_sex(
    c: &Client, sk: &SigningKey, kid: &str, patient: Uuid, wall: i64,
    value: &str, provenance: &str,
) {
    submit(c, sk, kid, patient, wall, "demographic.field.asserted", "demographic.field/1",
           sex_at_birth_assertion_body(value, provenance),
           Some(&render_sex_at_birth_twin(value, provenance)))
        .await.expect("valid sex-at-birth accepted");
}

#[tokio::test]
async fn dob_hard_veto_when_both_verified_same_precision_differ() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_dob(&c, &sk, &kid, a, 1, "1980-03-15", "day", "document-verified").await;
    submit_dob(&c, &sk, &kid, b, 2, "1980-03-16", "day", "document-verified").await;
    let rows = veto_rows(&c, a, b).await;
    assert_eq!(rows, vec![("dob".into(), "hard_veto".into(), "dob".into())]);
    assert!(has_hard_veto(&c, a, b).await);
}

#[tokio::test]
async fn dob_no_veto_when_precision_differs() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    // `1980` (year) vs `1980-03-15` (day): a consistent coarsening, not a clash (principle 4).
    submit_dob(&c, &sk, &kid, a, 1, "1980", "year", "document-verified").await;
    submit_dob(&c, &sk, &kid, b, 2, "1980-03-15", "day", "document-verified").await;
    assert!(veto_rows(&c, a, b).await.is_empty(), "different precision = no finding");
}

#[tokio::test]
async fn dob_no_veto_when_not_both_verified() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    // One verified, one patient-stated (rank < 60) -> not a hard veto.
    submit_dob(&c, &sk, &kid, a, 1, "1980-03-15", "day", "document-verified").await;
    submit_dob(&c, &sk, &kid, b, 2, "1980-03-16", "day", "patient-stated").await;
    assert!(veto_rows(&c, a, b).await.is_empty(), "clash only on verified-vs-verified");
}

#[tokio::test]
async fn sex_at_birth_hard_veto_when_both_verified_differ() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_sex(&c, &sk, &kid, a, 1, "female", "document-verified").await;
    submit_sex(&c, &sk, &kid, b, 2, "male", "document-verified").await;
    let rows = veto_rows(&c, a, b).await;
    assert_eq!(rows, vec![("sex-at-birth".into(), "hard_veto".into(), "sex-at-birth".into())]);
    assert!(has_hard_veto(&c, a, b).await);
}
```

- [ ] **Step 2: Run the new tests**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test match_veto`
Expected: all tests (Task 3 + Task 4) PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/cairn-node/tests/match_veto.rs
git commit -m "test(matcher): verified DOB/sex-at-birth coherence clash (precision-gated)"
```

---

### Task 5: Combined findings + symmetry

**Files:**
- Modify: `crates/cairn-node/tests/match_veto.rs` (append)

**Interfaces:**
- Consumes: all helpers from Tasks 3–4.
- Produces: multi-finding + symmetry coverage (the function's set-invariants).

- [ ] **Step 1: Append the combined + symmetry tests**

Add to `crates/cairn-node/tests/match_veto.rs`:

```rust
#[tokio::test]
async fn multiple_findings_identifier_and_dob() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_identifier(&c, &sk, &kid, a, 1, &idassert("medicare-au", "1000000000", Some("1000000000"))).await;
    submit_identifier(&c, &sk, &kid, b, 2, &idassert("medicare-au", "2000000000", Some("2000000000"))).await;
    submit_dob(&c, &sk, &kid, a, 3, "1980-03-15", "day", "document-verified").await;
    submit_dob(&c, &sk, &kid, b, 4, "1980-03-16", "day", "document-verified").await;
    let rows = veto_rows(&c, a, b).await;
    // ORDER BY veto_kind, subject -> dob row before identifier row.
    assert_eq!(rows, vec![
        ("dob".into(), "hard_veto".into(), "dob".into()),
        ("identifier".into(), "hard_veto".into(), "medicare-au".into()),
    ]);
    assert!(has_hard_veto(&c, a, b).await);
}

#[tokio::test]
async fn veto_is_symmetric() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_identifier(&c, &sk, &kid, a, 1, &idassert("medicare-au", "1000000000", Some("1000000000"))).await;
    submit_identifier(&c, &sk, &kid, b, 2, &idassert("medicare-au", "2000000000", Some("2000000000"))).await;
    assert_eq!(veto_rows(&c, a, b).await, veto_rows(&c, b, a).await,
               "cairn_match_veto(a,b) must equal cairn_match_veto(b,a)");
}
```

- [ ] **Step 2: Run the full module**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test match_veto`
Expected: all 13 tests PASS (6 from Task 3, 4 from Task 4, 1 multi-finding + 1 symmetry here = 12 named + the multi-valued one = 13 total — confirm count).

- [ ] **Step 3: Commit**

```bash
git add crates/cairn-node/tests/match_veto.rs
git commit -m "test(matcher): combined identifier+DOB findings and symmetry"
```

---

### Task 6: Full-suite regression + clippy + docs

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

**Interfaces:**
- Consumes: the completed feature.
- Produces: a green workspace + updated disposable docs.

- [ ] **Step 1: Run the full cairn-node + workspace suite**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace`
Expected: the new `match_veto` tests plus all demographics slices (010–015) and node tests PASS (no regression from the additive db/016).

- [ ] **Step 2: Clippy clean**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 3: Update HANDOVER.md + ROADMAP.md**

In `docs/HANDOVER.md`: add a new "This session" block summarising the §4.4/§5.2 hard-veto slice (db/016; `cairn_match_veto`/`cairn_has_hard_veto`; the two verdict levels; precision-gated DOB; deceased-veto + Python matcher + §5.7 link-apply deferred); demote the prior session block. In the "Open threads" menu, update the demographics "Next" line: the in-DB hard veto is now built; next is the advisory Python probabilistic matcher (piece B) and/or the §5.7 link-apply seam (piece C). Note the deferred deceased-status veto branch explicitly.

In `docs/ROADMAP.md` Phase 4: append to the demographics paragraph that the §4.4/§5.2 **in-DB hard-veto floor** is built (db/016, SCHEMA 14→15): `cairn_match_veto` returns the closed hard-veto set (same-system identifier mismatch · verified-DOB clash · verified-sex-at-birth clash), two verdict levels (hard_veto/degrade_hold), precision-gated/no-date-parsing; deceased veto deferred (no projection); advisory Python matcher + §5.7 link-apply seam are the remaining matcher pieces.

Keep both files concise (prune older detail as needed to stay under 500 lines).

- [ ] **Step 4: Commit**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs(matcher): record the §4.4/§5.2 in-DB hard-veto slice (db/016)"
```

---

## Self-Review

**Spec coverage:**
- §1 purpose / piece-A scope → Tasks 1–5 (the function) + Task 6 (docs).
- §2 interface (`cairn_match_veto` table fn + `cairn_has_hard_veto` scalar; verdict vocabulary; symmetry; a=b empty) → Task 1 SQL; symmetry test Task 5; a=b is naturally empty (covered by the same-normalized/value-equal logic, exercised implicitly — note: no dedicated a=b test; the symmetry + no-veto tests cover the mechanism).
- §3 two verdict levels → identifier hard_veto (Task 3) + degrade_hold (Task 3) + the scalar-gate assertion that degrade_hold does not trip it (Task 3).
- §4 set-based identifier comparison, `unknown` excluded → Tasks 3 (disjoint, same-normalized, degrade, unknown, multi-valued-shared).
- §5 precision-gated DOB, no date parsing → Task 4 (same-precision hard_veto, different-precision no-finding, not-both-verified no-finding) + sex-at-birth.
- §6 deceased deferral → Task 1 comment + Task 6 HANDOVER note.
- §7 placement/grants/schema → Task 1 (grants) + Task 2 (SCHEMA array).
- §8 test list (1–12 + 4b) → Tasks 3–5 (13 tests total).
- §9 out of scope (Python matcher, §5.7 seam, worklist, deceased) → not built; recorded in docs (Task 6).

**Placeholder scan:** No TBD/TODO/"handle edge cases" or dead-code placeholders remain (the earlier `const ID` paste-artifact was removed).

**Type consistency:** `veto_rows` returns `Vec<(String, String, String)>` of `(veto_kind, severity, subject)` consistently across Tasks 3–5; `idassert` / `submit` / `submit_identifier` / `submit_dob` / `submit_sex` signatures are defined once (Task 3/4) and reused; SQL function names (`cairn_match_veto`, `cairn_has_hard_veto`, `cairn_identifier_veto`, `cairn_field_clash`) match between Task 1 (definition), Task 2 (load), and Tasks 3–5 (use).
