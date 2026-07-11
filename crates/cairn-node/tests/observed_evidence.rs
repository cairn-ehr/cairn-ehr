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

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

async fn setup(c: &Client) -> (cairn_event::SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name CASCADE",
    )
    .await
    .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid]).await.unwrap();
    (sk, kid)
}

/// Read one (value, facets, provenance) demographic projection row for a field.
///
/// `facets` is fetched as `::text` (not the jsonb type directly) because `tokio-postgres`
/// has no `FromSql<serde_json::Value>` impl without the `with-serde_json-1` feature, which
/// this crate does not enable (no new dependency features per the task's constraints).
/// `demographics_fields.rs` sidesteps the same gap with `facets->>'key'`; here the caller
/// wants the whole object, so we cast to text and parse it back to `Value` in Rust — same
/// observable projection content, no new capability pulled in.
async fn demographic_of(
    c: &Client,
    patient: Uuid,
    field: &str,
) -> Option<(String, serde_json::Value, String)> {
    let p = patient.to_string();
    c.query_opt(
        "SELECT value, facets::text, provenance FROM patient_demographic \
         WHERE patient_id = $1::text::uuid AND field = $2",
        &[&p, &field],
    )
    .await
    .unwrap()
    .map(|r| {
        // `facets` is nullable (e.g. observed-sex with no stated basis omits it
        // entirely) — Option<String> so a NULL column doesn't panic the row decode.
        let facets_text: Option<String> = r.get(1);
        let facets = facets_text
            .map(|t| serde_json::from_str(&t).unwrap())
            .unwrap_or(serde_json::Value::Null);
        (r.get(0), facets, r.get(2))
    })
}

/// Author a single document-verified point dob (`precision=day`, rank 60) through the real
/// door, so a test can prove it DISPLACES a rank-30 clinician estimate in the projection.
async fn author_document_dob(
    db: &mut Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    node: &str,
    patient: Uuid,
    iso: &str,
) {
    let h = db::next_hlc(db, node).await.unwrap();
    let payload = cairn_event::demographics::dob_assertion_body(
        iso,
        "day",
        Some("passport"),
        "document-verified",
    );
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
        plaintext_twin: Some(cairn_event::demographics::render_dob_twin(
            iso,
            "day",
            "document-verified",
        )),
    };
    let signed = cairn_event::sign(&body, sk).unwrap();
    db.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap();
}

#[tokio::test]
async fn estimated_age_lands_as_a_clinician_observed_year_range_dob() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut db = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&db).await;
    let patient = Uuid::now_v7();

    let ev = ObservedEvidence {
        age: Some(AgeObservation {
            age_years: 40,
            tolerance_years: 5,
            basis: "dentition, greying".into(),
        }),
        sex: None,
    };
    evidence::assert_observed_evidence(&mut db, &sk, &kid, "test-node", patient, &ev, 2026)
        .await
        .unwrap();

    let (value, facets, prov) = demographic_of(&db, patient, "dob")
        .await
        .expect("dob projected");
    assert_eq!(value, "1981/1991");
    assert_eq!(facets["precision"], "year-range");
    assert_eq!(prov, "clinician-observed");
}

#[tokio::test]
async fn a_document_verified_dob_displaces_the_estimate() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut db = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&db).await;
    let patient = Uuid::now_v7();

    // First the clinician estimate (rank 30)...
    let est = ObservedEvidence {
        age: Some(AgeObservation {
            age_years: 40,
            tolerance_years: 5,
            basis: "dentition".into(),
        }),
        sex: None,
    };
    evidence::assert_observed_evidence(&mut db, &sk, &kid, "test-node", patient, &est, 2026)
        .await
        .unwrap();
    // ...then a document-verified exact dob (rank 60) — must win the projection.
    author_document_dob(&mut db, &sk, &kid, "test-node", patient, "1985-03-12").await;

    let (value, _facets, prov) = demographic_of(&db, patient, "dob")
        .await
        .expect("dob projected");
    assert_eq!(
        prov, "document-verified",
        "the document must displace the clinician estimate"
    );
    assert_eq!(value, "1985-03-12");
}

#[tokio::test]
async fn observed_sex_lands_on_administrative_sex() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut db = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&db).await;
    let patient = Uuid::now_v7();

    let ev = ObservedEvidence {
        age: None,
        sex: Some(SexObservation {
            value: "male".into(),
            basis: Some("external genitalia".into()),
        }),
    };
    evidence::assert_observed_evidence(&mut db, &sk, &kid, "test-node", patient, &ev, 2026)
        .await
        .unwrap();

    let (value, _facets, prov) = demographic_of(&db, patient, "administrative-sex")
        .await
        .expect("sex projected");
    assert_eq!(value, "male");
    assert_eq!(prov, "clinician-observed");
    // sex-at-birth must be UNTOUCHED — the clinician never claimed the birth fact.
    assert!(demographic_of(&db, patient, "sex-at-birth").await.is_none());
}

/// End-to-end loop between the pure `resolve_observed_year` validator and the DB projection:
/// a `--observed-year 2000` override resolves to 2000 (not "now"), and an age-40±5 estimate
/// authored against that override lands as the 1960-centred year-range (1955/1965), not a
/// range computed off the current year. Closes the design-doc-named DB-gated test gap.
#[tokio::test]
async fn observed_year_override_sets_the_birth_year_range() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut db = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&db).await;
    let patient = Uuid::now_v7();

    // The CLI edge resolves the observed year via the real validator, same as production.
    let oy = cairn_node::evidence::resolve_observed_year(Some(2000), 2026).unwrap();
    assert_eq!(oy, 2000);

    let ev = ObservedEvidence {
        age: Some(AgeObservation {
            age_years: 40,
            tolerance_years: 5,
            basis: "dentition, greying".into(),
        }),
        sex: None,
    };
    evidence::assert_observed_evidence(&mut db, &sk, &kid, "test-node", patient, &ev, oy)
        .await
        .unwrap();

    let (value, facets, prov) = demographic_of(&db, patient, "dob")
        .await
        .expect("dob projected");
    assert_eq!(value, "1955/1965");
    assert_eq!(facets["precision"], "year-range");
    assert_eq!(prov, "clinician-observed");
}

#[tokio::test]
async fn age_and_sex_are_authored_atomically() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut db = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&db).await;
    let patient = Uuid::now_v7();

    let ev = ObservedEvidence {
        age: Some(AgeObservation {
            age_years: 25,
            tolerance_years: 3,
            basis: "young adult".into(),
        }),
        sex: Some(SexObservation {
            value: "female".into(),
            basis: None,
        }),
    };
    evidence::assert_observed_evidence(&mut db, &sk, &kid, "test-node", patient, &ev, 2020)
        .await
        .unwrap();

    assert_eq!(
        demographic_of(&db, patient, "dob").await.unwrap().0,
        "1992/1998"
    );
    assert_eq!(
        demographic_of(&db, patient, "administrative-sex")
            .await
            .unwrap()
            .0,
        "female"
    );
}
