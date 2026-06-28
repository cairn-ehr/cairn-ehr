# Demographics §4.3 Address Slice — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the §4.3 culture-neutral three-facet address (display/geo/structured) to the demographics subsystem on `cairn-node`, with a per-use recency-first display-winner.

**Architecture:** Reuses the slice-2 generic `demographic.field.asserted` event with `field:"address"` and `value` = the mandatory `display` string (so the generic floor's non-empty-`value` check already covers it). Pure `cairn-event` builders shape the event; an additive `db/014` extends the shared structural floor and adds a retained-set table (`patient_address`) + a per-use display-winner VIEW (`patient_address_current`), mirroring the names slice (db/012). No new event type; no change to the validated `submit_event` door.

**Tech Stack:** Rust (`cairn-event` pure builders, `cairn-node` integration tests via `tokio-postgres`), PostgreSQL ≥ 18 + the `cairn_pgx` extension, PL/pgSQL.

## Global Constraints

- **AGPL-3.0**; every dependency AGPL-3.0-compatible (no new deps are needed here).
- **TDD** — failing test first, then minimal code. Load-bearing on this safety-critical surface (§9).
- **Reviewer-legible inline docs** for a junior contributor on every non-trivial function/trigger.
- **Files under ~500 lines** where feasible; `db/014` and `demographics.rs` stay well under.
- **Floor stays culture-neutral** (principle 12): structural invariants only, never holds a profile, never rejects on validation (principle 4). Lat/lon range bounds and `display==formatter(parts)` consistency are advisory — **out of scope**.
- **All tests pass before committing.**
- **DB-gated tests** read `$CAIRN_TEST_PG`; locally `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + cairn_pgx). They self-serialize cluster-wide via `db::test_serial_guard`.
- Optional facets are **omitted entirely when absent, never serialized as `null`**.
- Branch: `demographics-address` (already created; the design doc is committed there).

---

### Task 1: `cairn-event` address builders (pure)

**Files:**
- Modify: `crates/cairn-event/src/demographics.rs` (append builders + `#[cfg(test)]` unit tests)

**Interfaces:**
- Consumes: `serde_json::{json, Value}` (already imported); the existing `demographic_field_body(field, value, facets, provenance)` helper.
- Produces:
  - `pub struct Geo<'a> { pub lat: f64, pub lon: f64, pub accuracy_m: f64, pub basis: &'a str }`
  - `pub struct StructuredAddress<'a> { pub profile: &'a str, pub parts: serde_json::Value }`
  - `pub struct AddressAssertion<'a> { pub display: &'a str, pub provenance: &'a str, pub use_: Option<&'a str>, pub geo: Option<Geo<'a>>, pub structured: Option<StructuredAddress<'a>> }`
  - `pub fn address_assertion_body(a: &AddressAssertion) -> serde_json::Value`
  - `pub fn render_address_twin(a: &AddressAssertion) -> String`

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block in `crates/cairn-event/src/demographics.rs`:

```rust
    fn sample_address() -> AddressAssertion<'static> {
        AddressAssertion {
            display: "12 Main St, Springfield",
            provenance: "patient-stated",
            use_: Some("residential"),
            geo: Some(Geo { lat: -33.87, lon: 151.21, accuracy_m: 10.0, basis: "device_gps" }),
            structured: Some(StructuredAddress {
                profile: "au-address@b3-xyz",
                parts: json!({ "street": "12 Main St", "town": "Springfield", "country": "AU" }),
            }),
        }
    }

    #[test]
    fn address_body_carries_display_as_value_and_all_facets() {
        let v = address_assertion_body(&sample_address());
        assert_eq!(v["field"], "address");
        assert_eq!(v["value"], "12 Main St, Springfield"); // display is the value-core
        assert_eq!(v["provenance"], "patient-stated");
        assert_eq!(v["facets"]["use"], "residential");
        assert_eq!(v["facets"]["geo"]["lat"], -33.87);
        assert_eq!(v["facets"]["geo"]["lon"], 151.21);
        assert_eq!(v["facets"]["geo"]["accuracy_m"], 10.0);
        assert_eq!(v["facets"]["geo"]["basis"], "device_gps");
        assert_eq!(v["facets"]["structured"]["profile"], "au-address@b3-xyz");
        assert_eq!(v["facets"]["structured"]["parts"]["town"], "Springfield");
    }

    #[test]
    fn address_body_omits_absent_facets_never_null() {
        let a = AddressAssertion {
            display: "Refugee camp sector 4, tent 27",
            provenance: "clinician-observed",
            use_: None, geo: None, structured: None,
        };
        let v = address_assertion_body(&a);
        assert_eq!(v["field"], "address");
        assert_eq!(v["value"], "Refugee camp sector 4, tent 27");
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("facets"), "no use/geo/structured ⇒ no facets bag, never null");
    }

    #[test]
    fn address_body_geo_only_omits_use_and_structured() {
        let a = AddressAssertion {
            display: "-33.87, 151.21",
            provenance: "declared",
            use_: None,
            geo: Some(Geo { lat: -33.87, lon: 151.21, accuracy_m: 2000.0, basis: "region_centroid" }),
            structured: None,
        };
        let v = address_assertion_body(&a);
        let facets = v["facets"].as_object().unwrap();
        assert!(facets.contains_key("geo"));
        assert!(!facets.contains_key("use"), "absent use omitted");
        assert!(!facets.contains_key("structured"), "absent structured omitted");
    }

    #[test]
    fn address_twin_uses_use_when_present_else_provenance() {
        assert_eq!(
            render_address_twin(&sample_address()),
            "Address (residential): 12 Main St, Springfield"
        );
        let a = AddressAssertion {
            display: "Tent 27", provenance: "clinician-observed",
            use_: None, geo: None, structured: None,
        };
        assert_eq!(render_address_twin(&a), "Address (clinician-observed): Tent 27");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cairn-event address_ -- --nocapture`
Expected: FAIL — `cannot find type AddressAssertion` / `cannot find function address_assertion_body`.

- [ ] **Step 3: Write the minimal implementation**

Append to `crates/cairn-event/src/demographics.rs` *before* the `#[cfg(test)]` block:

```rust
/// A precision-aware geolocation facet (§4.3, principle 4 in space). `accuracy_m` is
/// the honest uncertainty radius (GPS ±10 m, village centroid ±2 km); `basis` is its
/// provenance (`device_gps` / `map_pin` / `geocoded_from_text` / `region_centroid` /
/// `declared`). The culture-neutral universal locator — often the only viable address
/// in informal-settlement / refugee / disaster / remote contexts.
pub struct Geo<'a> {
    pub lat: f64,
    pub lon: f64,
    pub accuracy_m: f64,
    pub basis: &'a str,
}

/// The optional structured-address facet (§4.3): an open bag of named `parts` plus a
/// content-addressed locale `profile` (`namespace@hash`). The core never interprets a
/// part name or value — the profile (which travels the distribution plane) does. `parts`
/// is expected to be a JSON object of text values; the in-DB floor enforces that shape.
pub struct StructuredAddress<'a> {
    pub profile: &'a str,
    pub parts: Value,
}

/// One §4.3 address assertion. `display` is the mandatory, complete human-readable
/// address — the value-core of the §4.5 legibility twin and the projection's member key
/// — carried as the generic field `value`, so the existing non-empty-`value` floor check
/// covers it. `geo` and `structured` are optional facets. `use_` is the recommended-but-
/// open use category (`residential`/`postal`/`work`/…), omitted entirely when None.
pub struct AddressAssertion<'a> {
    pub display: &'a str,
    pub provenance: &'a str,
    pub use_: Option<&'a str>,
    pub geo: Option<Geo<'a>>,
    pub structured: Option<StructuredAddress<'a>>,
}

/// Build the §4.3 address-assertion payload (the value of `EventBody.payload`). `display`
/// becomes the generic `value`; `use`/`geo`/`structured` go in the `facets` bag and are
/// each omitted when absent (never serialized null) so the in-DB floor's key-presence
/// checks see exactly what the author asserted. When no facet is present, no `facets` key
/// is emitted at all (matching the names/identifier builders).
pub fn address_assertion_body(a: &AddressAssertion) -> Value {
    let mut facets = serde_json::Map::new();
    if let Some(u) = a.use_ {
        facets.insert("use".into(), json!(u));
    }
    if let Some(g) = &a.geo {
        facets.insert("geo".into(), json!({
            "lat": g.lat, "lon": g.lon, "accuracy_m": g.accuracy_m, "basis": g.basis,
        }));
    }
    if let Some(s) = &a.structured {
        facets.insert("structured".into(), json!({
            "profile": s.profile, "parts": s.parts,
        }));
    }
    let facets = if facets.is_empty() { None } else { Some(Value::Object(facets)) };
    demographic_field_body("address", a.display, facets, a.provenance)
}

/// Render the §4.5 legibility twin for an address: `"Address (<use|provenance>): <display>"`.
/// Mirrors `render_name_twin` — the `use` sits in the parens when present, else the
/// provenance, so the parenthetical is never empty. `geo`/`structured` do not enter the
/// twin: `display` is by definition the complete human-readable address.
pub fn render_address_twin(a: &AddressAssertion) -> String {
    let context = a.use_.unwrap_or(a.provenance);
    format!("Address ({context}): {}", a.display)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cairn-event address_ -- --nocapture`
Expected: PASS (4 new tests). Then `cargo test -p cairn-event` — the existing demographics unit tests stay green.

- [ ] **Step 5: Clippy + commit**

Run: `cargo clippy -p cairn-event --all-targets -- -D warnings`
Expected: clean.

```bash
git add crates/cairn-event/src/demographics.rs
git commit -m "feat(cairn-event): §4.3 address assertion builders + twin

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `db/014` floor extension + projection + integration suite

**Files:**
- Create: `db/014_demographics_address.sql`
- Modify: `crates/cairn-node/src/db.rs` (register migration 014 in the `SCHEMA` array)
- Create: `crates/cairn-node/tests/demographics_address.rs`

**Interfaces:**
- Consumes: the existing `submit_event` door (db/005), the `cairn_event_twin` hook + `cairn_check_demographic_field` floor (db/011, last redefined there — db/013 left it untouched), `cairn_provenance_rank` (db/011), `db::{connect_and_load_schema, test_serial_guard}`, and `cairn_event::demographics::{address_assertion_body, render_address_twin, AddressAssertion, Geo, StructuredAddress}` (Task 1).
- Produces (SQL objects later code/tests rely on): table `patient_address(patient_id, use_key, display, use_raw, geo, structured, provenance, provenance_rank, last_hlc_wall, last_hlc_count, asserted_origin, updated_at)`; view `patient_address_current` (one row per `(patient_id, use_key)`); trigger `patient_address_apply_trg`; redefined `cairn_check_demographic_field` with the `field='address'` branch (dob branch carried forward verbatim).

- [ ] **Step 1: Write the failing integration test file**

Create `crates/cairn-node/tests/demographics_address.rs`:

```rust
//! Integration coverage for the §4.3 ADDRESS field: the retained-set patient_address
//! projection (most-recent-assertion-per-member) + the patient_address_current per-use
//! display-winner VIEW (recency-first within each use). Real Postgres, gated on
//! `$CAIRN_TEST_PG`, serialized cluster-wide via `db::test_serial_guard`. Matching
//! (§5.2) is a separate subsystem, not exercised here.
use cairn_event::demographics::{address_assertion_body, render_address_twin,
    AddressAssertion, Geo, StructuredAddress};
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use serde_json::json;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

async fn setup(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name, patient_address CASCADE").await.unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    ).await.unwrap();
    (sk, kid)
}

/// Author + sign + submit one demographic.field.asserted event with an explicit HLC.
async fn submit(
    c: &Client, sk: &SigningKey, kid: &str, patient: Uuid, wall: i64, counter: i32,
    payload: serde_json::Value, twin: &str,
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
        plaintext_twin: Some(twin.to_string()),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await
}

/// The current display address for a (patient, use). NULL-safe.
async fn current_for_use(c: &Client, p: Uuid, use_key: &str) -> Option<String> {
    let p_str = p.to_string();
    c.query_opt(
        "SELECT display FROM patient_address_current WHERE patient_id::text=$1 AND use_key=$2",
        &[&p_str, &use_key]).await.unwrap().map(|r| r.get(0))
}

/// Count retained address members for a patient.
async fn addr_count(c: &Client, p: Uuid) -> i64 {
    let p_str = p.to_string();
    c.query_one(
        "SELECT count(*) FROM patient_address WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0)
}

fn addr<'a>(display: &'a str, use_: Option<&'a str>, prov: &'a str) -> AddressAssertion<'a> {
    AddressAssertion { display, provenance: prov, use_, geo: None, structured: None }
}

async fn submit_addr(c: &Client, sk: &SigningKey, kid: &str, p: Uuid, wall: i64, a: &AddressAssertion<'_>)
    -> Result<u64, tokio_postgres::Error> {
    submit(c, sk, kid, p, wall, 0, address_assertion_body(a), &render_address_twin(a)).await
}

#[tokio::test]
async fn happy_path_display_only() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    let a = addr("12 Main St, Springfield", Some("residential"), "patient-stated");
    submit_addr(&c, &sk, &kid, p, 1, &a).await.unwrap();
    assert_eq!(addr_count(&c, p).await, 1);
    assert_eq!(current_for_use(&c, p, "residential").await.as_deref(),
        Some("12 Main St, Springfield"));
}

#[tokio::test]
async fn geo_and_structured_carried_into_retained_set() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    let a = AddressAssertion {
        display: "12 Main St, Springfield", provenance: "patient-stated",
        use_: Some("residential"),
        geo: Some(Geo { lat: -33.87, lon: 151.21, accuracy_m: 10.0, basis: "device_gps" }),
        structured: Some(StructuredAddress {
            profile: "au-address@b3-xyz",
            parts: json!({ "town": "Springfield", "country": "AU" }),
        }),
    };
    submit_addr(&c, &sk, &kid, p, 1, &a).await.unwrap();
    let p_str = p.to_string();
    let (geo, structured): (serde_json::Value, serde_json::Value) = {
        let row = c.query_one(
            "SELECT geo, structured FROM patient_address WHERE patient_id::text=$1", &[&p_str])
            .await.unwrap();
        (row.get(0), row.get(1))
    };
    assert_eq!(geo["basis"], "device_gps");
    assert_eq!(geo["accuracy_m"], 10.0);
    assert_eq!(structured["profile"], "au-address@b3-xyz");
    assert_eq!(structured["parts"]["town"], "Springfield");
}

#[tokio::test]
async fn each_use_is_independently_current() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // A residential AND a postal address are BOTH current — one winner per use.
    submit_addr(&c, &sk, &kid, p, 1, &addr("12 Main St", Some("residential"), "patient-stated")).await.unwrap();
    submit_addr(&c, &sk, &kid, p, 2, &addr("PO Box 9", Some("postal"), "patient-stated")).await.unwrap();
    assert_eq!(current_for_use(&c, p, "residential").await.as_deref(), Some("12 Main St"));
    assert_eq!(current_for_use(&c, p, "postal").await.as_deref(), Some("PO Box 9"));
}

#[tokio::test]
async fn recency_beats_provenance_within_a_use() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // Old, HIGHER-provenance address (document-verified, wall=1).
    submit_addr(&c, &sk, &kid, p, 1, &addr("Old Rd 1", Some("residential"), "document-verified")).await.unwrap();
    // Newer, LOWER-provenance "I moved" (patient-stated, wall=2) — recency wins (volatile field).
    submit_addr(&c, &sk, &kid, p, 2, &addr("New Rd 2", Some("residential"), "patient-stated")).await.unwrap();
    assert_eq!(current_for_use(&c, p, "residential").await.as_deref(), Some("New Rd 2"),
        "newer address wins display (recency beats provenance for a volatile field)");
    assert_eq!(addr_count(&c, p).await, 2, "the old address is retained, not overwritten");
}

#[tokio::test]
async fn set_union_reassertion_is_idempotent_and_order_independent() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // Apply newer first, then older — winner must still be the newer (apply-order-independent),
    // and the same (use, display) re-asserted stays ONE member.
    submit_addr(&c, &sk, &kid, p, 2, &addr("New Rd 2", Some("residential"), "patient-stated")).await.unwrap();
    submit_addr(&c, &sk, &kid, p, 1, &addr("Old Rd 1", Some("residential"), "patient-stated")).await.unwrap();
    submit_addr(&c, &sk, &kid, p, 3, &addr("New Rd 2", Some("residential"), "patient-stated")).await.unwrap();
    assert_eq!(addr_count(&c, p).await, 2, "two distinct members; the re-assertion deduped");
    assert_eq!(current_for_use(&c, p, "residential").await.as_deref(), Some("New Rd 2"));
}

#[tokio::test]
async fn floor_rejects_structured_without_profile() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // structured present without a profile violates the §4.3 structural invariant.
    let r = submit(&c, &sk, &kid, p, 1, 0,
        json!({"field":"address","value":"X","provenance":"patient-stated",
               "facets":{"structured":{"parts":{"town":"Springfield"}}}}),
        "Address (patient-stated): X").await;
    assert!(r.is_err(), "structured-without-profile is rejected by the floor");
    let p_str = p.to_string();
    let n: i64 = c.query_one("SELECT count(*) FROM event_log WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0);
    assert_eq!(n, 0, "rejected event is not appended");
    assert_eq!(addr_count(&c, p).await, 0, "rejected event is not projected");
}

#[tokio::test]
async fn floor_rejects_non_text_structured_part() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // A non-text parts value (number) violates "parts values are opaque TEXT to the core".
    let r = submit(&c, &sk, &kid, p, 1, 0,
        json!({"field":"address","value":"X","provenance":"patient-stated",
               "facets":{"structured":{"profile":"au@b3","parts":{"postcode":2000}}}}),
        "Address (patient-stated): X").await;
    assert!(r.is_err(), "a non-text parts value is rejected by the floor");
    assert_eq!(addr_count(&c, p).await, 0);
}

#[tokio::test]
async fn floor_rejects_malformed_geo() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // Each malformed geo is its own isolated rejection: non-number lat, negative
    // accuracy_m, empty basis. None may append or project.
    for facets in [
        json!({"geo":{"lat":"north","lon":151.2,"accuracy_m":10.0,"basis":"device_gps"}}),
        json!({"geo":{"lat":-33.8,"lon":151.2,"accuracy_m":-5.0,"basis":"device_gps"}}),
        json!({"geo":{"lat":-33.8,"lon":151.2,"accuracy_m":10.0,"basis":""}}),
    ] {
        let r = submit(&c, &sk, &kid, p, 1, 0,
            json!({"field":"address","value":"X","provenance":"declared","facets":facets}),
            "Address (declared): X").await;
        assert!(r.is_err(), "malformed geo must be rejected");
    }
    let p_str = p.to_string();
    let n: i64 = c.query_one("SELECT count(*) FROM event_log WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0);
    assert_eq!(n, 0, "no malformed-geo event appended");
    assert_eq!(addr_count(&c, p).await, 0);
}

#[tokio::test]
async fn unknown_field_and_legacy_unaffected() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // A name event must NOT create a patient_address row; an address event must NOT
    // create a patient_name row. The projections are blind to each other's fields.
    submit(&c, &sk, &kid, p, 1, 0,
        json!({"field":"name","value":"Mary Jones","provenance":"patient-stated","facets":{"use":"legal"}}),
        "Name (legal): Mary Jones").await.unwrap();
    submit_addr(&c, &sk, &kid, p, 2, &addr("12 Main St", Some("residential"), "patient-stated")).await.unwrap();

    let p_str = p.to_string();
    let names: i64 = c.query_one("SELECT count(*) FROM patient_name WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0);
    assert_eq!(names, 1, "name still projects only into patient_name");
    assert_eq!(addr_count(&c, p).await, 1, "address projects only into patient_address");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test demographics_address`
Expected: FAIL — `patient_address` does not exist (TRUNCATE / schema load errors), because `db/014` isn't written or registered yet.

- [ ] **Step 3: Write the `db/014` migration**

Create `db/014_demographics_address.sql`:

```sql
-- Cairn — demographic ADDRESS: the §4.3 three-facet value (display/geo/structured).
--
-- Slice 5 of the demographics subsystem. An address reuses the slice-2 generic
-- `demographic.field.asserted` event with field='address'; `value` carries the
-- mandatory `display` string (the value-core), so the generic floor's non-empty-value
-- check already enforces "display non-empty". This migration adds NO new event type:
-- it (1) extends the shared structural floor with an address branch (structured⇒profile,
-- parts are text, geo shape) — culture-neutral, never holds a profile, never rejects on
-- validation; and (2) adds a retained-set table + a per-use display-winner VIEW
-- (recency-first within each use — addresses are volatile, so a fresh "I moved" must
-- beat a stale verified address, mirroring names/ADR-0036, NOT DOB's provenance-lock).
-- Matching (§5.2) is a separate, later subsystem.

BEGIN;

-- Extend the shared §4.2/§4.3 structural floor with the address branch. CREATE OR REPLACE
-- supersedes db/011's definition (latest-loaded wins — db/013 left this function
-- untouched), so the dob branch is carried forward VERBATIM and the generic checks
-- (payload/field/provenance/value all present and non-empty) are unchanged. The address
-- branch enforces ONLY culture-neutral structural shape: it never interprets a part name,
-- never holds a profile, never validates geo semantics (lat/lon bounds are advisory).
CREATE OR REPLACE FUNCTION cairn_check_demographic_field(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p     jsonb := b -> 'payload';
    fld   text;
    geo   jsonb;
    part  record;
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'demographic field assertion: missing payload';
    END IF;
    IF jsonb_typeof(p -> 'field') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'field')) = 0 THEN
        RAISE EXCEPTION 'demographic field assertion: field must be a non-empty string';
    END IF;
    IF jsonb_typeof(p -> 'provenance') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'provenance')) = 0 THEN
        RAISE EXCEPTION 'demographic field assertion: provenance must be a non-empty string (§4.1)';
    END IF;
    -- value: the core scalar (§4.2/§4.3). For an address this IS the mandatory `display`.
    IF jsonb_typeof(p -> 'value') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'value')) = 0 THEN
        RAISE EXCEPTION 'demographic field assertion: value must be a non-empty string';
    END IF;

    fld := p ->> 'field';
    -- dob (carried forward from db/011, unchanged): precision mandatory; basis text when present.
    IF fld = 'dob' THEN
        IF jsonb_typeof(p -> 'facets' -> 'precision') IS DISTINCT FROM 'string'
           OR length(trim(p -> 'facets' ->> 'precision')) = 0 THEN
            RAISE EXCEPTION 'demographic field assertion: dob requires a non-empty facets.precision (principle 4)';
        END IF;
        IF (p -> 'facets' ? 'basis') AND (p -> 'facets' -> 'basis') IS DISTINCT FROM 'null'::jsonb THEN
            IF jsonb_typeof(p -> 'facets' -> 'basis') IS DISTINCT FROM 'string'
               OR length(trim(p -> 'facets' ->> 'basis')) = 0 THEN
                RAISE EXCEPTION 'demographic field assertion: dob facets.basis must be non-empty text when present';
            END IF;
        END IF;
    -- address (§4.3): structured ⇒ profile present + parts are text; geo shape when present.
    ELSIF fld = 'address' THEN
        -- structured: when present, profile is a non-empty string and every part value is text.
        IF (p -> 'facets' ? 'structured')
           AND (p -> 'facets' -> 'structured') IS DISTINCT FROM 'null'::jsonb THEN
            IF jsonb_typeof(p -> 'facets' -> 'structured' -> 'profile') IS DISTINCT FROM 'string'
               OR length(trim(p -> 'facets' -> 'structured' ->> 'profile')) = 0 THEN
                RAISE EXCEPTION 'demographic field assertion: address structured requires a non-empty profile (§4.3)';
            END IF;
            IF (p -> 'facets' -> 'structured' ? 'parts')
               AND (p -> 'facets' -> 'structured' -> 'parts') IS DISTINCT FROM 'null'::jsonb THEN
                IF jsonb_typeof(p -> 'facets' -> 'structured' -> 'parts') IS DISTINCT FROM 'object' THEN
                    RAISE EXCEPTION 'demographic field assertion: address structured.parts must be an object';
                END IF;
                FOR part IN
                    SELECT value AS v
                    FROM jsonb_each(p -> 'facets' -> 'structured' -> 'parts')
                LOOP
                    IF jsonb_typeof(part.v) IS DISTINCT FROM 'string' THEN
                        RAISE EXCEPTION 'demographic field assertion: address structured.parts values must be text (opaque to the core)';
                    END IF;
                END LOOP;
            END IF;
        END IF;
        -- geo: when present, lat/lon are numbers, accuracy_m a non-negative number, basis non-empty text.
        IF (p -> 'facets' ? 'geo') AND (p -> 'facets' -> 'geo') IS DISTINCT FROM 'null'::jsonb THEN
            geo := p -> 'facets' -> 'geo';
            IF jsonb_typeof(geo -> 'lat') IS DISTINCT FROM 'number'
               OR jsonb_typeof(geo -> 'lon') IS DISTINCT FROM 'number' THEN
                RAISE EXCEPTION 'demographic field assertion: address geo.lat/geo.lon must be numbers';
            END IF;
            IF jsonb_typeof(geo -> 'accuracy_m') IS DISTINCT FROM 'number'
               OR (geo ->> 'accuracy_m')::numeric < 0 THEN
                RAISE EXCEPTION 'demographic field assertion: address geo.accuracy_m must be a non-negative number';
            END IF;
            IF jsonb_typeof(geo -> 'basis') IS DISTINCT FROM 'string'
               OR length(trim(geo ->> 'basis')) = 0 THEN
                RAISE EXCEPTION 'demographic field assertion: address geo.basis must be non-empty text';
            END IF;
        END IF;
    END IF;
    -- unknown field: generic checks only — carried, legible, not projected.
END;
$$;

-- The §4.3 retained set: one row per distinct (patient, use, display) address. use_key
-- folds an absent/blank `use` to 'unspecified' and ASCII-lower-cases it (COLLATE "C")
-- exactly as patient_name does — `use` is an OPEN vocabulary, so "Residential"/"residential"
-- are one category, folded deterministically so the per-use winner and member dedup stay
-- convergent across the fleet (a locale lower() is collation-dependent). display is the
-- member discriminant (the value-core); geo/structured travel as the member's representative
-- facets, the most-recent assertion winning. provenance_rank is cached (reuses db/011's
-- cairn_provenance_rank) so the recency/provenance test is a plain tuple compare.
CREATE TABLE IF NOT EXISTS patient_address (
    patient_id         UUID    NOT NULL,
    use_key            TEXT    NOT NULL,   -- lower(coalesce(NULLIF(trim(use),''),'unspecified') COLLATE "C")
    display            TEXT    NOT NULL,   -- the mandatory human-readable address (value-core)
    use_raw            TEXT,               -- the original `use` facet (NULL when absent)
    geo                JSONB,              -- optional precision-aware geolocation facet
    structured         JSONB,              -- optional {profile, parts} facet
    provenance         TEXT    NOT NULL,
    provenance_rank    INT     NOT NULL,
    last_hlc_wall      BIGINT  NOT NULL,
    last_hlc_count     INTEGER NOT NULL,
    asserted_origin    TEXT    NOT NULL,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (patient_id, use_key, display)
);

-- Incremental maintenance: fold exactly the one new address event into the retained set.
-- event_log.body holds b->'payload' (see db/005 submit_event INSERT).
CREATE OR REPLACE FUNCTION patient_address_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p      jsonb := NEW.body;
    fld    text  := p ->> 'field';
    v_use  text  := NULLIF(trim(p -> 'facets' ->> 'use'), '');
    v_key  text;
    v_rank int;
BEGIN
    -- Only ADDRESS events project here. dob/sex-at-birth (db/011/013), name (db/012), and
    -- any unknown field are ignored — each projection gates to its own fields and writes a
    -- different table, so the several triggers on demographic.field.asserted are order-free.
    IF fld <> 'address' THEN
        RETURN NULL;
    END IF;
    v_key  := lower(coalesce(v_use, 'unspecified') COLLATE "C");
    v_rank := cairn_provenance_rank(p ->> 'provenance');

    INSERT INTO patient_address AS pa
        (patient_id, use_key, display, use_raw, geo, structured,
         provenance, provenance_rank, last_hlc_wall, last_hlc_count, asserted_origin)
    VALUES
        (NEW.patient_id, v_key, p ->> 'value', v_use,
         p -> 'facets' -> 'geo', p -> 'facets' -> 'structured',
         p ->> 'provenance', v_rank, NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
    -- Per (patient, use, display) member, keep the MOST-RECENT assertion as its
    -- representative (recency-first tuple — matches the display rule). The compare is a
    -- deterministic, apply-order-independent function of the member's assertion set, so
    -- every node converges. A re-assertion that does not advance the tuple is a no-op
    -- (set-union idempotency).
    ON CONFLICT (patient_id, use_key, display) DO UPDATE SET
        use_raw         = EXCLUDED.use_raw,
        geo             = EXCLUDED.geo,
        structured      = EXCLUDED.structured,
        provenance      = EXCLUDED.provenance,
        provenance_rank = EXCLUDED.provenance_rank,
        last_hlc_wall   = EXCLUDED.last_hlc_wall,
        last_hlc_count  = EXCLUDED.last_hlc_count,
        asserted_origin = EXCLUDED.asserted_origin,
        updated_at      = clock_timestamp()
    WHERE (EXCLUDED.last_hlc_wall, EXCLUDED.last_hlc_count,
           EXCLUDED.provenance_rank, EXCLUDED.asserted_origin)
        > (pa.last_hlc_wall, pa.last_hlc_count,
           pa.provenance_rank, pa.asserted_origin);
    RETURN NULL;
END;
$$;

DROP TRIGGER IF EXISTS patient_address_apply_trg ON event_log;
CREATE TRIGGER patient_address_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'demographic.field.asserted')
    EXECUTE FUNCTION patient_address_apply();

-- The §4.3 per-use display-winner: ONE row per (patient, use), selected from the retained
-- set with NO stored pointer. The ORDER BY is the whole rule: recency-first within the use
-- (newest address wins — recency beats provenance for a volatile field, the deliberate
-- divergence from DOB's provenance-lock), with provenance_rank then asserted_origin as
-- deterministic tiebreaks so every node converges to the same current address per use.
CREATE OR REPLACE VIEW patient_address_current AS
SELECT DISTINCT ON (patient_id, use_key)
    patient_id, use_key, display, use_raw, geo, structured,
    provenance, provenance_rank, last_hlc_wall, last_hlc_count, asserted_origin, updated_at
FROM patient_address
ORDER BY patient_id, use_key,
         last_hlc_wall DESC, last_hlc_count DESC,
         provenance_rank DESC, asserted_origin DESC;

GRANT SELECT ON patient_address, patient_address_current TO cairn_agent;

COMMIT;
```

- [ ] **Step 4: Register migration 014 in the schema array**

In `crates/cairn-node/src/db.rs`, add the entry after the `013_demographics_sex_gender` line in the `SCHEMA` array:

```rust
    ("013_demographics_sex_gender", include_str!("../../../db/013_demographics_sex_gender.sql")),
    ("014_demographics_address", include_str!("../../../db/014_demographics_address.sql")),
```

- [ ] **Step 5: Run the integration tests to verify they pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test demographics_address`
Expected: PASS (9 tests). If `$CAIRN_TEST_PG` is unset they self-skip with a printed notice — set it to actually exercise them.

- [ ] **Step 6: Full regression + clippy**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node` (slices 1–4 stay green) and `cargo clippy -p cairn-node --all-targets -- -D warnings`.
Expected: all green, clippy clean.

- [ ] **Step 7: Commit**

```bash
git add db/014_demographics_address.sql crates/cairn-node/src/db.rs crates/cairn-node/tests/demographics_address.rs
git commit -m "feat(db): §4.3 address — floor branch + per-use recency-first projection

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: ADR-0038 + spec §4.3 + currency

**Files:**
- Create: `docs/spec/decisions/0038-demographic-address-winner-per-use-recency.md`
- Modify: `docs/spec/demographics.md` (§4.3 prose note + the line-20 summary-table cell)
- Modify: `docs/spec/index.md` (spec version 0.38 → 0.39; ADR index row if one is listed there)
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md` (currency: slice 5 landed; "next" list updated)

**Interfaces:**
- Consumes: nothing in code; this task is docs-only. Follows the ADR template of the sibling ADR-0036.
- Produces: the canonical *why* for the per-use recency-first address winner.

- [ ] **Step 1: Write ADR-0038**

Create `docs/spec/decisions/0038-demographic-address-winner-per-use-recency.md`:

```markdown
# ADR-0038 — Demographic address display: per-use recency-first

- **Status:** Accepted
- **Date:** 2026-06-28
- **Refines:** [ADR-0032](0032-culture-neutral-address-representation.md) (representation),
  follows [ADR-0036](0036-demographic-name-display-recency-first.md) (volatile-field logic)

## Context

[ADR-0032](0032-culture-neutral-address-representation.md) fixed the address *representation*
(the three-facet value: mandatory `display`, optional `geo`, optional `structured`) but
deliberately left the *projection* — which assertion is the "current" address — open, calling
the thin "recency wins" treatment a matching statement, not a projection one. The §4.3 summary
table, written the same day, said the per-use current address was *"highest-provenance
most-recent"* — provenance-first, the DOB lock. That predates the names slice
([ADR-0036](0036-demographic-name-display-recency-first.md)), which established that a
**volatile, legitimately-changing field must be recency-first**, or a stale verified value pins
over the current truth (the deadname / stale-married-name failure).

## Decision

The per-`use` current address is **recency-first**: within a `use`, the newest assertion wins
(HLC wall then counter), with `provenance_rank` then `asserted_origin` as deterministic
tiebreaks. Address is the archetypal volatile field — people move — so a fresh patient-stated
"I moved last month" must displace a stale document-verified address; that is *where you would
send an ambulance or a letter*. This is the same reasoning ADR-0036 applied to names, and the
deliberate inverse of DOB's provenance-lock ([ADR-0037](0037-demographic-administrative-sex-and-per-field-winner-policy.md)).

The projection is a **retained set** (`patient_address`, keyed `(patient, use, display)`) plus a
per-use display-winner VIEW (`patient_address_current`, one row per `(patient, use)`) — **one
current address per use**; residential, postal, and work are independently current. There is no
legal-tier preference (unlike names) and no cross-use fallback; the UI surfaces past or other-use
addresses from the retained set. All addresses are retained as evidence regardless of which one
displays; provenance still feeds the later [§5.2](../identity.md) matcher.

## Consequences

- **Easier:** the current address always reflects the latest claim about where the patient is;
  address history is intact (append-only — "moved out" is a new assertion, never an overwrite);
  the projection reuses the names machinery verbatim (no new event type, additive floor branch).
- **The bet / trade:** a recency-first winner trusts the newest assertion even when lower
  provenance, so a mistaken or malicious fresh assertion can transiently displace a good one — but
  it is **overlay, never erasure** (the displaced address is retained, attributable, and re-assertable),
  and the matcher reads the whole set, not just the winner.
- **Explicitly out of scope (deferred):** an explicit address supersession/unlink event (the
  append-only set + recency covers "moved"); the §5.2 comparator using the address `profile`;
  advisory validators (lat/lon bounds, `display == formatter(parts)` drift, profile re-derivation).
```

- [ ] **Step 2: Fix the §4.3 summary-table cell and add the winner-policy prose**

In `docs/spec/demographics.md`, the §4 summary-table Address row (around line 20) currently reads
`Per use: displayed current = highest-provenance most-recent non-superseded assertion`. Replace
that cell text with:

```
Per `use`: displayed current = **most-recent** assertion within the use (recency-first — addresses are volatile; provenance/origin break ties; [ADR-0038](decisions/0038-demographic-address-winner-per-use-recency.md)); full history retained; supersession is an explicit assertion, never an overwrite
```

Then, at the end of the §4.3 section (after the "Honest degradation" paragraph), add:

```markdown
**Display-winner: per-use recency-first.** A patient holds one *current* address per `use`
(residential, postal, work are independently current). Within a use the **newest** assertion wins
(recency-first — addresses are volatile; a fresh patient-stated move must displace a stale
document-verified address, the same reasoning names follow and the inverse of DOB's
provenance-lock), with provenance then origin as deterministic tiebreaks. All addresses are
retained as matching evidence; the UI surfaces past/other-use addresses from the retained set
([ADR-0038](decisions/0038-demographic-address-winner-per-use-recency.md)).
```

- [ ] **Step 3: Bump the spec version and ADR index**

In `docs/spec/index.md`, change the spec version `0.38` → `0.39`. If `index.md` carries an ADR
index table, add a row:

```
| [0038](decisions/0038-demographic-address-winner-per-use-recency.md) | Demographic address display: per-use recency-first (volatile field; follows ADR-0036) | §4.3 (refines 0032) |
```

(Search `index.md` for the `0037` row to find the table; match its exact column format. If no ADR
table exists in `index.md`, skip the row — the version bump is the required change.)

- [ ] **Step 4: Update HANDOVER.md and ROADMAP.md currency**

In `docs/HANDOVER.md`: add a top "This session" paragraph summarising slice 5 (§4.3 address:
per-use recency-first; db/014; cairn-event address builders; ADR-0038; spec 0.38→0.39); move the
§4.3 address item out of the "Next" list in the open-threads menu (the remaining next items are the
§5.2 matcher and globalising the authored twin); add the ADR-0038 row to the decision-trail table.

In `docs/ROADMAP.md`: in the Phase 4 demographics paragraph, append a **Slice 5 — §4.3 address**
sentence mirroring the slice-3/4 sentences (retained-set `patient_address` + per-use
`patient_address_current` recency-first VIEW; additive floor branch; ADR-0038), and update the
**Next** list to drop address.

Keep both files concise (prune older detail if either approaches 500 lines).

- [ ] **Step 5: Build the docs to confirm they render**

Run: `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build`
Expected: builds with no broken-link errors for the new ADR-0038 cross-references.

- [ ] **Step 6: Commit**

```bash
git add docs/spec/decisions/0038-demographic-address-winner-per-use-recency.md docs/spec/demographics.md docs/spec/index.md docs/HANDOVER.md docs/ROADMAP.md
git commit -m "spec(demographics): §4.3 address winner — per-use recency-first (ADR-0038)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Final verification (before PR)

- [ ] `cargo fmt --all` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `CAIRN_TEST_PG=… cargo test --workspace` — all green (new address suite + slices 1–4 regress).
- [ ] `mkdocs build` clean.
- [ ] HANDOVER.md + ROADMAP.md reflect slice 5 as landed; "next" lists no longer include §4.3 address.
- [ ] Open the PR on `demographics-address` → `main` with a summary of the slice and the ADR-0038 rationale.
```
