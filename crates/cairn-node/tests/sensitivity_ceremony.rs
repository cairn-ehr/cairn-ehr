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

/// A chart-wide (`subject_kind: "patient"`) raise built by hand as a PEER's event
/// (`node_origin: "peer"`) for the REMOTE door. `submit_signed` only ever drives
/// `submit_event` (the local door), so a remote-door test cannot reuse it — this mirrors
/// the `peer_dob` idiom `patient_precedence.rs` already uses for the identical
/// local-vs-remote asymmetry.
///
/// `subject` is separate from `p` on purpose: with `subject == p` this is the rationale-less
/// raise the local door refuses for want of a rationale, and with `subject != p` it is the
/// MIS-TARGETED raise the local door refuses for naming another chart. Both must be admitted
/// remotely, and one helper covering both keeps that pairing visible.
fn peer_chart_wide_raise(kid: &str, p: Uuid, subject: Uuid, wall: i64) -> EventBody {
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
            "subject_kind": "patient", "subject_id": subject.to_string(),
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
    // "rationale" alone does not pin WHICH refusal fired: the db/048 structural floor's
    // withdrawal-rationale message also contains the word "rationale".
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
    let signed = sign(&peer_chart_wide_raise(&kid, p, p, 12), &sk).unwrap();
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
// A chart-wide raise that names ANOTHER chart: the silent-failure half of a mis-target.
//
// The read model (db/048 section 11) already makes such an assertion coarsen the chart it
// was AUTHORED on, so the over-protecting half is covered. What it cannot do is anything at
// all for the chart the author MEANT to seal: `sensitivity-assert --patient A
// --subject-kind patient --subject-id B` is two hand-typed UUIDs, and chart B goes on
// reading `routine` forever with nothing anywhere surfacing the mismatch. "The clinician
// believes they sealed a chart and did not" is unrecoverable, so the local authoring door
// refuses it — and, being a local ceremony rule, the remote door still admits it (a refusal
// there would fork the event set and, for a protective act, be a disclosure in itself).
// ===========================================================================

#[tokio::test]
async fn the_local_door_refuses_a_chart_wide_grade_that_names_another_chart() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    let other = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // A rationale IS supplied, so only the mis-target rule can fire — this pins the new
    // refusal rather than accidentally re-testing the rationale one.
    let err = submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: json!({
                "subject_kind": "patient", "subject_id": other.to_string(),
                "grade": "restricted", "source": "human",
                "rationale": "staff member treated here"
            }),
            plaintext_twin: Some("chart-wide".into()),
            wall: 10,
        },
    )
    .await
    .expect_err("a chart-wide grade naming a different chart must be refused locally");

    let msg = db_msg(&err);
    assert!(
        msg.contains("must name THIS chart"),
        "the refusal names what would repair it: {msg}"
    );
    // The mis-typed value itself is in the message: the operator has to see WHICH of the two
    // hand-typed UUIDs was wrong to fix it.
    assert!(
        msg.contains(&other.to_string()) && msg.contains(&p.to_string()),
        "the refusal names both the offered subject and this chart: {msg}"
    );

    // A thread-scoped grade naming something other than this chart is NOT affected — a
    // thread id is not a patient id, and narrowing that would break every thread raise.
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
            wall: 11,
        },
    )
    .await
    .expect("only a 'patient'-kind subject is pinned to the envelope");
}

#[tokio::test]
async fn the_remote_door_admits_a_chart_wide_grade_that_names_another_chart() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    let other = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // The identical mis-targeted body, arriving from a peer. It MUST apply: this rule lives
    // in the local ceremony and NOT in db/020 precisely so a peer's event can never be
    // refused at apply and wedge replication (ADR-0060) — and the row it writes is what
    // db/048 section 11's catch-all arm then coarsens this chart with.
    let signed = sign(&peer_chart_wide_raise(&kid, p, other, 12), &sk).unwrap();
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
    assert_eq!(n, 1, "the peer's mis-targeted assertion stands here too");
}

// ===========================================================================
// The WITHDRAWAL half of the ceremony (the bound-human-author requirement) had no failing
// test behind it at first: deleting `cairn_sensitivity_ceremony_ok`'s second `IF` block left
// every test in this crate green, because this file never submitted a withdrawal at all and
// the pre-existing suites only ever SATISFY the gate. These two tests are that half's
// local-refuses / remote-admits pair, mirroring the raise pairs above.
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

/// THE MIS-TARGET RULE COVERS ALL THREE SUBJECT KINDS, NOT JUST `patient`.
///
/// The chart-wide rule ("a 'patient' grade must name THIS chart") was argued from the fact
/// that `--patient` and `--subject-id` are two hand-typed UUIDs and a mis-typed pair fails in
/// BOTH directions at once. That argument transfers unchanged to `event` and `thread`, and
/// for a long time only `patient` was covered. Section 11's catch-all can only ever fix the
/// over-protecting half (this chart coarsens); the other half — the event or thread the
/// author MEANT to grade silently staying 'routine', because the assertion carries THIS
/// chart's patient_id and the standing set is patient-scoped — is undetectable afterwards.
///
/// The predicate is "known here AND demonstrably on another chart", never "not known to be
/// here", so an honest out-of-order write against a not-yet-replicated target is never
/// refused. That half is pinned by the second block below.
#[tokio::test]
async fn the_local_door_refuses_an_event_or_thread_grade_targeting_another_chart() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let chart_a = Uuid::now_v7();
    let chart_b = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, chart_a, 1).await;
    submit_registration(&c, &sk, &kid, chart_b, 1).await;

    let note_b = Uuid::now_v7();
    common::submit_signed_with_id(
        &c,
        &sk,
        &kid,
        note_b,
        EventSpec {
            patient: chart_b,
            event_type: "note.added",
            schema_version: "note.added/1",
            payload: json!({ "text": "b" }),
            plaintext_twin: Some("b".into()),
            wall: 2,
        },
    )
    .await
    .expect("note on chart B accepted");

    // An 'event' grade authored on chart A naming chart B's event: refused, by name.
    let err = submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: chart_a,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: json!({
                "subject_kind": "event", "subject_id": note_b.to_string(),
                "grade": "restricted", "source": "human"
            }),
            plaintext_twin: Some("mis-targeted event grade".into()),
            wall: 3,
        },
    )
    .await
    .expect_err("the event named is demonstrably on another chart");
    let err = db_msg(&err);
    assert!(
        err.contains("not this chart"),
        "the refusal must say which chart the target is really on: {err}"
    );

    // ARRIVAL-ORDER INDEPENDENCE: the SAME shape against a target this node has never seen is
    // ADMITTED. Set-union sync has no ordering, so an event-scoped grade legitimately precedes
    // the event it names; a rule that fired on "not found" would refuse honest traffic and —
    // on a custody-less node, where nothing resolves — refuse nearly all of it.
    submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: chart_a,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: json!({
                "subject_kind": "event", "subject_id": Uuid::now_v7().to_string(),
                "grade": "restricted", "source": "human"
            }),
            plaintext_twin: Some("not yet replicated".into()),
            wall: 4,
        },
    )
    .await
    .expect("a target that has not arrived yet must NOT be treated as a mis-target");
}

/// The remote half of the rule above: a peer's mis-targeted event grade is ADMITTED.
///
/// Same ADR-0060 reasoning as every other rule in this file — a door check at apply lets one
/// peer's honest act be refused by another peer's stricter node, forking the event set. For a
/// RAISE it is worse than a wedge: refusing a peer's protective assertion leaves THIS node
/// computing a LOWER grade than the peer already holds, so the refusal is itself a disclosure.
#[tokio::test]
async fn the_remote_door_admits_an_event_grade_targeting_another_chart() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let chart_a = Uuid::now_v7();
    let chart_b = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, chart_a, 1).await;
    submit_registration(&c, &sk, &kid, chart_b, 1).await;

    let note_b = Uuid::now_v7();
    common::submit_signed_with_id(
        &c,
        &sk,
        &kid,
        note_b,
        EventSpec {
            patient: chart_b,
            event_type: "note.added",
            schema_version: "note.added/1",
            payload: json!({ "text": "b" }),
            plaintext_twin: Some("b".into()),
            wall: 2,
        },
    )
    .await
    .expect("note on chart B accepted");

    let peer = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: chart_a.to_string(),
        event_type: SENSITIVITY_EVENT_TYPE.into(),
        schema_version: SENSITIVITY_SCHEMA_VERSION.into(),
        hlc: Hlc {
            wall: 3,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: json!([{"actor_id": kid, "role": "recorded"}]),
        payload: json!({
            "subject_kind": "event", "subject_id": note_b.to_string(),
            "grade": "restricted", "source": "human"
        }),
        attachments: vec![],
        plaintext_twin: Some("peer's mis-targeted event grade".into()),
        clock_grade: ClockGrade::SelfAsserted,
    };
    let signed = sign(&peer, &sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .map_err(|e| db_msg(&e))
        .expect("the remote door must admit what the local door refuses");

    let landed: i64 = c
        .query_one(
            "SELECT count(*) FROM sensitivity_assertion WHERE patient_id = $1::text::uuid",
            &[&chart_a.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        landed, 1,
        "and it must actually project, or the leniency claim is vacuous"
    );
}

/// The category refusal is a LOCAL rule too — a peer that already put the category on the
/// wire has already leaked it, so refusing at apply would un-disclose nothing and would only
/// fork the event set (ADR-0060). This pins that the asymmetry is deliberate rather than an
/// oversight, alongside the local-door refusal in `sensitivity_floor.rs`.
#[tokio::test]
async fn the_remote_door_admits_an_assertion_carrying_a_category() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    let peer = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: SENSITIVITY_EVENT_TYPE.into(),
        schema_version: SENSITIVITY_SCHEMA_VERSION.into(),
        hlc: Hlc {
            wall: 2,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: json!([{"actor_id": kid, "role": "recorded"}]),
        payload: json!({
            "subject_kind": "thread", "subject_id": Uuid::now_v7().to_string(),
            "grade": "restricted", "source": "advisory", "category": "leaked-by-the-peer"
        }),
        attachments: vec![],
        plaintext_twin: Some("peer's leaky assertion".into()),
        clock_grade: ClockGrade::SelfAsserted,
    };
    let signed = sign(&peer, &sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .map_err(|e| db_msg(&e))
        .expect("refusing here would fork the event set without un-disclosing anything");
}
