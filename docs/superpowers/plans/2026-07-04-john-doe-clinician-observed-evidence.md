# §5.4 Clinician-Observed Evidence (estimated-age range + observed sex) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a clinician record honest clinician-observed evidence (estimated age → an explicit birth-year *range* DOB, and observed sex) on a John-Doe chart, and make the estimated-age range a positive scoring signal in the advisory matcher.

**Architecture:** Three layers on the *existing* demographic spine — (1) pure `cairn-event` builders that turn an estimated age into a time-invariant birth-year-range `dob` assertion; (2) a `cairn-node` orchestrator + CLI that author the `demographic.field.asserted` events on an existing patient UUID through the reused `submit_event` door; (3) range-aware, **positive-only** `compare_dob` in the Python matcher so an estimate can *support* but never *suppress* a match. No `db/` floor change, no schema/SCHEMA bump, no new event type, no ADR/spec edit.

**Tech Stack:** Rust (`cairn-event`, `cairn-node`, tokio-postgres), PostgreSQL ≥ 18 + `cairn_pgx` 0.2.0, Python (`cairn-matcher`, uv, pytest).

## Global Constraints

- **AGPL-3.0**; **no new dependency** in any crate or the matcher (pure stdlib / existing deps only).
- **TDD**: failing test first, then minimal code. **All suites green before every commit** (house rule #6).
- **Files < 500 lines** (house rule #4): `cairn-event/src/demographics.rs` is already 422 lines → the new pure builders go in a **new** `cairn-event/src/evidence.rs`; the node orchestrator goes in a **new** `cairn-node/src/evidence.rs`.
- **Inline docs for a junior contributor** (house rule #3) on every non-trivial function.
- **No floor/schema/ADR/spec change.** The `dob` field already requires `facets.precision` and accepts optional `facets.basis` (db/011); `clinician-observed` is already provenance rank 30.
- **Provenance term** for all observed evidence is exactly `"clinician-observed"` (matches `cairn_provenance_rank`, rank 30).
- **Range value encoding** is exactly `"<min>/<max>"` (e.g. `"1981/1991"`); **precision** is exactly `"year-range"`.
- **Observed sex** is stored on field **`administrative-sex`** (apparent/phenotypic), never `sex-at-birth`.
- Rust DB-gated tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"`; they self-serialize via `db::test_serial_guard`. Matcher DB tests: `cd matcher && CAIRN_TEST_PG=… uv run --extra pipeline pytest`. Pure matcher: `cd matcher && uv run pytest` (uv, never venv/pip).
- Demographic events carry a **HARD-required non-empty `plaintext_twin`** — reuse `render_dob_twin` / `render_administrative_sex_twin` (no new twin code).

---

### Task 1: Pure `cairn-event` evidence builders

**Files:**
- Create: `crates/cairn-event/src/evidence.rs`
- Modify: `crates/cairn-event/src/lib.rs` (add `pub mod evidence;`)
- Test: inline `#[cfg(test)] mod tests` in `evidence.rs`

**Interfaces:**
- Consumes (from `crate::demographics`): `dob_assertion_body(value, precision, basis: Option<&str>, provenance) -> serde_json::Value`, `demographic_field_body(field, value, facets: Option<Value>, provenance) -> Value`, `render_dob_twin(value, precision, provenance) -> String`, `render_administrative_sex_twin(value, provenance) -> String`.
- Produces:
  - `pub const YEAR_RANGE_PRECISION: &str = "year-range";`
  - `pub const CLINICIAN_OBSERVED_PROVENANCE: &str = "clinician-observed";`
  - `pub fn birth_year_range_from_age(age_years: u32, tolerance_years: u32, observed_year: i32) -> (i32, i32)`
  - `pub fn format_year_range(min_year: i32, max_year: i32) -> String`
  - `pub fn estimated_dob_body(min_year: i32, max_year: i32, basis: &str, provenance: &str) -> serde_json::Value`
  - `pub fn observed_sex_body(value: &str, basis: Option<&str>, provenance: &str) -> serde_json::Value`

- [ ] **Step 1: Write the failing tests**

Create `crates/cairn-event/src/evidence.rs` with ONLY the test module first (the `use super::*;` will not resolve until Step 3 — that is the intended red):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn birth_year_range_is_observed_year_minus_age_widened_by_tolerance() {
        // "apparent age ~40 ± 5", observed in 2026 -> born 1981..=1991.
        assert_eq!(birth_year_range_from_age(40, 5, 2026), (1981, 1991));
    }

    #[test]
    fn zero_tolerance_is_a_single_year_still_expressed_as_a_range() {
        assert_eq!(birth_year_range_from_age(30, 0, 2020), (1990, 1990));
    }

    #[test]
    fn format_year_range_uses_the_slash_separator() {
        assert_eq!(format_year_range(1981, 1991), "1981/1991");
    }

    #[test]
    fn estimated_dob_body_is_a_year_range_dob_with_basis_and_provenance() {
        let v = estimated_dob_body(1981, 1991, "apparent age ~40±5: dentition, greying",
                                   CLINICIAN_OBSERVED_PROVENANCE);
        assert_eq!(v["field"], "dob");
        assert_eq!(v["value"], "1981/1991");
        assert_eq!(v["facets"]["precision"], "year-range");
        assert_eq!(v["facets"]["basis"], "apparent age ~40±5: dentition, greying");
        assert_eq!(v["provenance"], "clinician-observed");
    }

    #[test]
    fn estimated_dob_twin_is_legible_without_a_profile() {
        // The reused render_dob_twin must produce a non-empty, human-readable twin.
        let twin = render_dob_twin("1981/1991", YEAR_RANGE_PRECISION, CLINICIAN_OBSERVED_PROVENANCE);
        assert!(twin.contains("1981/1991"));
        assert!(twin.contains("year-range"));
        assert!(!twin.trim().is_empty());
    }

    #[test]
    fn observed_sex_body_is_administrative_sex_with_optional_basis() {
        let with = observed_sex_body("male", Some("external genitalia"), CLINICIAN_OBSERVED_PROVENANCE);
        assert_eq!(with["field"], "administrative-sex");
        assert_eq!(with["value"], "male");
        assert_eq!(with["facets"]["basis"], "external genitalia");
        assert_eq!(with["provenance"], "clinician-observed");

        let without = observed_sex_body("female", None, CLINICIAN_OBSERVED_PROVENANCE);
        assert_eq!(without["field"], "administrative-sex");
        assert!(without.get("facets").is_none(), "absent basis must omit facets entirely, never null");
    }
}
```

Add `pub mod evidence;` to `crates/cairn-event/src/lib.rs` (place it alphabetically near `pub mod demographics;`).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cairn-event evidence`
Expected: FAIL to compile — `cannot find function birth_year_range_from_age` (and siblings).

- [ ] **Step 3: Write the minimal implementation**

Prepend to `crates/cairn-event/src/evidence.rs` (above the test module):

```rust
//! §5.4 clinician-observed identity evidence for unidentified ("John Doe") patients.
//!
//! A clinician who registers an unknown patient cannot know a DOB — they have an
//! *estimated age with basis* ("apparent age ~40, dentition/greying"). This module turns
//! that honest, imprecise observation into a demographic assertion the existing db/011
//! `dob` field already accepts, and an observed-sex assertion on `administrative-sex`.
//!
//! Two principle-4 rules shape the representation:
//!   1. Store the derived **birth-year window**, never the raw age — birth year is
//!      time-invariant; age drifts, so storing "40" would silently age the record.
//!   2. Store an **explicit range**, never a single midpoint — a midpoint year is a
//!      *precise untruth* the matcher would wrongly DISAGREE against a nearby true DOB.
//!
//! Pure functions only (no DB, no clock): the caller (the node layer) supplies the
//! observation year so every function here is deterministic and unit-testable.

use crate::demographics::{demographic_field_body, dob_assertion_body};
use serde_json::{json, Value};

/// The `facets.precision` term marking a dob value as an inclusive birth-year interval
/// (`"<min>/<max>"`). Distinct from the point precisions ("year"/"month"/"day"); the
/// matcher keys its range parsing on exactly this string.
pub const YEAR_RANGE_PRECISION: &str = "year-range";

/// The §4.1 provenance ladder term for evidence a clinician directly observed. Ranks 30
/// in db/011 — below patient-stated (50) and document-verified (60), so a real document
/// correctly displaces the estimate the moment identity is established.
pub const CLINICIAN_OBSERVED_PROVENANCE: &str = "clinician-observed";

/// Convert an estimated age (with a ± tolerance) observed in a given year into an
/// inclusive birth-year range. `birth_year = observed_year - age`; the tolerance widens
/// it symmetrically. Example: age 40 ± 5 observed in 2026 -> (1981, 1991). Returns
/// (min_year, max_year) with min_year <= max_year for any non-negative inputs.
pub fn birth_year_range_from_age(age_years: u32, tolerance_years: u32, observed_year: i32) -> (i32, i32) {
    let mid = observed_year - age_years as i32;
    let tol = tolerance_years as i32;
    (mid - tol, mid + tol)
}

/// Render an inclusive birth-year range as the canonical dob value string `"<min>/<max>"`
/// (ISO-8601-interval-style `/` separator). The `/` never collides with the ISO `-` date
/// splitter, so a range value safely fails point-date parsing on any older node.
pub fn format_year_range(min_year: i32, max_year: i32) -> String {
    format!("{min_year}/{max_year}")
}

/// Build a §4.2 estimated-age `dob` assertion payload: value = `"<min>/<max>"`, precision
/// = `year-range`, basis = the clinician's stated basis (required — §5.4 is "estimated age
/// WITH basis"; principle 4). Reuses `dob_assertion_body`, so the db/011 dob floor
/// (precision required, basis optional-non-empty) accepts it unchanged.
pub fn estimated_dob_body(min_year: i32, max_year: i32, basis: &str, provenance: &str) -> Value {
    let value = format_year_range(min_year, max_year);
    dob_assertion_body(&value, YEAR_RANGE_PRECISION, Some(basis), provenance)
}

/// Build a §4.2 observed-sex assertion payload on the `administrative-sex` field (the
/// apparent/phenotypic marker a clinician can honestly observe — NOT the `sex-at-birth`
/// fact, which they cannot know for a stranger). `basis` (how it was observed) is optional
/// and omitted entirely when None. Value is an OPEN string (principle 4).
pub fn observed_sex_body(value: &str, basis: Option<&str>, provenance: &str) -> Value {
    let facets = basis.map(|b| json!({ "basis": b }));
    demographic_field_body("administrative-sex", value, facets, provenance)
}
```

Add the import the twin test needs to the test module — change `use super::*;` line already covers it, but `render_dob_twin` is in `crate::demographics`; add at the top of the test module: `use crate::demographics::render_dob_twin;`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cairn-event evidence`
Expected: PASS (6 tests).

- [ ] **Step 5: Verify clippy + whole-crate tests**

Run: `cargo clippy -p cairn-event --all-targets -- -D warnings && cargo test -p cairn-event`
Expected: no warnings; all cairn-event tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-event/src/evidence.rs crates/cairn-event/src/lib.rs
git commit -m "feat(cairn-event): §5.4 clinician-observed evidence builders (estimated-age range dob + observed sex)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `cairn-node` orchestrator — author observed evidence on a chart

**Files:**
- Create: `crates/cairn-node/src/evidence.rs`
- Modify: `crates/cairn-node/src/lib.rs` (add `pub mod evidence;`)
- Modify: `crates/cairn-node/src/db.rs:175` (promote `pub(crate) async fn next_hlc` → `pub async fn next_hlc` — the integration test authors a comparison event with a correctly-ticked HLC; a shared floor-adjacent helper is legitimately public)
- Test: `crates/cairn-node/tests/observed_evidence.rs`

**Interfaces:**
- Consumes: `cairn_event::evidence::{birth_year_range_from_age, estimated_dob_body, observed_sex_body, CLINICIAN_OBSERVED_PROVENANCE, YEAR_RANGE_PRECISION}`; `cairn_event::demographics::{render_dob_twin, render_administrative_sex_twin}`; `cairn_event::{sign, EventBody, Hlc, SigningKey}`; `crate::db::next_hlc(client, node_origin) -> Result<Hlc>`.
- Produces:
  - `pub struct AgeObservation { pub age_years: u32, pub tolerance_years: u32, pub basis: String }`
  - `pub struct SexObservation { pub value: String, pub basis: Option<String> }`
  - `pub struct ObservedEvidence { pub age: Option<AgeObservation>, pub sex: Option<SexObservation> }`
  - `pub fn build_estimated_dob_event(event_id: Uuid, patient_id: Uuid, min_year: i32, max_year: i32, basis: &str, kid: &str, hlc: Hlc) -> EventBody`
  - `pub fn build_observed_sex_event(event_id: Uuid, patient_id: Uuid, value: &str, basis: Option<&str>, kid: &str, hlc: Hlc) -> EventBody`
  - `pub async fn assert_observed_evidence(client: &mut Client, sk: &SigningKey, kid: &str, node_origin: &str, patient_id: Uuid, ev: &ObservedEvidence, observed_year: i32) -> anyhow::Result<()>`

- [ ] **Step 1: Write the pure builder unit tests (inline in `evidence.rs`)**

Create `crates/cairn-node/src/evidence.rs` with the test module (red — `super::*` unresolved until Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn pid() -> Uuid { Uuid::parse_str("00000000-0000-0000-0000-0000000000ab").unwrap() }
    fn eid() -> Uuid { Uuid::parse_str("22222222-0000-0000-0000-000000000000").unwrap() }
    fn hlc() -> Hlc { Hlc { wall: 5, counter: 0, node_origin: "n".into() } }

    #[test]
    fn estimated_dob_event_is_a_clinician_observed_year_range_dob_with_twin() {
        let body = build_estimated_dob_event(eid(), pid(), 1981, 1991,
            "apparent age ~40±5: dentition", "kid", hlc());
        assert_eq!(body.event_type, "demographic.field.asserted");
        assert_eq!(body.patient_id, pid().to_string());
        assert_eq!(body.payload["field"], "dob");
        assert_eq!(body.payload["value"], "1981/1991");
        assert_eq!(body.payload["facets"]["precision"], "year-range");
        assert_eq!(body.payload["provenance"], "clinician-observed");
        assert_eq!(body.contributors[0]["role"], "recorded");
        assert!(body.contributors[0].get("responsibility").is_none(), "additive: no attestation");
        assert!(!body.plaintext_twin.as_deref().unwrap().trim().is_empty());
    }

    #[test]
    fn observed_sex_event_is_clinician_observed_administrative_sex() {
        let body = build_observed_sex_event(eid(), pid(), "male", Some("external genitalia"), "kid", hlc());
        assert_eq!(body.payload["field"], "administrative-sex");
        assert_eq!(body.payload["value"], "male");
        assert_eq!(body.payload["facets"]["basis"], "external genitalia");
        assert_eq!(body.payload["provenance"], "clinician-observed");
        assert!(!body.plaintext_twin.as_deref().unwrap().trim().is_empty());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p cairn-node --lib evidence`
Expected: FAIL to compile (`build_estimated_dob_event` not found).

- [ ] **Step 3: Write the implementation**

Prepend to `crates/cairn-node/src/evidence.rs`:

```rust
//! §5.4 clinician-observed evidence authoring. Composes the pure `cairn-event::evidence`
//! builders into `demographic.field.asserted` events and submits them on an EXISTING
//! patient chart (a John Doe, or any poorly-documented chart) through the reused
//! `submit_event` door. Additive events, low ceremony — the sole contributor is the
//! recording actor with role `recorded`, so no attestation is demanded (mirrors
//! `john_doe::build_callsign_name_body`).
//!
//! Split: pure body assembly (unit-tested, no DB) + the async `assert_observed_evidence`
//! orchestrator (one transaction, so age + sex land atomically).

use cairn_event::demographics::{render_administrative_sex_twin, render_dob_twin};
use cairn_event::evidence::{
    birth_year_range_from_age, estimated_dob_body, observed_sex_body,
    CLINICIAN_OBSERVED_PROVENANCE, YEAR_RANGE_PRECISION,
};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use tokio_postgres::Client;
use uuid::Uuid;

/// schema_version for a demographic field assertion (mirrors `john_doe.rs`).
const DEMOGRAPHIC_FIELD_SCHEMA_VERSION: &str = "demographic.field/1";

/// A clinician's estimated-age observation: `age_years` ± `tolerance_years`, with a
/// mandatory `basis` (§5.4 "estimated age WITH basis"; principle 4).
pub struct AgeObservation {
    pub age_years: u32,
    pub tolerance_years: u32,
    pub basis: String,
}

/// A clinician's observed-sex observation: an OPEN `value` (apparent/phenotypic) with an
/// optional `basis` (how it was observed).
pub struct SexObservation {
    pub value: String,
    pub basis: Option<String>,
}

/// The evidence to record in one call — either or both kinds. `assert_observed_evidence`
/// errors if both are None (nothing to assert).
pub struct ObservedEvidence {
    pub age: Option<AgeObservation>,
    pub sex: Option<SexObservation>,
}

/// Assemble the estimated-age `dob` `EventBody`. Pure: `event_id`/`hlc`/the resolved
/// year range are supplied so the body is fully testable.
pub fn build_estimated_dob_event(
    event_id: Uuid, patient_id: Uuid, min_year: i32, max_year: i32, basis: &str, kid: &str, hlc: Hlc,
) -> EventBody {
    let value = cairn_event::evidence::format_year_range(min_year, max_year);
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: DEMOGRAPHIC_FIELD_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: estimated_dob_body(min_year, max_year, basis, CLINICIAN_OBSERVED_PROVENANCE),
        attachments: vec![],
        plaintext_twin: Some(render_dob_twin(&value, YEAR_RANGE_PRECISION, CLINICIAN_OBSERVED_PROVENANCE)),
    }
}

/// Assemble the observed-sex `administrative-sex` `EventBody`. Pure.
pub fn build_observed_sex_event(
    event_id: Uuid, patient_id: Uuid, value: &str, basis: Option<&str>, kid: &str, hlc: Hlc,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: DEMOGRAPHIC_FIELD_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: observed_sex_body(value, basis, CLINICIAN_OBSERVED_PROVENANCE),
        attachments: vec![],
        plaintext_twin: Some(render_administrative_sex_twin(value, CLINICIAN_OBSERVED_PROVENANCE)),
    }
}

/// Author the supplied clinician-observed evidence on `patient_id` in ONE transaction.
/// `observed_year` is the year the estimate was made (the caller owns the clock). Ticks
/// the HLC once per event (age before sex when both present). Errors if `ev` carries
/// neither kind.
pub async fn assert_observed_evidence(
    client: &mut Client,
    sk: &SigningKey,
    kid: &str,
    node_origin: &str,
    patient_id: Uuid,
    ev: &ObservedEvidence,
    observed_year: i32,
) -> anyhow::Result<()> {
    if ev.age.is_none() && ev.sex.is_none() {
        anyhow::bail!("assert_observed_evidence: supply at least one of age or sex");
    }
    // Build + sign each event OUTSIDE the txn (HLC ticks self-commit and may gap safely).
    let mut signed = Vec::new();
    if let Some(a) = &ev.age {
        let (lo, hi) = birth_year_range_from_age(a.age_years, a.tolerance_years, observed_year);
        let h = crate::db::next_hlc(client, node_origin).await?;
        signed.push(sign(&build_estimated_dob_event(Uuid::now_v7(), patient_id, lo, hi, &a.basis, kid, h), sk)?);
    }
    if let Some(s) = &ev.sex {
        let h = crate::db::next_hlc(client, node_origin).await?;
        signed.push(sign(&build_observed_sex_event(Uuid::now_v7(), patient_id, &s.value, s.basis.as_deref(), kid, h), sk)?);
    }
    let tx = client.transaction().await?;
    for s in &signed {
        tx.execute("SELECT submit_event($1)", &[&s.signed_bytes]).await?;
    }
    tx.commit().await?;
    Ok(())
}
```

Add `pub mod evidence;` to `crates/cairn-node/src/lib.rs`.

> Note: `format_year_range` is called with a `pub use`-free path — confirm `cairn_event::evidence::format_year_range` is public (Task 1 made it `pub`). If clippy flags the long path, add `use cairn_event::evidence::format_year_range;` to the imports.

- [ ] **Step 4: Run the unit tests**

Run: `cargo test -p cairn-node --lib evidence`
Expected: PASS (2 tests).

- [ ] **Step 5: Promote `db::next_hlc` to `pub`**

The integration test (Step 6) authors a competing document-verified dob and needs a
correctly-ticked HLC. In `crates/cairn-node/src/db.rs:175`, change:

```rust
pub(crate) async fn next_hlc(
```
to:
```rust
pub async fn next_hlc(
```

Run: `cargo build -p cairn-node` — still compiles.

- [ ] **Step 6: Write the DB-gated integration tests**

Create `crates/cairn-node/tests/observed_evidence.rs`, mirroring `tests/john_doe.rs`'s setup.
The `author_document_dob` helper authors a real document-verified point dob through the same
`submit_event` door (rank 60), so the displacement test is faithful — no SQL back-door, no
placeholder:

```rust
//! Integration coverage for §5.4 clinician-observed evidence: `evidence::assert_observed_evidence`
//! authors an estimated-age `year-range` dob (clinician-observed) and/or an observed
//! `administrative-sex`, through the real `submit_event` door, and the db/011/db/013
//! projections carry them. Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via
//! `db::test_serial_guard`.

use cairn_event::generate_key;
use cairn_node::db;
use cairn_node::evidence::{self, AgeObservation, ObservedEvidence, SexObservation};
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

async fn setup(c: &Client) -> (cairn_event::SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name CASCADE").await.unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid]).await.unwrap();
    (sk, kid)
}

/// Read one (value, facets, provenance) demographic projection row for a field.
async fn demographic_of(c: &Client, patient: Uuid, field: &str) -> Option<(String, serde_json::Value, String)> {
    let p = patient.to_string();
    c.query_opt(
        "SELECT value, facets, provenance FROM patient_demographic \
         WHERE patient_id = $1::text::uuid AND field = $2", &[&p, &field])
        .await.unwrap().map(|r| (r.get(0), r.get(1), r.get(2)))
}

/// Author a single document-verified point dob (`precision=day`, rank 60) through the real
/// door, so a test can prove it DISPLACES a rank-30 clinician estimate in the projection.
async fn author_document_dob(
    db: &mut Client, sk: &cairn_event::SigningKey, kid: &str, node: &str, patient: Uuid, iso: &str,
) {
    let h = db::next_hlc(db, node).await.unwrap();
    let payload = cairn_event::demographics::dob_assertion_body(iso, "day", Some("passport"), "document-verified");
    let body = cairn_event::EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: "demographic.field/1".into(),
        hlc: h,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(cairn_event::demographics::render_dob_twin(iso, "day", "document-verified")),
    };
    let signed = cairn_event::sign(&body, sk).unwrap();
    db.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await.unwrap();
}

#[tokio::test]
async fn estimated_age_lands_as_a_clinician_observed_year_range_dob() {
    let Some(cs) = cs() else { eprintln!("skipping: no CAIRN_TEST_PG"); return; };
    let mut db = db::connect(&cs).await.unwrap();
    let _g = db::test_serial_guard(&db).await;
    let (sk, kid) = setup(&db).await;
    let patient = Uuid::now_v7();

    let ev = ObservedEvidence {
        age: Some(AgeObservation { age_years: 40, tolerance_years: 5, basis: "dentition, greying".into() }),
        sex: None,
    };
    evidence::assert_observed_evidence(&mut db, &sk, &kid, "test-node", patient, &ev, 2026).await.unwrap();

    let (value, facets, prov) = demographic_of(&db, patient, "dob").await.expect("dob projected");
    assert_eq!(value, "1981/1991");
    assert_eq!(facets["precision"], "year-range");
    assert_eq!(prov, "clinician-observed");
}

#[tokio::test]
async fn a_document_verified_dob_displaces_the_estimate() {
    let Some(cs) = cs() else { eprintln!("skipping: no CAIRN_TEST_PG"); return; };
    let mut db = db::connect(&cs).await.unwrap();
    let _g = db::test_serial_guard(&db).await;
    let (sk, kid) = setup(&db).await;
    let patient = Uuid::now_v7();

    // First the clinician estimate (rank 30)...
    let est = ObservedEvidence {
        age: Some(AgeObservation { age_years: 40, tolerance_years: 5, basis: "dentition".into() }),
        sex: None,
    };
    evidence::assert_observed_evidence(&mut db, &sk, &kid, "test-node", patient, &est, 2026).await.unwrap();
    // ...then a document-verified exact dob (rank 60) — must win the projection.
    author_document_dob(&mut db, &sk, &kid, "test-node", patient, "1985-03-12").await;

    let (value, _facets, prov) = demographic_of(&db, patient, "dob").await.expect("dob projected");
    assert_eq!(prov, "document-verified", "the document must displace the clinician estimate");
    assert_eq!(value, "1985-03-12");
}

#[tokio::test]
async fn observed_sex_lands_on_administrative_sex() {
    let Some(cs) = cs() else { eprintln!("skipping: no CAIRN_TEST_PG"); return; };
    let mut db = db::connect(&cs).await.unwrap();
    let _g = db::test_serial_guard(&db).await;
    let (sk, kid) = setup(&db).await;
    let patient = Uuid::now_v7();

    let ev = ObservedEvidence {
        age: None,
        sex: Some(SexObservation { value: "male".into(), basis: Some("external genitalia".into()) }),
    };
    evidence::assert_observed_evidence(&mut db, &sk, &kid, "test-node", patient, &ev, 2026).await.unwrap();

    let (value, _facets, prov) = demographic_of(&db, patient, "administrative-sex").await.expect("sex projected");
    assert_eq!(value, "male");
    assert_eq!(prov, "clinician-observed");
    // sex-at-birth must be UNTOUCHED — the clinician never claimed the birth fact.
    assert!(demographic_of(&db, patient, "sex-at-birth").await.is_none());
}

#[tokio::test]
async fn age_and_sex_are_authored_atomically() {
    let Some(cs) = cs() else { eprintln!("skipping: no CAIRN_TEST_PG"); return; };
    let mut db = db::connect(&cs).await.unwrap();
    let _g = db::test_serial_guard(&db).await;
    let (sk, kid) = setup(&db).await;
    let patient = Uuid::now_v7();

    let ev = ObservedEvidence {
        age: Some(AgeObservation { age_years: 25, tolerance_years: 3, basis: "young adult".into() }),
        sex: Some(SexObservation { value: "female".into(), basis: None }),
    };
    evidence::assert_observed_evidence(&mut db, &sk, &kid, "test-node", patient, &ev, 2020).await.unwrap();

    assert_eq!(demographic_of(&db, patient, "dob").await.unwrap().0, "1992/1998");
    assert_eq!(demographic_of(&db, patient, "administrative-sex").await.unwrap().0, "female");
}
```

- [ ] **Step 7: Run the DB-gated integration tests**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test observed_evidence`
Expected: PASS (4 tests). (Without `CAIRN_TEST_PG` they print "skipping" and pass.)

- [ ] **Step 8: Clippy + workspace tests**

Run: `cargo clippy -p cairn-node --all-targets -- -D warnings && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node`
Expected: no warnings; all pass.

- [ ] **Step 9: Commit**

```bash
git add crates/cairn-node/src/evidence.rs crates/cairn-node/src/lib.rs crates/cairn-node/src/db.rs crates/cairn-node/tests/observed_evidence.rs
git commit -m "feat(cairn-node): assert-observed-evidence orchestrator (estimated-age dob + observed sex)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: `assert-observed-evidence` CLI subcommand

**Files:**
- Modify: `crates/cairn-node/src/main.rs` (add a `Cmd::AssertObservedEvidence` variant + its match arm)

**Interfaces:**
- Consumes: `cairn_node::evidence::{assert_observed_evidence, AgeObservation, ObservedEvidence, SexObservation}`; the existing `load_signing_key`, `ensure_registration_actor`, `cairn_node::db::connect`, `cairn_node::identity::load_local`.
- Produces: a CLI subcommand; no new library surface.

- [ ] **Step 1: Add the subcommand variant**

In `crates/cairn-node/src/main.rs`, after the `RegisterJohnDoe { … }` variant (ends at line ~299), add:

```rust
    /// Record clinician-observed identity evidence on an existing chart (§5.4): an
    /// estimated age (-> a year-range dob) and/or an observed sex (-> administrative-sex),
    /// both provenance `clinician-observed`. Supply at least one of --age / --sex.
    AssertObservedEvidence {
        /// The patient UUID to record evidence on.
        patient: Uuid,
        /// Estimated age in years (apparent age).
        #[arg(long)]
        age: Option<u32>,
        /// ± tolerance in years around the estimated age (default 5).
        #[arg(long, default_value_t = 5)]
        tol: u32,
        /// How the age was estimated (required when --age is given).
        #[arg(long)]
        age_basis: Option<String>,
        /// Observed (apparent) sex — an open string.
        #[arg(long)]
        sex: Option<String>,
        /// How the sex was observed (optional).
        #[arg(long)]
        sex_basis: Option<String>,
    },
```

- [ ] **Step 2: Add the match arm**

After the `Cmd::RegisterJohnDoe { … } => { … }` arm (ends line ~813), add:

```rust
        Cmd::AssertObservedEvidence { patient, age, tol, age_basis, sex, sex_basis } => {
            let sk = load_signing_key(&cli.key, true)?;
            let kid = hex::encode(sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            // Observation year comes from the node's own DB clock (the DB is the clock).
            let observed_year: i32 = db.query_one("SELECT extract(year FROM current_date)::int", &[])
                .await?.get(0);
            ensure_registration_actor(&db, &kid).await?;

            let age_obs = match (age, age_basis) {
                (Some(age_years), Some(basis)) =>
                    Some(cairn_node::evidence::AgeObservation { age_years, tolerance_years: tol, basis }),
                (Some(_), None) => anyhow::bail!("--age requires --age-basis (§5.4: estimated age WITH basis)"),
                (None, _) => None,
            };
            let sex_obs = sex.map(|value| cairn_node::evidence::SexObservation { value, basis: sex_basis });
            let ev = cairn_node::evidence::ObservedEvidence { age: age_obs, sex: sex_obs };

            cairn_node::evidence::assert_observed_evidence(
                &mut db, &sk, &kid, &id.node_id_hex, patient, &ev, observed_year).await?;
            println!("recorded clinician-observed evidence on {patient}");
        }
```

- [ ] **Step 3: Build**

Run: `cargo build -p cairn-node`
Expected: compiles clean.

- [ ] **Step 4: Manual CLI smoke (requires a provisioned node)**

On a provisioned dev node, register a John Doe, then attach evidence and confirm the projection:

```bash
cargo run -p cairn-node -- register-john-doe --class ED
# note the printed patient UUID <PID>
cargo run -p cairn-node -- assert-observed-evidence <PID> \
    --age 40 --tol 5 --age-basis "dentition, greying" --sex male --sex-basis "external genitalia"
psql "$CAIRN_TEST_PG" -c "SELECT field, value, provenance FROM patient_demographic WHERE patient_id='<PID>';"
```

Expected: rows `dob | 1981/1991 | clinician-observed` and `administrative-sex | male | clinician-observed`; the John-Doe chart still renders *unconfirmed* on `chart_trust` (unchanged by this evidence).

- [ ] **Step 5: Clippy**

Run: `cargo clippy -p cairn-node --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/src/main.rs
git commit -m "feat(cairn-node): assert-observed-evidence CLI subcommand

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Matcher `DateValue` gains a birth-year interval

**Files:**
- Modify: `matcher/src/cairn_matcher/records.py:26-36`
- Test: `matcher/tests/test_date_value_range.py` (create)

**Interfaces:**
- Produces: `DateValue` with two new optional fields `year_min: int | None = None`, `year_max: int | None = None`, and a `@property is_range -> bool`. Point-date shape (`year/month/day`) is unchanged.

- [ ] **Step 1: Write the failing test**

Create `matcher/tests/test_date_value_range.py`:

```python
"""A DateValue can carry an inclusive birth-year interval (year_min..year_max)."""
from cairn_matcher.records import DateValue


def test_point_date_is_not_a_range():
    assert DateValue(year=1985, month=3, day=12).is_range is False


def test_year_interval_is_a_range():
    dv = DateValue(year_min=1981, year_max=1991)
    assert dv.is_range is True
    assert dv.year_min == 1981
    assert dv.year_max == 1991
    # A range has no point parts.
    assert dv.year is None and dv.month is None and dv.day is None
```

- [ ] **Step 2: Run to verify failure**

Run: `cd matcher && uv run pytest tests/test_date_value_range.py -v`
Expected: FAIL — `TypeError: __init__() got an unexpected keyword argument 'year_min'`.

- [ ] **Step 3: Implement**

Edit `matcher/src/cairn_matcher/records.py`, replacing the `DateValue` body:

```python
@dataclass(frozen=True)
class DateValue:
    """A canonical, already-parsed date. Precision is implied by which parts are present.

    Two shapes, never mixed:
      * a POINT date — some prefix of (year, month, day) present, the rest None;
      * a birth-year RANGE — year_min..year_max present (inclusive), all point parts None.
        A range is how a clinician-observed *estimated age* is carried (§5.4): honest
        imprecision, never a false-precise midpoint (principle 4).

    The core never parses a locale date STRING into this — that is locale-specific and
    belongs to B2/locale packs. compare_dob operates only on the parts present here.
    """

    year: int | None = None
    month: int | None = None
    day: int | None = None
    year_min: int | None = None
    year_max: int | None = None

    @property
    def is_range(self) -> bool:
        """True iff this is a birth-year interval rather than a point date."""
        return self.year_min is not None and self.year_max is not None
```

- [ ] **Step 4: Run to verify pass**

Run: `cd matcher && uv run pytest tests/test_date_value_range.py -v`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/records.py matcher/tests/test_date_value_range.py
git commit -m "feat(matcher): DateValue carries an optional birth-year interval

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: `parse_dob` reads a `year-range` value

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/adapter.py:39-73`
- Test: `matcher/tests/test_parse_dob_range.py` (create)

**Interfaces:**
- Consumes: `DateValue` (with `year_min/year_max` from Task 4).
- Produces: `parse_dob(value, precision)` returns a range `DateValue` when `precision == "year-range"` and `value` is `"<yyyy>/<yyyy>"` with min ≤ max; `None` (safe degrade) otherwise. Point behaviour unchanged.

- [ ] **Step 1: Write the failing tests**

Create `matcher/tests/test_parse_dob_range.py`:

```python
"""parse_dob understands the §5.4 'year-range' precision (e.g. '1981/1991')."""
from cairn_matcher.pipeline.adapter import parse_dob


def test_year_range_parses_to_an_interval():
    dv = parse_dob("1981/1991", "year-range")
    assert dv is not None and dv.is_range
    assert (dv.year_min, dv.year_max) == (1981, 1991)


def test_single_year_range_is_a_degenerate_interval():
    dv = parse_dob("1990/1990", "year-range")
    assert (dv.year_min, dv.year_max) == (1990, 1990)


def test_reversed_or_malformed_range_degrades_to_none():
    assert parse_dob("1991/1981", "year-range") is None      # min > max
    assert parse_dob("1981", "year-range") is None            # no separator
    assert parse_dob("1981/xx", "year-range") is None         # non-numeric
    assert parse_dob("81/91", "year-range") is None           # not 4-digit years


def test_point_precision_still_parses_a_point_date():
    dv = parse_dob("1985-03-12", "day")
    assert dv is not None and not dv.is_range
    assert (dv.year, dv.month, dv.day) == (1985, 3, 12)
```

- [ ] **Step 2: Run to verify failure**

Run: `cd matcher && uv run pytest tests/test_parse_dob_range.py -v`
Expected: FAIL — `year-range` currently hits `precision not in _PRECISION_PARTS` → returns None, so the first test fails.

- [ ] **Step 3: Implement**

In `matcher/src/cairn_matcher/pipeline/adapter.py`, add the range branch at the top of `parse_dob` (before the `_PRECISION_PARTS` guard) and a helper below it:

```python
def parse_dob(value: str | None, precision: str | None) -> DateValue | None:
    """Extract a DateValue from a dob value at the projection's declared precision.

    A 'year-range' precision (§5.4 clinician-observed estimated age) parses '<yyyy>/<yyyy>'
    into an inclusive birth-year interval. All other precisions are point dates parsed from
    an ISO value. Returns None (a safe, gradeable absence) for anything unreadable.
    """
    if not value:
        return None
    if precision == "year-range":
        return _parse_year_range(value)
    if precision not in _PRECISION_PARTS:
        return None
    # ... existing point-date body unchanged ...
```

Add the helper immediately after `parse_dob`:

```python
def _parse_year_range(value: str) -> DateValue | None:
    """Parse '<yyyy>/<yyyy>' -> a birth-year interval DateValue, else None (safe degrade).

    Both sides must be full 4-digit years (mirroring the point-date 4-digit discipline) and
    min <= max. A malformed range is simply absent — never a guessed or reversed interval.
    """
    parts = value.split("/")
    if len(parts) != 2:
        return None
    lo, hi = parts
    if len(lo) != 4 or not lo.isdigit() or len(hi) != 4 or not hi.isdigit():
        return None
    lo_i, hi_i = int(lo), int(hi)
    if lo_i > hi_i:
        return None
    return DateValue(year_min=lo_i, year_max=hi_i)
```

- [ ] **Step 4: Run to verify pass**

Run: `cd matcher && uv run pytest tests/test_parse_dob_range.py -v`
Expected: PASS (4 tests).

- [ ] **Step 5: Full pure suite (no regression)**

Run: `cd matcher && uv run pytest`
Expected: all pass (existing point-date adapter tests unaffected).

- [ ] **Step 6: Commit**

```bash
git add matcher/src/cairn_matcher/pipeline/adapter.py matcher/tests/test_parse_dob_range.py
git commit -m "feat(matcher): parse_dob reads the year-range precision into an interval

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: `compare_dob` — range-aware, positive-only

**Files:**
- Modify: `matcher/src/cairn_matcher/comparators.py:122-151`
- Test: `matcher/tests/test_compare_dob_range.py` (create)

**Interfaces:**
- Consumes: `DateValue` (point or range), `AgreementLevel`, `Context`.
- Produces: `compare_dob` that, when EITHER side is a range, returns `PARTIAL` on year-interval overlap and `INSUFFICIENT_DATA` otherwise — **never `DISAGREE`** (a soft estimate supports but never suppresses a match; mirrors `compare_identifier_sets`). Point-vs-point behaviour unchanged.

- [ ] **Step 1: Write the failing tests**

Create `matcher/tests/test_compare_dob_range.py`:

```python
"""compare_dob is range-aware and positive-only for clinician-observed estimates (§5.4)."""
from cairn_matcher.agreement import AgreementLevel, Context
from cairn_matcher.comparators import compare_dob
from cairn_matcher.records import DateValue

CTX = Context()  # compare_dob does not read ctx for ranges; a default Context is fine


def _point(y, m=None, d=None):
    return DateValue(year=y, month=m, day=d)


def _range(lo, hi):
    return DateValue(year_min=lo, year_max=hi)


def test_point_inside_range_is_partial():
    assert compare_dob(_range(1981, 1991), _point(1985, 3, 12), CTX) == AgreementLevel.PARTIAL
    # order-independent
    assert compare_dob(_point(1985, 3, 12), _range(1981, 1991), CTX) == AgreementLevel.PARTIAL


def test_point_outside_range_is_insufficient_never_disagree():
    got = compare_dob(_range(1981, 1991), _point(1950, 1, 1), CTX)
    assert got == AgreementLevel.INSUFFICIENT_DATA


def test_overlapping_ranges_are_partial():
    assert compare_dob(_range(1981, 1991), _range(1988, 1995), CTX) == AgreementLevel.PARTIAL


def test_disjoint_ranges_are_insufficient():
    assert compare_dob(_range(1981, 1991), _range(2000, 2005), CTX) == AgreementLevel.INSUFFICIENT_DATA


def test_range_vs_point_with_no_year_is_insufficient():
    assert compare_dob(_range(1981, 1991), _point(None), CTX) == AgreementLevel.INSUFFICIENT_DATA


def test_point_vs_point_regression_unchanged():
    assert compare_dob(_point(1985, 3, 12), _point(1985, 3, 12), CTX) == AgreementLevel.EXACT
    assert compare_dob(_point(1985), _point(1985, 3, 12), CTX) == AgreementLevel.PARTIAL
    assert compare_dob(_point(1985), _point(1990), CTX) == AgreementLevel.DISAGREE
```

- [ ] **Step 2: Run to verify failure**

Run: `cd matcher && uv run pytest tests/test_compare_dob_range.py -v`
Expected: FAIL — range inputs currently fall through the point logic (`getattr(a, "year")` is None on a range → no shared parts → INSUFFICIENT for the inside-range case, so `test_point_inside_range_is_partial` fails).

- [ ] **Step 3: Implement**

In `matcher/src/cairn_matcher/comparators.py`, insert the range branch near the top of `compare_dob` (after the None / type guards, before the `shared = [...]` point logic), and add two helpers below the function:

```python
    if a is None or b is None:
        return AgreementLevel.INSUFFICIENT_DATA
    if not isinstance(a, DateValue) or not isinstance(b, DateValue):
        raise MatcherTypeError("compare_dob expects DateValue or None")

    # §5.4 clinician-observed estimated ages arrive as birth-year RANGES. A soft estimate
    # may only SUPPORT a match (interval overlap -> PARTIAL), never contradict one: it is
    # positive-only, exactly like compare_identifier_sets. Never DISAGREE for a range — a
    # visual age guess must not suppress a true returning-patient match (§5.4 recognition,
    # principle 4). A point date participates as the degenerate interval [year, year].
    if a.is_range or b.is_range:
        return _compare_year_intervals(a, b)

    # ... existing point-vs-point body unchanged (shared parts / depth) ...
```

Add below `compare_dob`:

```python
def _as_year_interval(d: DateValue) -> tuple[int, int] | None:
    """A DateValue's inclusive birth-year interval: the range itself, or [year, year] for a
    point date with a year, or None when there is no year to place on the axis."""
    if d.is_range:
        return (d.year_min, d.year_max)
    if d.year is not None:
        return (d.year, d.year)
    return None


def _compare_year_intervals(a: DateValue, b: DateValue) -> AgreementLevel:
    """Positive-only birth-year overlap: PARTIAL if the intervals intersect, else
    INSUFFICIENT_DATA. Never DISAGREE (a soft estimate cannot veto a match)."""
    ia = _as_year_interval(a)
    ib = _as_year_interval(b)
    if ia is None or ib is None:
        return AgreementLevel.INSUFFICIENT_DATA
    if max(ia[0], ib[0]) <= min(ia[1], ib[1]):
        return AgreementLevel.PARTIAL
    return AgreementLevel.INSUFFICIENT_DATA
```

- [ ] **Step 4: Run to verify pass**

Run: `cd matcher && uv run pytest tests/test_compare_dob_range.py -v`
Expected: PASS (6 tests).

- [ ] **Step 5: Full pure suite + ruff**

Run: `cd matcher && uv run pytest && uv run ruff check .`
Expected: all pass; ruff clean.

- [ ] **Step 6: Commit**

```bash
git add matcher/src/cairn_matcher/comparators.py matcher/tests/test_compare_dob_range.py
git commit -m "feat(matcher): range-aware positive-only compare_dob for clinician-observed ages

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: DB-gated e2e — an estimated range scores as a positive signal

**Files:**
- Test: `matcher/tests/test_observed_age_pipeline.py` (create)

**Interfaces:**
- Consumes: the `pipeline` extra — `cairn_matcher.pipeline.db.load_candidate(conn, patient_id) -> CandidateRecord`, `cairn_matcher.orchestrator.field_comparisons(a, b) -> list[FieldComparison]` (each `FieldComparison` has `.field: str`, `.level: AgreementLevel`), the `pg_conn` + `seed_patient` conftest fixtures (`seed_patient(conn, id, *, dob=(value, provenance_rank, precision), sex=(value, rank))`), and the `patient_demographic` projection.
- Produces: e2e proof that a John-Doe `year-range` dob, projected → `load_candidate`-adapted → compared, yields a positive (`PARTIAL`) dob agreement when a candidate's real DOB falls inside the range, and `INSUFFICIENT_DATA` (never a penalty) when it falls outside.

> This exercises the real projection → adapter → comparator loop on live DB rows. We assert at the `field_comparisons` seam (not `runner.propose`, which returns only a `Band`). Blocking is out of the loop on purpose — see the design's deferred note: a range does not generate a blocking key, so scoring is where the estimate helps once a pair is blocked by another key.

- [ ] **Step 1: Confirm the conftest fixture names**

Run: `grep -n "def pg_conn\|def seed_patient" matcher/tests/conftest.py`
Expected: both exist (`pg_conn` at ~line 50, `seed_patient` at ~line 68). If the connection fixture has a different name, match it in the test below.

- [ ] **Step 2: Write the failing e2e test**

Create `matcher/tests/test_observed_age_pipeline.py`:

```python
"""§5.4 e2e: a clinician-observed estimated-age range scores as a positive dob signal.

Exercises the real projection -> load_candidate -> field_comparisons loop on live DB rows.
"""
import os
import uuid

import pytest

pytestmark = pytest.mark.skipif(not os.environ.get("CAIRN_TEST_PG"), reason="no CAIRN_TEST_PG")

from cairn_matcher.agreement import AgreementLevel
from cairn_matcher.orchestrator import field_comparisons
from cairn_matcher.pipeline.db import load_candidate


def _uid() -> str:
    return str(uuid.uuid4())


def _dob_level(conn, a: str, b: str) -> AgreementLevel:
    """The dob AgreementLevel for the pair, straight from the real scoring path."""
    rec_a = load_candidate(conn, a)
    rec_b = load_candidate(conn, b)
    comparisons = field_comparisons(rec_a, rec_b)
    return next(fc.level for fc in comparisons if fc.field == "dob")


def test_candidate_dob_inside_the_estimated_range_is_a_positive_dob_signal(pg_conn, seed_patient):
    john_doe, candidate = _uid(), _uid()
    # John Doe: an estimated birth-year range (clinician-observed, rank 30).
    seed_patient(pg_conn, john_doe, dob=("1981/1991", 30, "year-range"), sex=("male", 30))
    # Candidate: a real document dob INSIDE the range.
    seed_patient(pg_conn, candidate, dob=("1985-03-12", 60, "day"), sex=("male", 60))

    assert _dob_level(pg_conn, john_doe, candidate) == AgreementLevel.PARTIAL


def test_candidate_dob_outside_the_range_is_neither_penalty_nor_veto(pg_conn, seed_patient):
    john_doe, candidate = _uid(), _uid()
    seed_patient(pg_conn, john_doe, dob=("1981/1991", 30, "year-range"))
    seed_patient(pg_conn, candidate, dob=("1950-01-01", 60, "day"))

    # Outside the range -> INSUFFICIENT_DATA (positive-only; never a DISAGREE penalty).
    assert _dob_level(pg_conn, john_doe, candidate) == AgreementLevel.INSUFFICIENT_DATA
```

> If `field_comparisons` requires its third `config` argument in this codebase version, it defaults (`DEFAULT_CONFIG`) — call it with two args as shown.

- [ ] **Step 3: Run to verify failure THEN pass**

First run BEFORE Tasks 4–6 are merged would fail; since this task runs after them, the range support exists. Run to confirm the loop is wired:

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_observed_age_pipeline.py -v`
Expected: PASS (2 tests). If `load_candidate` returns `dob=None` for the range (i.e. the adapter dropped it), that means Task 5's `parse_dob` range branch is not wired — fix Task 5 before proceeding.

- [ ] **Step 4: Full DB matcher suite (no regression)**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest && uv run ruff check .`
Expected: all pass; ruff clean.

- [ ] **Step 5: Commit**

```bash
git add matcher/tests/test_observed_age_pipeline.py
git commit -m "test(matcher): e2e — estimated-age range scores as a positive dob signal

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Final verification (before opening the PR)

- [ ] `cargo test --workspace` + `cargo clippy --workspace --all-targets -- -D warnings` (with `CAIRN_TEST_PG` set) — all green.
- [ ] `cd matcher && uv run pytest` (pure) and `CAIRN_TEST_PG=… uv run --extra pipeline pytest` (DB) — all green; `uv run ruff check .` clean.
- [ ] Update `docs/HANDOVER.md` + `docs/ROADMAP.md` (concise; §5.4 slice B built; deferred birth-year-range blocking pass recorded).
- [ ] Commit doc updates, push the branch, open a PR to `main` describing the three layers + the honest blocking limitation, linking §5.4.
```
