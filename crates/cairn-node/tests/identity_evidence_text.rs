//! §5.4 TEXT identity-evidence e2e (marks / belongings / EMS-context). Registers a John Doe,
//! records a distinguishing-mark evidence assertion through `assert_text_evidence`, and proves:
//! the event lands with the right type/kind/description/provenance, the authored twin is legible,
//! `attachments` is empty, and the orchestrator's guards reject a bad kind / empty description
//! before writing anything. Real Postgres, gated on $CAIRN_TEST_PG.

use cairn_node::db;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

async fn setup(c: &Client) -> (cairn_event::SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name, blob_store, blob_chunk CASCADE",
    )
    .await
    .unwrap();
    db::reset_node_federation_tables(c).await.unwrap();
    let (sk, kid) = cairn_event::generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid]).await.unwrap();
    cairn_node::identity::provision(c, &sk, &kid, "test-node", "127.0.0.1:0")
        .await
        .unwrap();
    (sk, kid)
}

#[tokio::test]
async fn mark_evidence_lands_with_kind_description_provenance_and_legible_twin() {
    let Some(base) = cs() else {
        eprintln!("skip: no CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap(); // conn string; hold until drop
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let id = cairn_node::identity::load_local(&c).await.unwrap();

    // A realistic §5.4 flow: register the unidentified patient, then record a mark.
    let (patient, _callsign) = cairn_node::john_doe::register_john_doe(
        &mut c,
        &sk,
        &kid,
        &id.node_id_hex,
        "ED",
        "site1",
        "2026-07-08",
        "unconscious ED arrival, no ID",
    )
    .await
    .unwrap();

    let event_id = cairn_node::identity_evidence::assert_text_evidence(
        &c,
        &sk,
        &kid,
        &id.node_id_hex,
        patient,
        "mark",
        "scar on left forearm, ~5cm, healed",
        Some("visible on primary survey"),
    )
    .await
    .unwrap();

    // Read the derived body view + top-level twin/attachments columns back. The `body` column
    // holds the event PAYLOAD directly (envelope fields — event_type, attachments, twin — are
    // their own columns), so the payload keys are top-level in `body`.
    let eid = event_id.to_string();
    let r = c
        .query_one(
            "SELECT event_type, body->>'kind', body->>'description', \
                body->>'provenance', attachments::text, plaintext_twin \
         FROM event_log WHERE event_id = $1::text::uuid",
            &[&eid],
        )
        .await
        .unwrap();
    let event_type: String = r.get(0);
    let kind: String = r.get(1);
    let description: String = r.get(2);
    let provenance: String = r.get(3);
    let atts: String = r.get(4);
    let twin: String = r.get(5);

    assert_eq!(event_type, "identity.evidence.asserted");
    assert_eq!(kind, "mark");
    assert_eq!(description, "scar on left forearm, ~5cm, healed");
    assert_eq!(provenance, "clinician-observed");
    assert_eq!(atts, "[]", "a text kind carries no attachment: {atts}");
    assert!(
        twin.contains("scar on left forearm"),
        "twin is legible: {twin}"
    );
    assert!(twin.contains("mark"), "twin names the kind: {twin}");
    assert!(
        twin.contains("visible on primary survey"),
        "twin carries the basis: {twin}"
    );
}

#[tokio::test]
async fn assert_text_evidence_rejects_bad_kind_and_empty_description_without_writing() {
    let Some(base) = cs() else {
        eprintln!("skip: no CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let id = cairn_node::identity::load_local(&c).await.unwrap();
    let (patient, _callsign) = cairn_node::john_doe::register_john_doe(
        &mut c,
        &sk,
        &kid,
        &id.node_id_hex,
        "ED",
        "site1",
        "2026-07-08",
        "unconscious ED arrival, no ID",
    )
    .await
    .unwrap();

    // Unknown kind → error, no event authored.
    let bad_kind = cairn_node::identity_evidence::assert_text_evidence(
        &c,
        &sk,
        &kid,
        &id.node_id_hex,
        patient,
        "scar",
        "left forearm",
        None,
    )
    .await;
    assert!(bad_kind.is_err(), "an unknown kind must be refused");

    // Empty description → error, no event authored.
    let empty = cairn_node::identity_evidence::assert_text_evidence(
        &c,
        &sk,
        &kid,
        &id.node_id_hex,
        patient,
        "mark",
        "   ",
        None,
    )
    .await;
    assert!(empty.is_err(), "an empty description must be refused");

    // Nothing landed for this patient.
    let pid = patient.to_string();
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE patient_id = $1::text::uuid \
         AND event_type = 'identity.evidence.asserted'",
            &[&pid],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 0,
        "no evidence event should have been written by the rejected calls"
    );
}
