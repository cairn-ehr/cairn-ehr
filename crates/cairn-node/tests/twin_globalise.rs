//! Integration coverage for ADR-0039 — the globalised authored legibility twin. Real Postgres,
//! gated on `$CAIRN_TEST_PG`, serialized cluster-wide via `db::test_serial_guard`. Proves: an
//! authored twin on a non-demographic event passes through verbatim and reads back as authored;
//! a twin-less event degrades honestly to a flagged, payload-rendering derived skeleton (set-union
//! preserved); a twin-less demographic event is still HARD-rejected (ADR-0034).
use cairn_event::demographics::{identifier_assertion_body, IdentifierAssertion};
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

/// Author + sign + submit one note.added for `patient`, optionally carrying an authored twin.
async fn submit_note(
    c: &Client, sk: &SigningKey, kid: &str, patient: Uuid, wall: i64, twin: Option<&str>,
) -> Result<u64, tokio_postgres::Error> {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: Hlc { wall, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "BP 120/80, afebrile"}),
        attachments: vec![],
        plaintext_twin: twin.map(|t| t.to_string()),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await
}

#[tokio::test]
async fn authored_twin_on_note_passes_through_verbatim_and_reads_authored() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();
    submit_note(&c, &sk, &kid, p, 1, Some("Progress note: BP 120/80, afebrile"))
        .await.expect("authored-twin note accepted");
    let p_str = p.to_string();
    let twin: String = c.query_one(
        "SELECT plaintext_twin FROM event_log WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0);
    assert_eq!(twin, "Progress note: BP 120/80, afebrile", "authored twin carried verbatim");
    let authored: bool = c.query_one(
        "SELECT cairn_twin_is_authored(signed_bytes) FROM event_log WHERE patient_id::text=$1",
        &[&p_str]).await.unwrap().get(0);
    assert!(authored, "an authored twin reads back as authored");
    // The provenance view agrees.
    let view_authored: bool = c.query_one(
        "SELECT ep.twin_authored FROM event_twin_provenance ep
           JOIN event_log el USING (event_id) WHERE el.patient_id::text=$1",
        &[&p_str]).await.unwrap().get(0);
    assert!(view_authored, "event_twin_provenance flags it authored");
}

#[tokio::test]
async fn twinless_note_degrades_to_flagged_skeleton() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();
    submit_note(&c, &sk, &kid, p, 1, None)
        .await.expect("twin-less note still accepted (set-union preserved)");
    let p_str = p.to_string();
    let twin: String = c.query_one(
        "SELECT plaintext_twin FROM event_log WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0);
    assert!(twin.starts_with("[note.added]"), "derived skeleton twin, got: {twin}");
    assert!(twin.contains("BP 120/80"), "skeleton renders the payload (db/005 TODO closed)");
    let authored: bool = c.query_one(
        "SELECT cairn_twin_is_authored(signed_bytes) FROM event_log WHERE patient_id::text=$1",
        &[&p_str]).await.unwrap().get(0);
    assert!(!authored, "a derived twin reads back as NOT authored (honest flag)");
}

#[tokio::test]
async fn twinless_demographic_is_still_hard_rejected() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();
    // A structurally VALID identifier assertion, but with the authored twin DROPPED.
    let a = IdentifierAssertion {
        value: "943 476 5919", system: "nhs-number", provenance: "document-verified",
        normalized: Some("9434765919"), profile: Some("nhs-number@b3-abc"), use_: Some("national-id"),
    };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: "demographic.identifier.asserted".into(),
        schema_version: "demographic.identifier/1".into(),
        hlc: Hlc { wall: 1, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: identifier_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: None, // <-- the floor must reject this for a demographic type
    };
    let signed = sign(&body, &sk).unwrap();
    let err = c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await.expect_err("twin-less demographic must be rejected");
    // err.to_string() for a DB error yields "db error" — the actual PG message is in
    // as_db_error().message() (project convention; see admission.rs, restore.rs).
    let msg = err.as_db_error().map(|e| e.message()).unwrap_or("<no db message>");
    assert!(msg.contains("authored twin") || msg.contains("§4.5"), "rejection cites the twin: {msg}");
    // Triple-gate: nothing landed in the log.
    let n: i64 = c.query_one(
        "SELECT count(*) FROM event_log WHERE patient_id::text=$1", &[&p.to_string()])
        .await.unwrap().get(0);
    assert_eq!(n, 0, "rejected demographic event is not stored");
}

#[tokio::test]
async fn whitespace_twin_demographic_is_still_hard_rejected() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();
    // A structurally VALID identifier assertion, but with a WHITESPACE-ONLY authored twin.
    // The floor must treat a blank twin as equivalent to an absent twin (ADR-0039).
    let a = IdentifierAssertion {
        value: "943 476 5920", system: "nhs-number", provenance: "document-verified",
        normalized: Some("9434765920"), profile: Some("nhs-number@b3-abc"), use_: Some("national-id"),
    };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: "demographic.identifier.asserted".into(),
        schema_version: "demographic.identifier/1".into(),
        hlc: Hlc { wall: 1, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: identifier_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: Some("   \n".into()), // <-- whitespace-only; must be treated as blank
    };
    let signed = sign(&body, &sk).unwrap();
    let err = c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await.expect_err("whitespace-only twin demographic must be rejected");
    let msg = err.as_db_error().map(|e| e.message()).unwrap_or("<no db message>");
    assert!(msg.contains("authored twin") || msg.contains("§4.5"), "rejection cites the twin: {msg}");
    // Triple-gate: nothing landed in the log.
    let n: i64 = c.query_one(
        "SELECT count(*) FROM event_log WHERE patient_id::text=$1", &[&p.to_string()])
        .await.unwrap().get(0);
    assert_eq!(n, 0, "rejected demographic event is not stored");
}
