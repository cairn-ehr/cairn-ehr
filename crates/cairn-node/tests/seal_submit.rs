//! ADR-0052 strict-door sealed arm (db/005). DB-gated on $CAIRN_TEST_PG,
//! serialized cluster-wide via db::test_serial_guard (shared-DB + TRUNCATE pattern,
//! like medication.rs). Key material is derived at runtime (generate_key), never a
//! literal (house rule 6).
//!
//! These pin the four legs of the born-sealed floor contract:
//! - sealed + DEK: full strict checks on the CLEAR body, custody + shadow stored,
//!   projections fire through the shadow;
//! - sealed without DEK: refused legibly at THIS door;
//! - unsealed clinical.*: refused (born-sealed floor);
//! - wrong DEK: refused (the container will not open).
use cairn_event::seal::{
    derive_unwrap_secret, seal_event_payload, seal_stub_twin, unwrap_dek, unwrap_public,
};
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;
use zeroize::Zeroizing;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The Postgres error message text for a failed statement (tokio_postgres wraps a
/// DB-originated error as a generic "db error"; the real RAISE text is on the DbError).
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Truncate the log + custody plane + medication projections and enroll a fresh device
/// actor. node_unwrap_key/event_dek/event_clear/erasure_shred_log have NO FK to
/// event_log, so the CASCADE from event_log does not reach them — they must be named
/// explicitly, or a prior test's node key would collide with this test's fresh one at
/// cairn_register_unwrap_key (the singleton refuses a different key).
async fn setup_node(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, \
         node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
    )
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

/// Build a CLEAR medication-assert EventBody (mirror of medication/assert.rs's field
/// construction), then seal it: payload → container, twin → stub. Returns the sealed
/// body (ready to sign) and the DEK the strict door needs as its 4th arg.
fn sealed_assert_body(node_kid: &str, patient: Uuid, hlc: Hlc) -> (EventBody, Zeroizing<[u8; 32]>) {
    let event_id = Uuid::now_v7().to_string();
    let medication_id = Uuid::now_v7().to_string();
    let payload = serde_json::json!({
        "medication_id": medication_id,
        "substance": {"term": "amoxicillin"},
        "info_source": "patient",
    });
    let twin = format!("amoxicillin — asserted for {patient}");
    let (container, dek) = seal_event_payload(&payload, &twin, &event_id).unwrap();
    let body = EventBody {
        event_id,
        patient_id: patient.to_string(),
        event_type: "clinical.medication.asserted".into(),
        schema_version: "clinical.medication/1".into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: container,
        attachments: vec![],
        plaintext_twin: Some(seal_stub_twin("clinical.medication.asserted")),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    (body, dek)
}

/// The SAME clear body as `sealed_assert_body`, but LEFT UNSEALED — a legacy plaintext
/// clinical body (payload is the clear medication payload, plaintext_twin is the clear
/// twin, no container). This is exactly what the born-sealed floor must refuse.
fn unsealed_assert_body(node_kid: &str, patient: Uuid, hlc: Hlc) -> EventBody {
    let event_id = Uuid::now_v7().to_string();
    let medication_id = Uuid::now_v7().to_string();
    let payload = serde_json::json!({
        "medication_id": medication_id,
        "substance": {"term": "amoxicillin"},
        "info_source": "patient",
    });
    let twin = format!("amoxicillin — asserted for {patient}");
    EventBody {
        event_id,
        patient_id: patient.to_string(),
        event_type: "clinical.medication.asserted".into(),
        schema_version: "clinical.medication/1".into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    }
}

#[tokio::test]
async fn sealed_submit_with_dek_projects_and_stores_custody() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // Register this node's X25519 unwrap key so the door can wrap the DEK into custody.
    let secret = derive_unwrap_secret(&sk.to_bytes());
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&unwrap_public(&secret).as_slice()],
    )
    .await
    .unwrap();

    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let (body, dek) = sealed_assert_body(&kid, patient, hlc);
    let event_id = body.event_id.clone();
    let signed = sign(&body, &sk).unwrap();
    c.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_slice()],
    )
    .await
    .expect("sealed body with its DEK is admitted");

    // event_log stores the CIPHERTEXT container, the outer stub twin, and sealed = true.
    let (sealed, twin, body_text): (bool, String, String) = {
        let row = c
            .query_one(
                "SELECT sealed, plaintext_twin, body::text FROM event_log WHERE event_id = $1::text::uuid",
                &[&event_id],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1), row.get(2))
    };
    assert!(sealed, "the row is marked sealed");
    assert!(
        twin.contains("twin under seal"),
        "the stored outer twin is the mechanical stub, got: {twin}"
    );
    let body_json: serde_json::Value = serde_json::from_str(&body_text).unwrap();
    assert_eq!(
        body_json["sealed"],
        serde_json::json!(true),
        "event_log.body is the sealed container"
    );
    assert!(
        !body_text.contains("amoxicillin"),
        "no cleartext leaks into event_log.body"
    );

    // event_clear holds the CLEAR payload + twin (the operational shadow).
    let (clear_term, clear_twin): (String, String) = {
        let row = c
            .query_one(
                "SELECT body -> 'substance' ->> 'term', twin FROM event_clear WHERE event_id = $1::text::uuid",
                &[&event_id],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1))
    };
    assert_eq!(clear_term, "amoxicillin");
    assert!(
        clear_twin.contains("amoxicillin"),
        "the clear twin is stored"
    );

    // event_dek holds the 104-byte wrapped DEK, and it unwraps back to our DEK with the
    // node's secret half — proving custody is genuinely recoverable for a future shred.
    let dek_wrapped: Vec<u8> = c
        .query_one(
            "SELECT dek_wrapped FROM event_dek WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(dek_wrapped.len(), 104, "wrapped DEK is eph‖nonce‖ct+tag");
    let opened = unwrap_dek(&dek_wrapped, &secret).unwrap();
    assert_eq!(
        opened.as_slice(),
        dek.as_slice(),
        "custody unwraps to the DEK"
    );

    // The projection fired THROUGH the shadow: the medication statement exists.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_statement WHERE patient_id = $1::text::uuid AND term = 'amoxicillin'",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the sealed assert projected via the clear shadow");
}

#[tokio::test]
async fn sealed_submit_without_dek_is_refused_legibly() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let (body, _dek) = sealed_assert_body(&kid, patient, hlc);
    let signed = sign(&body, &sk).unwrap();
    // The 1-arg call presents the sealed body but no DEK — the door cannot open it.
    let err = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .expect_err("a sealed body without its DEK must be refused");
    assert!(
        db_msg(&err).contains("requires its DEK"),
        "the refusal names the missing DEK, got: {}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn unsealed_clinical_body_is_refused_at_the_strict_door() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body = unsealed_assert_body(&kid, patient, hlc);
    let signed = sign(&body, &sk).unwrap();
    // A plaintext clinical body is permanently un-shreddable — the strict door refuses it.
    let err = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .expect_err("a plaintext clinical body must be refused at the strict door");
    assert!(
        db_msg(&err).contains("born-sealed"),
        "the refusal names the born-sealed floor, got: {}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn wrong_dek_is_refused() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let secret = derive_unwrap_secret(&sk.to_bytes());
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&unwrap_public(&secret).as_slice()],
    )
    .await
    .unwrap();

    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let (body, _dek) = sealed_assert_body(&kid, patient, hlc);
    let signed = sign(&body, &sk).unwrap();
    // A DEK derived at runtime that is NOT the sealing DEK (house rule 6: never a literal)
    // — the AEAD open must fail and the door must refuse rather than store garbage.
    let wrong_dek: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(3).wrapping_add(1));
    let err = c
        .execute(
            "SELECT submit_event($1, NULL, NULL, $2)",
            &[&signed.signed_bytes, &wrong_dek.as_slice()],
        )
        .await
        .expect_err("the wrong DEK must be refused");
    assert!(
        db_msg(&err).contains("failed to open"),
        "the refusal names the failed open, got: {}",
        db_msg(&err)
    );
}

/// Build a CLEAR demographic.field.asserted payload (dob) and wrongly SEAL it. Demographic
/// bodies are NON-clinical — plaintext BY NECESSITY (their projections/matchers bind on
/// NEW.body directly), so a sealed one is the never-lawful shape ADR-0052 §2 forbids.
/// Returns the sealed body (ready to sign) and its DEK.
fn sealed_demographic_body(
    node_kid: &str,
    patient: Uuid,
    hlc: Hlc,
) -> (EventBody, Zeroizing<[u8; 32]>) {
    let event_id = Uuid::now_v7().to_string();
    let payload = serde_json::json!({
        "field": "dob",
        "value": "1980",
        "provenance": "document-verified",
    });
    let twin = format!("dob 1980 — asserted for {patient}");
    let (container, dek) = seal_event_payload(&payload, &twin, &event_id).unwrap();
    let body = EventBody {
        event_id,
        patient_id: patient.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: "demographic.field/1".into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: container,
        attachments: vec![],
        plaintext_twin: Some(seal_stub_twin("demographic.field.asserted")),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    (body, dek)
}

#[tokio::test]
async fn sealed_non_clinical_body_is_refused_at_the_strict_door() {
    // ADR-0052 §2 (inverse of the born-sealed floor): ONLY clinical.* bodies are sealed;
    // demographic/identity/patient/node/erasure bodies are plaintext by necessity. A sealed
    // NON-clinical body is a never-lawful shape — its ciphertext body can never project
    // (the projection reads NEW.body directly). The strict door must refuse it CLEANLY,
    // BEFORE it is stored, or its ciphertext would detonate a NEW.body-reading projection.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // Register the node unwrap key so that — WITHOUT the scope check — the door would run
    // the full sealed path all the way to the projection INSERT that detonates. The scope
    // refusal must fire FIRST, stopping the never-lawful shape at the boundary.
    let secret = derive_unwrap_secret(&sk.to_bytes());
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&unwrap_public(&secret).as_slice()],
    )
    .await
    .unwrap();

    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let (body, dek) = sealed_demographic_body(&kid, patient, hlc);
    let signed = sign(&body, &sk).unwrap();
    let err = c
        .execute(
            "SELECT submit_event($1, NULL, NULL, $2)",
            &[&signed.signed_bytes, &dek.as_slice()],
        )
        .await
        .expect_err("a sealed NON-clinical body must be refused at the strict door");
    assert!(
        db_msg(&err).contains("only clinical") && db_msg(&err).contains("ADR-0052"),
        "the refusal names the seal-scope floor (ADR-0052 §2), got: {}",
        db_msg(&err)
    );
}
