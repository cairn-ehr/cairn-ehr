//! ADR-0053 author-time human authorship on the medication stream. DB-gated on
//! $CAIRN_TEST_PG, serialized cluster-wide via db::test_serial_guard. When a human
//! author is supplied, the content event is signed by the human and carries
//! {human,"authored"} + {node,"recorded"}, while the node keeps custody (event_dek).
use cairn_event::generate_key;
use cairn_node::db;
use cairn_node::medication::{assert_medication, AssertMedicationInput, AuthorParams};
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

async fn setup(
    c: &Client,
) -> (
    cairn_event::SigningKey,
    String,
    cairn_event::SigningKey,
    String,
) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, \
         node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.medication_statement') IS NOT NULL THEN TRUNCATE medication_statement; END IF; \
         END $$;",
    )
    .await
    .unwrap();
    let (node_sk, node_kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)",
        &[&node_kid],
    )
    .await
    .unwrap();
    let (human_sk, human_kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&human_kid],
    )
    .await
    .unwrap();
    (node_sk, node_kid, human_sk, human_kid)
}

#[tokio::test]
async fn human_authored_medication_is_signed_by_the_human_node_keeps_custody() {
    let Some(cs) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let mut c = db::connect_and_load_schema(&cs).await.unwrap();
    let (node_sk, node_kid, human_sk, human_kid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let input = AssertMedicationInput {
        term: "atorvastatin",
        inn_code: None,
        formulation: None,
        dose_amount: Some("40"),
        dose_unit: Some("mg"),
        sig: None,
        info_source: "patient-reported",
        started: None,
        started_precision: None,
    };
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let med_id = assert_medication(
        &mut c,
        &node_sk,
        &node_kid,
        &node_kid,
        patient,
        &input,
        Some(&author),
        None,
    )
    .await
    .unwrap();

    // The content event is signed by the human and names both contributors.
    let row = c
        .query_one(
            "SELECT signer_key_id, contributors::text, sealed FROM event_log \
             WHERE event_type = 'clinical.medication.asserted' \
               AND (cairn_clear_payload(event_log) ->> 'medication_id')::uuid = $1::text::uuid",
            &[&med_id.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>("signer_key_id"), human_kid);
    let contributors: serde_json::Value =
        serde_json::from_str(&row.get::<_, String>("contributors")).unwrap();
    assert_eq!(contributors[0]["actor_id"], human_kid);
    assert_eq!(contributors[0]["role"], "authored");
    assert_eq!(contributors[1]["actor_id"], node_kid);
    assert_eq!(contributors[1]["role"], "recorded");
    assert!(row.get::<_, bool>("sealed"));

    // The NODE (not the human) holds custody: an event_dek row exists for this event.
    let event_id: String = c
        .query_one(
            "SELECT event_id::text FROM event_log \
             WHERE (cairn_clear_payload(event_log) ->> 'medication_id')::uuid = $1::text::uuid \
               AND event_type = 'clinical.medication.asserted'",
            &[&med_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    let custody: i64 = c
        .query_one(
            "SELECT count(*) FROM event_dek WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        custody, 1,
        "the node must hold the DEK even though the human signed"
    );
}

#[tokio::test]
async fn human_authored_cessation_is_signed_by_the_human() {
    let Some(cs) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let mut c = db::connect_and_load_schema(&cs).await.unwrap();
    let (node_sk, node_kid, human_sk, human_kid) = setup(&c).await;
    let patient = Uuid::now_v7();
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };

    let input = AssertMedicationInput {
        term: "warfarin",
        inn_code: None,
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "patient-reported",
        started: None,
        started_precision: None,
    };
    let med_id = assert_medication(
        &mut c,
        &node_sk,
        &node_kid,
        &node_kid,
        patient,
        &input,
        Some(&author),
        None,
    )
    .await
    .unwrap();

    let cease_input = cairn_node::medication::CeaseMedicationInput {
        stopped: None,
        stopped_precision: None,
        reason: Some("bleeding risk"),
    };
    cairn_node::medication::cease_medication(
        &mut c,
        &node_sk,
        &node_kid,
        &node_kid,
        patient,
        med_id,
        &cease_input,
        Some(&author),
        None,
    )
    .await
    .unwrap();

    let signer: String = c
        .query_one(
            "SELECT signer_key_id FROM event_log WHERE event_type = 'clinical.medication-cessation.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(signer, human_kid);
}

/// The Postgres RAISE text behind a failed statement.
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Craft a sealed `clinical.medication.asserted` that CLAIMS `human_kid` authored it
/// but is SIGNED BY THE NODE, no attestation token — a forgery. Returns the signed wire
/// bytes + the DEK (the strict door's 4th arg; unused before the step-4b refusal).
fn craft_forged_authorship_event(
    node_sk: &cairn_event::SigningKey,
    node_kid: &str,
    human_kid: &str,
    patient: Uuid,
) -> (Vec<u8>, zeroize::Zeroizing<[u8; 32]>) {
    use cairn_event::seal::{seal_event_payload, seal_stub_twin};
    use cairn_event::{sign, EventBody, Hlc};
    let event_id = Uuid::now_v7();
    let medication_id = Uuid::now_v7();
    let payload = serde_json::json!({
        "medication_id": medication_id.to_string(),
        "term": "atorvastatin", "info_source": "patient-reported"
    });
    let (container, dek) = seal_event_payload(
        &payload,
        "Atorvastatin (patient-reported)",
        &event_id.to_string(),
    )
    .unwrap();
    let body = EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication.asserted".into(),
        schema_version: "clinical.medication/1".into(),
        hlc: Hlc {
            wall: 1_700_000_000_000,
            counter: 0,
            node_origin: node_kid.to_string(),
        },
        t_effective: None,
        signer_key_id: node_kid.to_string(), // the NODE signs...
        contributors: serde_json::json!([
            {"actor_id": human_kid, "role": "authored"}, // ...but claims the human authored
            {"actor_id": node_kid,  "role": "recorded"}]),
        payload: container,
        attachments: vec![],
        plaintext_twin: Some(seal_stub_twin("clinical.medication.asserted")),
    };
    let signed = sign(&body, node_sk).unwrap();
    (signed.signed_bytes, dek)
}

#[tokio::test]
async fn forged_authorship_refused_at_the_strict_door() {
    let Some(cs) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let c = db::connect_and_load_schema(&cs).await.unwrap();
    let (node_sk, node_kid, _human_sk, human_kid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let (signed, dek) = craft_forged_authorship_event(&node_sk, &node_kid, &human_kid, patient);
    let err = c
        .execute(
            "SELECT submit_event($1, NULL, NULL, $2)",
            &[&signed, &dek.as_slice()],
        )
        .await
        .expect_err("forged authorship must be refused");
    assert!(
        db_msg(&err).contains("forged authorship refused"),
        "expected the ADR-0053 authorship-binding refusal, got: {}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn device_additive_assert_still_valid_with_no_author() {
    let Some(cs) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let mut c = db::connect_and_load_schema(&cs).await.unwrap();
    let (node_sk, node_kid, _hs, _hk) = setup(&c).await;
    let patient = Uuid::now_v7();
    let input = AssertMedicationInput {
        term: "metformin",
        inn_code: None,
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "patient-reported",
        started: None,
        started_precision: None,
    };
    // No author, no attest -> device-additive: node signs, recorded-only. Must succeed.
    let med_id = assert_medication(
        &mut c, &node_sk, &node_kid, &node_kid, patient, &input, None, None,
    )
    .await
    .unwrap();
    let signer: String = c
        .query_one(
            "SELECT signer_key_id FROM event_log WHERE event_type = 'clinical.medication.asserted' \
               AND (cairn_clear_payload(event_log) ->> 'medication_id')::uuid = $1::text::uuid",
            &[&med_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        signer, node_kid,
        "device-additive assert is still signed by the node"
    );
}

#[tokio::test]
async fn human_author_owns_suppression_rights() {
    let Some(cs) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let mut c = db::connect_and_load_schema(&cs).await.unwrap();
    let (node_sk, node_kid, human_sk, human_kid) = setup(&c).await;
    let patient = Uuid::now_v7();
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let input = AssertMedicationInput {
        term: "lisinopril",
        inn_code: None,
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "patient-reported",
        started: None,
        started_precision: None,
    };
    let med_id = assert_medication(
        &mut c,
        &node_sk,
        &node_kid,
        &node_kid,
        patient,
        &input,
        Some(&author),
        None,
    )
    .await
    .unwrap();
    let event_id: String = c
        .query_one(
            "SELECT event_id::text FROM event_log WHERE event_type = 'clinical.medication.asserted' \
               AND (cairn_clear_payload(event_log) ->> 'medication_id')::uuid = $1::text::uuid",
            &[&med_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);

    // The human author IS an owner (may suppress their own event).
    let author_vk = human_sk.verifying_key().to_bytes().to_vec();
    let owns: bool = c
        .query_one(
            "SELECT cairn_suppression_author_ok($1::text::uuid, $2)",
            &[&event_id, &author_vk],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        owns,
        "the human author must own suppression rights over their own event"
    );

    // A different human does NOT (cross-human suppression is refused — ADR-0043).
    let (other_sk, _other_kid) = cairn_event::generate_key().unwrap();
    let other_vk = other_sk.verifying_key().to_bytes().to_vec();
    let stranger_owns: bool = c
        .query_one(
            "SELECT cairn_suppression_author_ok($1::text::uuid, $2)",
            &[&event_id, &other_vk],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !stranger_owns,
        "a stranger must NOT own suppression rights over the human's event"
    );
}
