//! §3.14/ADR-0042 floor coverage: the submit door learns a lazy blob reference for every
//! BY-REFERENCE rendition of an event's attachments (reference-eager, byte-lazy), skipping
//! inline renditions. Real Postgres, gated on $CAIRN_TEST_PG.
//!
//! Coverage note: the submit door (db/005) and the remote-apply door (db/020) learn references
//! through the SAME `cairn_learn_attachment_refs(b)` helper, so their rendition walk cannot
//! behaviourally diverge — the shared-helper guarantee. Both call sites are now exercised
//! directly here: `by_reference_rendition_registers_a_blob_reference_row` (submit) and
//! `apply_door_learns_a_blob_reference_per_by_reference_rendition` (apply), so a future edit that
//! breaks either door's one-line call (passing the wrong `b`) is caught.

use cairn_event::attachment::{Attachment, Rendition};
use cairn_event::{blob_address, generate_key, sign, EventBody, Hlc};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

async fn setup(c: &Client) -> (cairn_event::SigningKey, String) {
    c.batch_execute("TRUNCATE event_log, actor_event, blob_store, blob_chunk CASCADE")
        .await
        .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid]).await.unwrap();
    (sk, kid)
}

/// Author a note.added-style event carrying `attachments`, through the real submit door.
async fn submit_with_attachments(
    db_: &mut Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    node: &str,
    atts: Vec<Attachment>,
) {
    let h = db::next_hlc(db_, node).await.unwrap();
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: Uuid::now_v7().to_string(),
        event_type: "note.added".into(), // registered fail-closed type, allows attachments
        schema_version: "advisory/1".into(),
        hlc: h,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "see attachment"}),
        attachments: atts,
        plaintext_twin: Some("note with attachment".into()),
    };
    let signed = sign(&body, sk).unwrap();
    db_.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap();
}

/// A plausible HLC wall (ms since epoch, ≈ 2026) for a remote event, so the apply door's
/// t_recorded ceiling compares against a sane instant (mirrors apply_remote_event.rs).
const WALL_2026: i64 = 1_782_000_000_000;

/// Author a note.added event "arriving from a peer" through the real remote-apply door (db/020).
/// Unlike the submit helper it does NOT tick the local clock — a replicated event carries its
/// origin node's HLC, which the apply door merges forward.
async fn apply_with_attachments(
    c: &Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    atts: Vec<Attachment>,
) {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: Uuid::now_v7().to_string(),
        event_type: "note.added".into(),
        schema_version: "advisory/1".into(),
        hlc: Hlc {
            wall: WALL_2026,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "see attachment"}),
        attachments: atts,
        plaintext_twin: Some("note with attachment".into()),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap();
}

#[tokio::test]
async fn by_reference_rendition_registers_a_blob_reference_row() {
    let Some(base) = cs() else {
        eprintln!("skip: no CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap(); // conn string; hold until drop
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;

    let r = Rendition::reference("original", b"jpegbytes", "image/jpeg");
    let att = Attachment::single("id photo", r);
    submit_with_attachments(&mut c, &sk, &kid, "n", vec![att]).await;

    let addr = blob_address(b"jpegbytes");
    let row = c
        .query_one(
            "SELECT media_type, byte_len, present FROM blob_store WHERE blob_address = $1",
            &[&addr],
        )
        .await
        .unwrap();
    let mt: String = row.get(0);
    let len: i64 = row.get(1);
    let present: bool = row.get(2);
    assert_eq!(mt, "image/jpeg");
    assert_eq!(len, 9);
    assert!(
        !present,
        "reference-eager, byte-lazy: the row is a reference only"
    );
}

#[tokio::test]
async fn inline_rendition_registers_no_blob_reference() {
    let Some(base) = cs() else {
        eprintln!("skip: no CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap(); // conn string; hold until drop
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;

    let mut r = Rendition::reference("original", b"tiny", "image/png");
    r.inline = Some(serde_bytes::ByteBuf::from(b"tiny".to_vec())); // bytes ride the event
    let att = Attachment::single("tiny inline sketch", r);
    submit_with_attachments(&mut c, &sk, &kid, "n", vec![att]).await;

    let n: i64 = c
        .query_one("SELECT count(*) FROM blob_store", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 0,
        "an inline rendition's bytes are in the event; no lazy blob reference"
    );
}

#[tokio::test]
async fn apply_door_learns_a_blob_reference_per_by_reference_rendition() {
    // Directly exercise the db/020 remote-apply call site of cairn_learn_attachment_refs: an
    // event arriving BY SYNC with a by-reference rendition must register the same reference-only
    // (present=FALSE) blob row the submit door learns. Closes the coverage gap the module note
    // previously named (only the submit door was exercised before).
    let Some(base) = cs() else {
        eprintln!("skip: no CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap(); // conn string; hold until drop
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;

    let r = Rendition::reference("original", b"remote-jpeg-bytes", "image/jpeg");
    let att = Attachment::single("id photo arriving by sync", r);
    apply_with_attachments(&c, &sk, &kid, vec![att]).await;

    let addr = blob_address(b"remote-jpeg-bytes");
    let row = c
        .query_one(
            "SELECT media_type, byte_len, present FROM blob_store WHERE blob_address = $1",
            &[&addr],
        )
        .await
        .unwrap();
    let mt: String = row.get(0);
    let len: i64 = row.get(1);
    let present: bool = row.get(2);
    assert_eq!(mt, "image/jpeg");
    assert_eq!(len, 17);
    assert!(!present, "reference-eager, byte-lazy at the apply door too");
}
