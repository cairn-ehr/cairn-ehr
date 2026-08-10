//! Raising is free; lowering is a ceremony — and the ceremony is a LOCAL-AUTHORING rule.
//!
//! The asymmetry is tested, never merely commented (the Slice 64 pattern). A door check at
//! apply would let a peer's rationale-less act be refused, forking the event set and
//! wedging replication (ADR-0060, the #342 trap). For a RAISE the asymmetry is doubly
//! forced: refusing a peer's protective assertion would leave this node computing a LOWER
//! grade than the peer's, so the refusal is itself a disclosure.
mod common;
use cairn_event::sensitivity::*;
use cairn_event::{sign, ClockGrade, EventBody, Hlc};
use common::{cs, db_msg, setup, submit_registration, submit_signed, EventSpec};
use serde_json::json;
use tokio_postgres::error::SqlState;
use uuid::Uuid;

/// The same rationale-less chart-wide raise `the_local_door_requires_a_rationale_for_a_chart_wide_raise`
/// exercises at the LOCAL door, built by hand as a PEER's event (`node_origin: "peer"`) for
/// the remote door instead. `submit_signed` only ever drives `submit_event` (the local
/// door), so the remote-door test cannot reuse it — this mirrors the `peer_dob` idiom
/// `patient_precedence.rs` already uses for the identical local-vs-remote asymmetry.
fn peer_chart_wide_raise(kid: &str, p: Uuid, wall: i64) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: SENSITIVITY_EVENT_TYPE.into(),
        schema_version: SENSITIVITY_SCHEMA_VERSION.into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: json!([{"actor_id": kid, "role": "recorded"}]),
        payload: json!({
            "subject_kind": "patient", "subject_id": p.to_string(),
            "grade": "restricted", "source": "human"
        }),
        attachments: vec![],
        plaintext_twin: Some("chart-wide".into()),
        clock_grade: ClockGrade::SelfAsserted,
    }
}

#[tokio::test]
async fn the_local_door_requires_a_rationale_for_a_chart_wide_raise() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // A THREAD raise needs no ceremony — raising must stay frictionless.
    let a = SensitivityAssertion {
        subject_kind: SubjectKind::Thread,
        subject_id: Uuid::now_v7(),
        grade: "restricted",
        source: "human",
        rationale: None,
    };
    submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: sensitivity_assertion_body(&a),
            plaintext_twin: Some(render_sensitivity_twin(&a)),
            wall: 10,
        },
    )
    .await
    .expect("a thread raise carries no ceremony");

    // A CHART-WIDE raise does: it is the one act whose blast radius is the whole record.
    let err = submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: json!({
                "subject_kind": "patient", "subject_id": p.to_string(),
                "grade": "restricted", "source": "human"
            }),
            plaintext_twin: Some("chart-wide".into()),
            wall: 11,
        },
    )
    .await
    .expect_err("a chart-wide raise with no rationale must be refused locally");

    // db_msg only carries the message text, never the SQLSTATE (see its doc comment in
    // common/mod.rs) — the deliberate-refusal code is read straight off the driver error.
    let code = err
        .as_db_error()
        .expect("the refusal must be a database error, not a transport failure")
        .code()
        .code()
        .to_string();
    let msg = db_msg(&err);
    assert_eq!(
        code,
        SqlState::RAISE_EXCEPTION.code(),
        "deliberate refusal: {msg}"
    );
    assert!(
        msg.contains("rationale"),
        "the refusal names what would repair it: {msg}"
    );
}

#[tokio::test]
async fn the_remote_door_admits_what_the_local_door_refuses() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // The same rationale-less chart-wide raise, arriving from a peer. It MUST apply: a
    // refusal would both wedge replication and leave us less protected than the peer.
    let signed = sign(&peer_chart_wide_raise(&kid, p, 12), &sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("the remote door is lenient BY DESIGN");

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM sensitivity_assertion WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the peer's protective assertion stands here too");
}
