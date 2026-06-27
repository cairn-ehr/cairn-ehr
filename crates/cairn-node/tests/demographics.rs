//! Integration coverage for the §4.4 demographic identifier assertion: the in-DB
//! floor + the set-union patient_identifier projection. Real Postgres, gated on
//! `$CAIRN_TEST_PG`, serialized cluster-wide via `db::test_serial_guard` (the
//! shared-DB + TRUNCATE pattern, identical to `attestation.rs`). Matching/veto is
//! a separate subsystem and is NOT exercised here.
use cairn_event::demographics::{identifier_assertion_body, render_identifier_twin, IdentifierAssertion};
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

/// Truncate the clinical tables and enroll one agent signer. Returns (sk, kid).
async fn setup(c: &Client) -> (SigningKey, String) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart, patient_identifier CASCADE")
        .await.unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    ).await.unwrap();
    (sk, kid)
}

/// Author + sign + submit one §4.4 identifier assertion for `patient`. Returns the
/// raw submit result so rejection tests (Task 4) can assert the error.
async fn assert_identifier(
    c: &Client, sk: &SigningKey, kid: &str, patient: Uuid, wall: i64,
    a: &IdentifierAssertion<'_>,
) -> Result<u64, tokio_postgres::Error> {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "demographic.identifier.asserted".into(),
        schema_version: "demographic.identifier/1".into(),
        hlc: Hlc { wall, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: identifier_assertion_body(a),
        attachments: vec![],
        plaintext_twin: Some(render_identifier_twin(a)),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await
}

#[tokio::test]
async fn happy_path_appends_and_projects_with_authored_twin() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();
    let a = IdentifierAssertion {
        value: "943 476 5919", system: "nhs-number", provenance: "document-verified",
        normalized: Some("9434765919"), profile: Some("nhs-number@b3-abc"),
        use_: Some("national-id"),
    };
    assert_identifier(&c, &sk, &kid, p, 1, &a).await.expect("valid assertion accepted");

    // Projection: one row, keyed on the normalized match_key. Compare UUID columns
    // as text (cast the column) since tokio-postgres has no uuid ToSql in this project.
    // Two single-table queries (not a join): the projection columns live on
    // patient_identifier, the authored twin on event_log — joining them would fan out
    // once a patient carries more than one identifier event.
    let p_str = p.to_string();
    let row = c.query_one(
        "SELECT match_key, value FROM patient_identifier WHERE patient_id::text = $1",
        &[&p_str]).await.unwrap();
    let match_key: String = row.get(0);
    let value: String = row.get(1);
    assert_eq!(match_key, "9434765919");
    assert_eq!(value, "943 476 5919");
    // The AUTHORED twin is stored verbatim (proving cairn_body passed the top-level
    // plaintext_twin field through to submit_event, which carried it for a demographic event).
    let twin: String = c.query_one(
        "SELECT plaintext_twin FROM event_log WHERE patient_id::text = $1", &[&p_str])
        .await.unwrap().get(0);
    assert_eq!(twin, "nhs-number, document-verified: 943 476 5919");
}

#[tokio::test]
async fn set_union_dedups_same_key_keeps_different_key() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();
    let same = IdentifierAssertion {
        value: "943 476 5919", system: "nhs-number", provenance: "patient-stated",
        normalized: Some("9434765919"), profile: Some("nhs-number@b3-abc"), use_: None,
    };
    let same_formatted = IdentifierAssertion { value: "9434765919", ..same_clone(&same) };
    let other = IdentifierAssertion {
        value: "111 222 3334", system: "nhs-number", provenance: "patient-stated",
        normalized: Some("1112223334"), profile: Some("nhs-number@b3-abc"), use_: None,
    };
    assert_identifier(&c, &sk, &kid, p, 1, &same).await.unwrap();
    assert_identifier(&c, &sk, &kid, p, 2, &same_formatted).await.unwrap(); // same normalized → dedup
    assert_identifier(&c, &sk, &kid, p, 3, &other).await.unwrap();          // different normalized → 2nd row
    let p_str = p.to_string();
    let n: i64 = c.query_one(
        "SELECT count(*) FROM patient_identifier WHERE patient_id::text=$1 AND system='nhs-number'",
        &[&p_str]).await.unwrap().get(0);
    assert_eq!(n, 2, "same-normalized dedups; different-normalized keeps both");
}

// Helper: clone an IdentifierAssertion's borrowed fields (test-only convenience).
fn same_clone<'a>(a: &IdentifierAssertion<'a>) -> IdentifierAssertion<'a> {
    IdentifierAssertion {
        value: a.value, system: a.system, provenance: a.provenance,
        normalized: a.normalized, profile: a.profile, use_: a.use_,
    }
}

#[tokio::test]
async fn honest_degradation_no_normalized_no_profile_accepted() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();
    let a = IdentifierAssertion {
        value: "OLD-CARD-77", system: "unknown", provenance: "imported",
        normalized: None, profile: None, use_: None,
    };
    assert_identifier(&c, &sk, &kid, p, 1, &a).await.expect("profile-less assertion accepted");
    let p_str = p.to_string();
    let mk: String = c.query_one(
        "SELECT match_key FROM patient_identifier WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0);
    assert_eq!(mk, "OLD-CARD-77", "match_key falls back to value when normalized absent");
}

// ── Task 4: floor rejection tests + legacy derived-twin regression ────────────

/// Submit a raw body (bypassing the typed builder) so we can author floor-violating
/// payloads the safe builder would never produce.  Returns the submit result so the
/// caller can assert the error and inspect side-effects.
async fn submit_raw_demographic(
    c: &Client, sk: &SigningKey, kid: &str, patient: Uuid,
    payload: serde_json::Value, twin: Option<&str>,
) -> Result<u64, tokio_postgres::Error> {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "demographic.identifier.asserted".into(),
        schema_version: "demographic.identifier/1".into(),
        hlc: Hlc { wall: 1, counter: 0, node_origin: "n".into() },
        t_effective: None, signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload, attachments: vec![],
        plaintext_twin: twin.map(|t| t.to_string()),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await
}

/// Assert that the floor REJECTS the payload (submit errors) AND that nothing was
/// written — neither to `event_log` nor to the `patient_identifier` projection.
/// Uses `patient_id::text = $1` (string param) matching the project's tokio-postgres
/// convention (no uuid ToSql feature in this project).
async fn assert_rejected_and_empty(
    c: &Client, sk: &SigningKey, kid: &str, p: Uuid,
    payload: serde_json::Value, twin: Option<&str>, label: &str,
) {
    let r = submit_raw_demographic(c, sk, kid, p, payload, twin).await;
    assert!(r.is_err(), "{label}: must be rejected by the floor");
    let p_str = p.to_string();
    let n: i64 = c.query_one(
        "SELECT count(*) FROM event_log WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0);
    assert_eq!(n, 0, "{label}: nothing appended to event_log");
    let m: i64 = c.query_one(
        "SELECT count(*) FROM patient_identifier WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0);
    assert_eq!(m, 0, "{label}: nothing projected");
}

/// Adversarial gate: prove the in-DB floor rejects every structural violation that
/// the safe typed builder (`identifier_assertion_body`) would never produce.  Each
/// sub-case gets its own fresh UUID so failures are independent and the TRUNCATE /
/// serial-guard scope covers all of them.
#[tokio::test]
async fn floor_rejects_each_invariant_violation() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    // A valid authored twin used for cases where only the payload is malformed.
    let good_twin = Some("nhs-number, document-verified: x");

    // value empty — §4.4 mandatory, non-empty string
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"identifier","value":"","system":"nhs-number","provenance":"x"}),
        good_twin, "value-empty").await;
    // system missing — §4.4 mandatory
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"identifier","value":"v","provenance":"x"}),
        good_twin, "system-missing").await;
    // provenance missing — §4.1 ladder mandatory
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"identifier","value":"v","system":"nhs-number"}),
        good_twin, "provenance-missing").await;
    // normalized non-text — §4.4 requires string when present
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"identifier","value":"v","system":"s","provenance":"x","normalized":123,"profile":"p@h"}),
        good_twin, "normalized-non-text").await;
    // normalized without profile — §4.4 materialised-key rule: profile names the bundle
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"identifier","value":"v","system":"s","provenance":"x","normalized":"vv"}),
        good_twin, "normalized-without-profile").await;
    // normalized whitespace-only — §4.4: a materialised key must be a real non-empty
    // string. Without this the projection's match_key = coalesce(norm, value) becomes
    // pure whitespace, silently conflating two distinct identifiers under one PK row.
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"identifier","value":"v","system":"s","provenance":"x","normalized":"   ","profile":"p@h"}),
        good_twin, "normalized-whitespace").await;
    // empty authored twin — §4.5: demographic assertions must carry a non-empty twin
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"identifier","value":"v","system":"s","provenance":"x"}),
        Some(""), "empty-twin").await;
}

/// Regression guard: a legacy event type with NO authored twin must still be accepted
/// by the updated submit_event and receive the derived skeleton twin (not a panic or
/// a rejection).  Proves the §4.5 demographic branch does not break ordinary events.
#[tokio::test]
async fn legacy_patient_created_still_uses_derived_twin() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();
    // A legacy additive event with NO authored twin must still be accepted and get
    // the derived skeleton twin (the demographics-only twin scope, regression guard).
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(), patient_id: p.to_string(),
        event_type: "patient.created".into(), schema_version: "demo/1".into(),
        hlc: Hlc { wall: 1, counter: 0, node_origin: "n".into() }, t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"name":"A B","dob":"1980","sex":"x"}),
        attachments: vec![], plaintext_twin: None,
    };
    let signed = sign(&body, &sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await
        .expect("legacy event with no authored twin still accepted");
    // Retrieve the derived twin using event_id::text cast (no uuid ToSql in this project).
    let twin: String = c.query_one(
        "SELECT plaintext_twin FROM event_log WHERE event_id::text=$1",
        &[&body.event_id]).await.unwrap().get(0);
    assert!(twin.starts_with("[patient.created]"), "legacy derives the skeleton twin");
}
