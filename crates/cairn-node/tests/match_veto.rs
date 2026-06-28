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
#[allow(clippy::too_many_arguments)]
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
/// Uses `$1::text::uuid` rather than `$1::uuid` so that tokio-postgres (no uuid
/// feature) infers the param type as `text`, avoiding a ToSql WrongType error.
async fn veto_rows(c: &Client, a: Uuid, b: Uuid) -> Vec<(String, String, String)> {
    let a_s = a.to_string();
    let b_s = b.to_string();
    let rows = c.query(
        "SELECT veto_kind, severity, subject FROM cairn_match_veto($1::text::uuid, $2::text::uuid) \
         ORDER BY veto_kind, subject",
        &[&a_s, &b_s]).await.unwrap();
    rows.iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect()
}

async fn has_hard_veto(c: &Client, a: Uuid, b: Uuid) -> bool {
    let a_s = a.to_string();
    let b_s = b.to_string();
    c.query_one("SELECT cairn_has_hard_veto($1::text::uuid, $2::text::uuid)", &[&a_s, &b_s])
        .await.unwrap().get(0)
}

/// Build an IdentifierAssertion borrowing the given strings (helper for readability).
/// When `normalized` is `Some`, the §4.4 DB floor requires a non-empty `profile` name
/// (the materialised-key rule: a normalized form must declare the comparator bundle
/// that produced it). A stub profile tag is supplied automatically for test fixtures
/// that don't need a real namespace@hash — sufficient to satisfy the floor without
/// wiring up a real comparator distribution.
fn idassert<'a>(system: &'a str, value: &'a str, normalized: Option<&'a str>) -> IdentifierAssertion<'a> {
    IdentifierAssertion {
        value, system, provenance: "patient-stated",
        normalized,
        profile: if normalized.is_some() { Some("test-profile@stub") } else { None },
        use_: None,
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
async fn identifier_degrade_hold_when_one_side_normalized_other_not() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    // A carries a normalized form (profile present); B is profile-less (normalized absent).
    // The two sides cannot be trustworthily compared (B's difference may be pure formatting
    // noise), so a same-system mismatch must DEGRADE to human review — never escalate to a
    // trustworthy hard veto. (both_have_norm is false because B lacks a normalized form.)
    submit_identifier(&c, &sk, &kid, a, 1, &idassert("local-mrn", "00123", Some("00123"))).await;
    submit_identifier(&c, &sk, &kid, b, 2, &idassert("local-mrn", "00999", None)).await;
    let rows = veto_rows(&c, a, b).await;
    assert_eq!(rows, vec![("identifier".into(), "degrade_hold".into(), "local-mrn".into())]);
    assert!(!has_hard_veto(&c, a, b).await, "asymmetric-normalized mismatch must not be a hard veto");
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

/// Submit one §4.2 DOB assertion.
#[allow(clippy::too_many_arguments)]
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
