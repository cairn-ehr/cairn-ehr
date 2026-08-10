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

/// A structurally well-formed withdrawal (non-empty rationale, valid hex `withdraws`) as a
/// PEER's event, for the remote door — the withdrawal twin of [`peer_chart_wide_raise`], and
/// built the same way for the same reason: `submit_signed` only ever drives the local door.
fn peer_withdrawal(kid: &str, p: Uuid, withdraws_hex: &str, wall: i64) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: WITHDRAWAL_EVENT_TYPE.into(),
        schema_version: WITHDRAWAL_SCHEMA_VERSION.into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: json!([{"actor_id": kid, "role": "recorded"}]),
        payload: json!({ "withdraws": withdraws_hex, "rationale": "patient consent" }),
        attachments: vec![],
        plaintext_twin: Some("withdrawn".into()),
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
    // "rationale" alone does not pin WHICH refusal fired (review finding F4): the db/048
    // structural floor's withdrawal-rationale message also contains the word "rationale".
    // "chart-wide" is unique to this ceremony's raise refusal.
    assert!(
        msg.contains("chart-wide"),
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

// ===========================================================================
// Review round 1, finding F1: the raise half of the ceremony was pinned above, but the
// WITHDRAWAL half (the bound-human-author requirement) had no failing test behind it —
// deleting `cairn_sensitivity_ceremony_ok`'s second `IF` block left every test in this
// crate green, because `sensitivity_ceremony.rs` never submitted a withdrawal at all and
// the pre-existing suites this task fixed only ever SATISFY the gate. These two tests are
// that half's local-refuses / remote-admits pair, mirroring the raise pair above.
// ===========================================================================

#[tokio::test]
async fn the_local_door_requires_a_bound_human_author_for_a_withdrawal() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // Structurally well-formed — a non-empty rationale and a syntactically valid hex
    // `withdraws` value, so db/048's structural floor (section 4) is satisfied — but
    // authored by the plain agent signer with an un-attested `recorded` contributor: no
    // bound human author. It is the CEREMONY, not the structural floor, that must refuse
    // this, so the refusal has to name authorship, not the rationale.
    let ghost = "aa".repeat(34); // a syntactically valid multihash-shaped hex value
    let err = submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: WITHDRAWAL_EVENT_TYPE,
            schema_version: WITHDRAWAL_SCHEMA_VERSION,
            payload: json!({ "withdraws": ghost, "rationale": "patient consent" }),
            plaintext_twin: Some("withdrawn".into()),
            wall: 10,
        },
    )
    .await
    .expect_err("a withdrawal with no bound human author must be refused locally");

    let msg = db_msg(&err);
    assert!(
        msg.contains("bound human author"),
        "the refusal names what would repair it: {msg}"
    );
}

#[tokio::test]
async fn the_remote_door_admits_a_withdrawal_the_local_door_refuses() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // The identical un-attested withdrawal, arriving from a peer. It MUST apply: the
    // human-author ceremony is a LOCAL-authoring rule (db/048's header), so a peer's
    // honestly-authored-under-a-different-policy withdrawal must not fork the event set.
    let ghost = "bb".repeat(34);
    let signed = sign(&peer_withdrawal(&kid, p, &ghost, 10), &sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("the remote door is lenient BY DESIGN");

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM sensitivity_withdrawal WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the peer's withdrawal stands here too");
}
