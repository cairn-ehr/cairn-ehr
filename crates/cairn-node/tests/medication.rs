//! §3.15 medication recording — DB-gated on $CAIRN_TEST_PG, serialized cluster-wide
//! via db::test_serial_guard (shared-DB + TRUNCATE pattern, like identify.rs).
//! Patients need no pre-existence (offline-first: no patient FK), so tests use a
//! bare Uuid as the patient. Key material is derived at runtime (generate_key).
use cairn_event::{generate_key, sign, EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::{
    assert_medication, build_assert_body, cease_medication, AssertMedicationInput,
    CeaseMedicationInput,
};
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The Postgres error message text for a failed statement (see identity_dispute.rs /
/// identity_repudiate.rs) — `tokio_postgres::Error::to_string()` for a DB-originated
/// error just returns the generic "db error"; the real RAISE EXCEPTION text lives on
/// the wrapped DbError.
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Truncate the log + medication projections and enroll a fresh device actor.
async fn setup_node(c: &Client) -> (SigningKey, String) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
        .await
        .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.medication_statement') IS NOT NULL THEN TRUNCATE medication_statement; END IF; \
           IF to_regclass('public.medication_cessation') IS NOT NULL THEN TRUNCATE medication_cessation; END IF; \
         END $$;",
    )
    .await
    .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

fn sample_input() -> AssertMedicationInput<'static> {
    AssertMedicationInput {
        term: "atorvastatin",
        inn_code: None,
        formulation: Some("tablet"),
        dose_amount: Some("40"),
        dose_unit: Some("mg"),
        sig: Some("one BD"),
        info_source: "patient-reported",
        started: Some("2024"),
        started_precision: Some("year"),
    }
}

async fn current_terms(c: &Client, patient: Uuid) -> Vec<String> {
    c.query(
        "SELECT term FROM patient_medication_current WHERE patient_id = $1::text::uuid ORDER BY term",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get::<_, String>(0))
    .collect()
}

#[tokio::test]
async fn assert_appears_as_current() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_input())
        .await
        .unwrap();
    assert_eq!(
        current_terms(&c, patient).await,
        vec!["atorvastatin".to_string()]
    );

    // The thread id is a real minted uuid.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_statement WHERE medication_id = $1::text::uuid",
            &[&med_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1);
}

#[tokio::test]
async fn empty_term_is_rejected_by_the_floor() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // Bypass the Rust validate_term guard: hand-build a whitespace-only-term event
    // and submit it directly, proving the DB FLOOR rejects it (defense in depth).
    let mut input = sample_input();
    input.term = "   ";
    // Use a real HLC tick so the ONLY rejection reason is the empty term (not an
    // HLC regression against node state).
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body: EventBody =
        build_assert_body(Uuid::now_v7(), Uuid::now_v7(), patient, &input, &kid, hlc);
    let signed = sign(&body, &sk).unwrap();
    let res = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await;
    let err = db_msg(&res.unwrap_err());
    assert!(
        err.contains("term"),
        "floor must reject empty term, got: {err}"
    );
    assert!(current_terms(&c, patient).await.is_empty());
}

#[tokio::test]
async fn validate_term_rejects_blank() {
    // Pure guard test — no DB needed.
    assert!(cairn_node::medication::validate_term("  ").is_err());
    assert!(cairn_node::medication::validate_term("aspirin").is_ok());
}

async fn past_terms(c: &Client, patient: Uuid) -> Vec<String> {
    c.query(
        "SELECT term FROM patient_medication_past WHERE patient_id = $1::text::uuid ORDER BY term",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get::<_, String>(0))
    .collect()
}

/// Inject an assert with a CHOSEN medication_id (the orchestrator mints its own,
/// so tests that need a specific thread id build+sign+submit directly).
async fn inject_assert(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    medication_id: Uuid,
    input: &AssertMedicationInput<'_>,
) {
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let body: EventBody =
        build_assert_body(Uuid::now_v7(), medication_id, patient, input, kid, hlc);
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap();
}

#[tokio::test]
async fn cease_flips_current_to_past() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_input())
        .await
        .unwrap();
    assert_eq!(
        current_terms(&c, patient).await,
        vec!["atorvastatin".to_string()]
    );

    cease_medication(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        &CeaseMedicationInput {
            stopped: Some("2025"),
            stopped_precision: Some("year"),
            reason: Some("switched"),
        },
    )
    .await
    .unwrap();

    assert!(
        current_terms(&c, patient).await.is_empty(),
        "ceased med leaves current"
    );
    assert_eq!(
        past_terms(&c, patient).await,
        vec!["atorvastatin".to_string()]
    );
}

#[tokio::test]
async fn orphan_cessation_has_no_row_then_resolves_on_assert_arrival() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med_id = Uuid::now_v7();

    // Cessation authored BEFORE its assert exists locally (offline-first).
    cease_medication(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        &CeaseMedicationInput {
            stopped: None,
            stopped_precision: None,
            reason: None,
        },
    )
    .await
    .unwrap();
    assert!(current_terms(&c, patient).await.is_empty());
    assert!(
        past_terms(&c, patient).await.is_empty(),
        "orphan cessation shows no renderable row"
    );

    // The assert for that same thread now replicates in.
    inject_assert(&c, &sk, &kid, patient, med_id, &sample_input()).await;
    assert!(
        current_terms(&c, patient).await.is_empty(),
        "still ceased — not current"
    );
    assert_eq!(
        past_terms(&c, patient).await,
        vec!["atorvastatin".to_string()],
        "thread now surfaces in past, arrival-order-independent"
    );
}
