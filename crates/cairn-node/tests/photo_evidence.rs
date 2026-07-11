//! §5.4 photo evidence e2e (ADR-0042 attachment tier, first clinical-surface use). Registers
//! a John Doe, attaches a photo, and proves: the blob is present + re-verifies against the
//! db/026 floor, the event references it, and the authored twin is legible without pixels.
//! Real Postgres, gated on $CAIRN_TEST_PG.

use cairn_event::blob_address;
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
    // `reset_node_federation_tables` truncates `local_node`, so a fresh node identity must be
    // (re-)provisioned before `identity::load_local` (used below to get a real `node_id_hex`
    // for event authoring) can succeed — mirrors the reset->provision->load_local sequence
    // every other identity-using test in this suite follows (e.g. genesis_hlc.rs,
    // floor_enforced.rs, backup.rs). The node-genesis plane (`node_event`/`local_node`) never
    // touches `actor_current`, so reusing this same key for the clinical `agent` actor above
    // is safe — no dual-actor-mapping degradation (see `ensure_registration_actor`'s doc comment
    // in main.rs for why that guard exists).
    cairn_node::identity::provision(c, &sk, &kid, "test-node", "127.0.0.1:0")
        .await
        .unwrap();
    (sk, kid)
}

#[tokio::test]
async fn photo_evidence_stores_a_verified_blob_and_references_it() {
    let Some(base) = cs() else {
        eprintln!("skip: no CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap(); // conn string; hold until drop
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let id = cairn_node::identity::load_local(&c).await.unwrap();

    // A realistic §5.4 flow: register the unidentified patient, then attach a photo.
    let (patient, _callsign, _ord) = cairn_node::john_doe::register_john_doe(
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

    let photo = b"\xff\xd8\xff\xe0JFIF-pretend-jpeg-bytes";
    let event_id = cairn_node::photo_evidence::assert_photo_evidence(
        &mut c,
        &sk,
        &kid,
        &id.node_id_hex,
        patient,
        photo,
        "image/jpeg",
        "frontal face photograph of unidentified patient",
        Some("on arrival"),
    )
    .await
    .unwrap();

    // 1. The blob is present, and its bytes re-verify against the address (db/026 floor fn).
    let addr = blob_address(photo);
    let row = c.query_one(
        "SELECT present, cairn_blob_verify(blob_address, content) FROM blob_store WHERE blob_address = $1",
        &[&addr]).await.unwrap();
    let present: bool = row.get(0);
    let verifies: bool = row.get(1);
    assert!(present, "the locally-authored blob is present");
    assert!(
        verifies,
        "content re-hashes to the address (self-verifying)"
    );

    // 2. The event references the blob by digest in its stored attachments, and NOT the bytes.
    let (atts_text, twin): (String, String) = {
        // `tokio-postgres` has no `ToSql` impl for `uuid::Uuid` without the `with-uuid-1`
        // feature, which this crate does not enable (mirrors the text-cast pattern already
        // used for patient ids in observed_evidence.rs/john_doe.rs) — bind as text, cast in SQL.
        let eid = event_id.to_string();
        let r = c.query_one(
            "SELECT attachments::text, plaintext_twin FROM event_log WHERE event_id = $1::text::uuid",
            &[&eid]).await.unwrap();
        (r.get(0), r.get(1))
    };
    assert!(
        atts_text.contains(&hex::encode(&addr)),
        "attachment names the blob by digest"
    );
    assert!(
        !atts_text.contains("JFIF-pretend"),
        "pixel bytes are NOT inlined in the reference"
    );

    // 3. The authored twin is legible and pixel-free.
    assert!(
        twin.contains("frontal face photograph of unidentified patient"),
        "twin: {twin}"
    );
    assert!(twin.contains("image/jpeg"));
    assert!(
        !twin.contains("JFIF-pretend"),
        "twin is descriptor-derived, never pixels"
    );
}

#[tokio::test]
async fn photo_evidence_fills_a_preexisting_reference_only_placeholder() {
    // Regression for the ON CONFLICT DO NOTHING bug: a present=FALSE placeholder row may
    // already sit at the photo's content address (e.g. a remote-synced event referenced the
    // same photo before this node held its bytes). assert_photo_evidence must FLIP it to
    // present=TRUE with the real bytes (DO UPDATE), never silently discard them and commit
    // an event referencing an empty blob.
    let Some(base) = cs() else {
        eprintln!("skip: no CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap(); // conn string; hold until drop
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let id = cairn_node::identity::load_local(&c).await.unwrap();
    let (patient, _callsign, _ord) = cairn_node::john_doe::register_john_doe(
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

    // Pre-seat a reference-only placeholder (present=FALSE, content NULL) at the address.
    let photo = b"\xff\xd8\xff\xe0JFIF-second-photo-bytes";
    let addr = blob_address(photo);
    c.execute(
        "SELECT blob_note_reference($1, 'image/jpeg', $2)",
        &[&addr, &(photo.len() as i64)],
    )
    .await
    .unwrap();
    let before: bool = c
        .query_one(
            "SELECT present FROM blob_store WHERE blob_address = $1",
            &[&addr],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!before, "placeholder starts present=FALSE");

    // Now author the photo evidence with the real bytes.
    cairn_node::photo_evidence::assert_photo_evidence(
        &mut c,
        &sk,
        &kid,
        &id.node_id_hex,
        patient,
        photo,
        "image/jpeg",
        "second identification photograph",
        None,
    )
    .await
    .unwrap();

    let row = c.query_one(
        "SELECT present, cairn_blob_verify(blob_address, content) FROM blob_store WHERE blob_address = $1",
        &[&addr]).await.unwrap();
    let present: bool = row.get(0);
    let verifies: bool = row.get(1);
    assert!(
        present,
        "the placeholder must be flipped present with the real bytes, not left empty"
    );
    assert!(verifies, "the filled bytes re-hash to the address");
}
