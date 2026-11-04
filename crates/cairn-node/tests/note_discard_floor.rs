//! Sign-in slice 3b-2b, Task 2 — the `clinical.note-discard.asserted` stream end to end below
//! the window: a discard built by `cairn_event::note_discard` is sealed, signed by a HUMAN author
//! (ADR-0053), admitted by the strict door's floor (`db/068`'s `cairn_check_note_discard`), and
//! projected in the clear into `clinical_note_discard` (and NEVER into `clinical_note`); a shred
//! then scrubs that row (db/037).
//!
//! `db/tests/068_clinical_note_discard_test.sql` pins the floor rule by rule; this file proves the
//! real pipeline produces a body the floor admits, and that a hostile payload is refused at the
//! door with the floor's own sentence.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::note::{EscapedNarrative, StatedTime};
use cairn_event::note_discard::{
    discard_payload, render_discard_twin, DiscardedPart, DiscardedWriter, NOTE_DISCARD_EVENT_TYPE,
    NOTE_DISCARD_SCHEMA_VERSION,
};
use cairn_event::{EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::sealed_submit::seal_sign_submit;
use cairn_node::medication::AuthorParams;
use cairn_node::shred::shred_event;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;

const DISCARD_INSTANT: &str = "2026-01-02T03:10:00Z";

/// A device-shaped clear discard body (`seal_sign_submit` rewrites it to human authorship).
fn discard_body(
    event_id: Uuid,
    patient: Uuid,
    node_kid: &str,
    hlc: cairn_event::Hlc,
    payload: serde_json::Value,
    twin: String,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: NOTE_DISCARD_EVENT_TYPE.into(),
        schema_version: NOTE_DISCARD_SCHEMA_VERSION.into(),
        hlc,
        t_effective: Some(DISCARD_INSTANT.into()),
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

struct Node {
    sk: SigningKey,
    kid: String,
}

/// The two parts a window would discard: one by the discarder, one whose writer is unknown.
fn two_parts<'a>() -> Vec<DiscardedPart<'a>> {
    vec![
        DiscardedPart {
            writer: DiscardedWriter::Discarder,
            narrative: EscapedNarrative::of_typed("BP <90 on arrival"),
            stated: StatedTime::NotStated,
            recording_started: "2026-01-02T03:04:05Z",
            recording_ended: "2026-01-02T03:09:05Z",
        },
        DiscardedPart {
            writer: DiscardedWriter::NotKnown,
            narrative: EscapedNarrative::of_typed("[L] arm (R)"),
            stated: StatedTime::Stated("2026-01-02T02:00:00+10:00"),
            recording_started: "2026-01-02T03:09:06Z",
            recording_ended: "2026-01-02T03:09:30Z",
        },
    ]
}

/// A registered chart; submit a discard of `payload`/`twin` on it through the strict door.
async fn submit_discard(
    c: &mut Client,
    payload: serde_json::Value,
    twin: String,
) -> (Node, Uuid, anyhow::Result<Uuid>) {
    let (node_sk, node_kid, human_sk, human_kid) = common::medication_setup(c).await;
    let patient = Uuid::now_v7();
    // #345: a chart's first event is its registration.
    common::submit_registration(c, &node_sk, &node_kid, patient, 0).await;
    let hlc = db::next_hlc(c, &node_kid).await.unwrap();
    let event_id = Uuid::now_v7();
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let r = seal_sign_submit(
        c,
        &node_sk,
        discard_body(event_id, patient, &node_kid, hlc, payload, twin),
        Some(&author),
        None,
    )
    .await;
    (
        Node {
            sk: node_sk,
            kid: node_kid,
        },
        event_id,
        r,
    )
}

async fn rows(c: &Client, table: &str, id: Uuid) -> i64 {
    c.query_one(
        &format!("SELECT count(*) FROM {table} WHERE event_id = $1::text::uuid"),
        &[&id.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn a_sealed_discard_submits_and_projects_its_parts_not_a_note() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let parts = two_parts();
    let payload = discard_payload(&parts);
    let (_, id, r) = submit_discard(&mut c, payload.clone(), render_discard_twin(&parts)).await;
    assert_eq!(
        r.unwrap_or_else(|e| panic!(
            "the strict door admits a discard the window would write: {e:#}"
        )),
        id
    );

    let row = c
        .query_one(
            "SELECT parts::text, t_effective = $2::text::timestamptz \
               FROM clinical_note_discard WHERE event_id = $1::text::uuid",
            &[&id.to_string(), &DISCARD_INSTANT],
        )
        .await
        .expect("one projected row per discard");
    let stored: serde_json::Value = serde_json::from_str(&row.get::<_, String>(0)).unwrap();
    assert_eq!(
        stored, payload["parts"],
        "the parts as signed, escaped narratives and all"
    );
    assert!(
        row.get::<_, bool>(1),
        "t_effective is the envelope's discard instant"
    );
    assert_eq!(
        rows(&c, "clinical_note", id).await,
        0,
        "a discard is never a note"
    );

    let sealed: bool = c
        .query_one(
            "SELECT body::text NOT LIKE '%arrival%' FROM event_log WHERE event_id = $1::text::uuid",
            &[&id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(sealed, "the discard is born sealed (ADR-0052)");
}

#[tokio::test]
async fn the_strict_door_refuses_a_hostile_discard_with_the_floors_own_sentence() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    // A raw-SQL-style client skips the escape: a live tag in a part's narrative.
    let payload = serde_json::json!({"parts": [{
        "writer": "discarder",
        "narrative": "pt said <b>x</b>",
        "stated_time": null,
        "recording_started": "2026-01-02T03:04:05Z",
        "recording_ended": "2026-01-02T03:09:05Z",
    }]});
    let (_, id, r) = submit_discard(&mut c, payload, "pt said <b>x</b>".into()).await;
    let msg = format!("{:#}", r.expect_err("raw HTML in a discard is refused"));
    assert!(msg.contains("raw HTML"), "the markup rule spoke: {msg}");
    assert_eq!(rows(&c, "clinical_note_discard", id).await, 0);
}

#[tokio::test]
async fn a_shred_scrubs_the_discard_projection() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let parts = two_parts();
    let (node, id, r) =
        submit_discard(&mut c, discard_payload(&parts), render_discard_twin(&parts)).await;
    r.expect("admitted");
    assert_eq!(rows(&c, "clinical_note_discard", id).await, 1);

    shred_event(
        &mut c,
        &node.sk,
        &node.kid,
        &node.kid,
        id,
        "retention ceiling",
        None,
    )
    .await
    .expect("the shred is admitted");
    assert_eq!(
        rows(&c, "clinical_note_discard", id).await,
        0,
        "the parts are clinical plaintext: the shred scrubs their row"
    );
}
