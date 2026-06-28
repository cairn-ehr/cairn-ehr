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
#[allow(clippy::too_many_arguments)]
async fn submit_field(
    c: &Client, sk: &SigningKey, kid: &str, patient: Uuid, wall: i64, counter: i32,
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
