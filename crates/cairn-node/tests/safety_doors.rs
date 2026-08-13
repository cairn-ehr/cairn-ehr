//! §5.9 part B (ADR-0063) — the DELIBERATE door asymmetry.
//!
//! The local door refuses a self-contradictory safety signal; the remote door admits it AND
//! the clinical content it rides on. The second half is the one that matters: a defect in a
//! de-identified advisory field must never cancel a clinical event (ADR-0060).
mod common;
use common::{cs, db_msg, setup};
use uuid::Uuid;

/// Build a signed `note.added` body carrying `safety` verbatim, so a test can put a shape on
/// the wire that no honest authoring path would produce.
///
/// `note.added` rather than a medication verb on purpose: it is unsealed, so the test needs
/// no DEK and exercises the ENVELOPE-level check without the seal path in the way.
fn body_with_safety(
    patient: Uuid,
    kid: &str,
    wall: i64,
    safety: Option<serde_json::Value>,
) -> cairn_event::EventBody {
    cairn_event::EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: cairn_event::Hlc { wall, counter: 0, node_origin: "n1".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "a note"}),
        attachments: vec![],
        plaintext_twin: Some("a note".into()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety,
    }
}

#[tokio::test]
async fn the_local_door_refuses_a_class_the_rung_does_not_license() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let patient = Uuid::now_v7();
    // MY RULING P2 (issue #345, db/005 step 8b): the first event on a chart must be its
    // registration, or submit_event refuses it on the PRECEDENCE rule rather than on
    // whatever this test is actually exercising. Wall 0 keeps the chart's birth act below
    // the suite's own events (wall 1 below).
    common::submit_registration(&c, &sk, &kid, patient, 0).await;

    let body = body_with_safety(
        patient,
        &kid,
        1,
        Some(serde_json::json!({"rung": "existence", "class": "rh-sensitizing"})),
    );
    let signed = cairn_event::sign(&body, &sk).expect("signs");
    let e = c
        .execute("SELECT submit_event($1, NULL, NULL, NULL)", &[&signed.signed_bytes])
        .await
        .expect_err("the local door must refuse a self-contradictory signal");
    let msg = db_msg(&e);
    assert!(msg.contains("class"), "the refusal names the offending key: {msg}");
}

#[tokio::test]
async fn the_remote_door_admits_the_same_body_and_keeps_the_clinical_content() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let patient = Uuid::now_v7();
    // No registration needed here: apply_remote_event (the sync door) never runs db/005's
    // step 8b precedence rule — that is the whole point of this test file's asymmetry.

    let body = body_with_safety(
        patient,
        &kid,
        2,
        Some(serde_json::json!({"rung": "existence", "class": "rh-sensitizing"})),
    );
    let event_id: Uuid = body.event_id.parse().expect("uuid");
    let signed = cairn_event::sign(&body, &sk).expect("signs");
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("the remote door must ADMIT it — refusing forks the event set (#342)");

    // The half that actually matters: the clinical content landed.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id.to_string()],
        )
        .await
        .expect("query")
        .get(0);
    assert_eq!(
        n, 1,
        "a defect in a de-identified advisory field must never cancel clinical content (ADR-0060)"
    );

    // And the column is a FAITHFUL derived view — never sanitized on the way in, which
    // would make it disagree with signed_bytes. Section 7's read model is what refuses to
    // ACT on the contradiction.
    //
    // Cast to ::text, not bound as jsonb directly: tokio-postgres has no `FromSql` for
    // `serde_json::Value` without the `with-serde_json-1` feature, which this crate does
    // not enable (no new dependency features — see observed_evidence.rs's identical idiom).
    let stored_text: Option<String> = c
        .query_one(
            "SELECT safety::text FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id.to_string()],
        )
        .await
        .expect("query")
        .get(0);
    let stored: serde_json::Value =
        serde_json::from_str(&stored_text.expect("the signal is stored")).expect("valid json");
    assert_eq!(stored["rung"], "existence");
    assert_eq!(stored["class"], "rh-sensitizing", "stored verbatim, not scrubbed");
}

#[tokio::test]
async fn a_well_formed_signal_lands_in_the_column_through_the_local_door() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let patient = Uuid::now_v7();
    // MY RULING P2 (issue #345, db/005 step 8b): see the first test in this file.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;

    let body = body_with_safety(
        patient,
        &kid,
        3,
        Some(serde_json::json!({"rung": "kind", "severity": "high"})),
    );
    let event_id: Uuid = body.event_id.parse().expect("uuid");
    let signed = cairn_event::sign(&body, &sk).expect("signs");
    c.execute("SELECT submit_event($1, NULL, NULL, NULL)", &[&signed.signed_bytes])
        .await
        .expect("a well-formed signal is admitted");

    // Cast to ::text — see the remote-door test above for why.
    let stored_text: Option<String> = c
        .query_one(
            "SELECT safety::text FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id.to_string()],
        )
        .await
        .expect("query")
        .get(0);
    let stored: serde_json::Value =
        serde_json::from_str(&stored_text.expect("stored")).expect("valid json");
    assert_eq!(stored["severity"], "high");
}

#[tokio::test]
async fn an_event_with_no_signal_stores_null() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let patient = Uuid::now_v7();
    // MY RULING P2 (issue #345, db/005 step 8b): see the first test in this file.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;

    let body = body_with_safety(patient, &kid, 4, None);
    let event_id: Uuid = body.event_id.parse().expect("uuid");
    let signed = cairn_event::sign(&body, &sk).expect("signs");
    c.execute("SELECT submit_event($1, NULL, NULL, NULL)", &[&signed.signed_bytes])
        .await
        .expect("no signal is the common case");

    // Cast to ::text — see the remote-door test above for why.
    let stored_text: Option<String> = c
        .query_one(
            "SELECT safety::text FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id.to_string()],
        )
        .await
        .expect("query")
        .get(0);
    assert!(stored_text.is_none(), "absence stays absence, never an empty object");
}
