//! The db/048 structural floor. Every rule is a judgement about the SHAPE of the claim, so
//! it is safe at BOTH doors — a peer that produced one of these shapes produced something
//! no conformant door of any version could have minted.
mod common;
use cairn_event::sensitivity::*;
use common::{cs, db_msg, setup, submit_registration, submit_signed, EventSpec};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn the_floor_refuses_a_malformed_assertion_and_admits_a_well_formed_one() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let p = Uuid::now_v7();
    // The precedence rule (#345): a chart's FIRST event must be its registration, and a
    // sensitivity assertion bears patient_id — so the chart is registered first.
    submit_registration(&c, &sk, &kid, p, 1).await;

    // Well-formed: accepted.
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
    .expect("a well-formed assertion is accepted");

    // A non-uuid subject_id is refused, legibly.
    let err = submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: json!({
                "subject_kind": "thread", "subject_id": "not-a-uuid",
                "grade": "restricted", "source": "human"
            }),
            plaintext_twin: Some("x".into()),
            wall: 11,
        },
    )
    .await
    .expect_err("a non-uuid subject_id must be refused");
    let err = db_msg(&err);
    assert!(
        err.contains("subject_id"),
        "the refusal names the field: {err}"
    );

    // A blank grade is refused: "" would rank MAX and coarsen everything, so it looks safe
    // while being a shape no author meant to write.
    let err = submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: json!({
                "subject_kind": "thread", "subject_id": Uuid::now_v7().to_string(),
                "grade": "  ", "source": "human"
            }),
            plaintext_twin: Some("x".into()),
            wall: 12,
        },
    )
    .await
    .expect_err("a blank grade must be refused");
    let err = db_msg(&err);
    assert!(err.contains("grade"), "the refusal names the field: {err}");
}

#[tokio::test]
async fn an_unknown_subject_kind_is_admitted_because_the_floor_gates_effect_not_presence() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // A future peer's `episode` subject must be ADMITTED (ADR-0056) — a closed CHECK here
    // would wedge the apply door on honest traffic. Task 5 pins that it is then INTERPRETED
    // conservatively, as chart-wide.
    submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: json!({
                "subject_kind": "episode", "subject_id": Uuid::now_v7().to_string(),
                "grade": "restricted", "source": "human"
            }),
            plaintext_twin: Some("future kind".into()),
            wall: 10,
        },
    )
    .await
    .expect("an unknown subject_kind is admitted, not refused");
}

#[tokio::test]
async fn a_malformed_withdraws_hex_fails_legibly_with_p0001() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    // Call the floor check DIRECTLY rather than through submit_event: `common::db_msg`
    // returns only `message()`, and the SQLSTATE is the entire point of this test. Same
    // approach, and the same reasoning, as crates/cairn-node/tests/hex_decode_helper.rs —
    // read its section 2 before changing this.
    // $1::text::jsonb, not a bare $1::jsonb: with a bare ::jsonb cast, Postgres infers OID
    // jsonb for $1, and tokio-postgres's ToSql for a String only accepts TEXT/VARCHAR/NAME/
    // UNKNOWN — so binding fails CLIENT-SIDE as a WrongType transport error before the query
    // ever reaches the server, which would satisfy `.expect_err` for the wrong reason and
    // never exercise the P0001 path this test exists to pin. Same idiom, same reasoning, as
    // dispatch_runs_the_registered_structural_check in twin_registry.rs.
    let body = json!({ "payload": { "withdraws": "0xNOTHEX", "rationale": "consent" } });
    let err = c
        .query_one(
            "SELECT cairn_check_sensitivity_withdrawal(
                 'sensitivity.grade-withdrawal.asserted', $1::text::jsonb)",
            &[&body.to_string()],
        )
        .await
        .expect_err("a malformed hex value must be refused");
    let db = err
        .as_db_error()
        .expect("the refusal must be a database error, not a transport failure");

    // P0001 is a CONTRACT with cairn-sync's pull loop: it means "deliberate, skip and
    // re-offer". Any other SQLSTATE is read as a transient fault, which FREEZES the cursor
    // and stalls sync from that peer forever — the #228 defect. A message-only assertion
    // would stay green through a well-meaning `USING ERRCODE = SQLSTATE`.
    assert_eq!(db.code().code(), "P0001", "message was: {}", db.message());
    assert!(
        db.message().contains("withdraws"),
        "the refusal names the field: {}",
        db.message()
    );
}

#[tokio::test]
async fn an_assertion_projects_and_a_withdrawal_projects_independently_of_arrival_order() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // The withdrawal is authored FIRST, naming an assertion that does not exist yet. Set-
    // union sync has no ordering, so this is normal traffic, and no FK may forbid it.
    let ghost = "aa".repeat(34); // a syntactically valid multihash-shaped hex value
    submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: WITHDRAWAL_EVENT_TYPE,
            schema_version: WITHDRAWAL_SCHEMA_VERSION,
            payload: json!({ "withdraws": ghost, "rationale": "consent" }),
            plaintext_twin: Some("withdrawn".into()),
            wall: 10,
        },
    )
    .await
    .expect("a withdrawal naming an unseen assertion must be accepted");

    let rows: i64 = c
        .query_one(
            "SELECT count(*) FROM sensitivity_withdrawal WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        rows, 1,
        "the withdrawal projects even with no target present"
    );

    let a = SensitivityAssertion {
        subject_kind: SubjectKind::Patient,
        subject_id: p,
        grade: "sensitive",
        source: "human",
        rationale: Some("staff member treated here"),
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
    .expect("assertion accepted");

    let row = c
        .query_one(
            "SELECT subject_kind, grade, source FROM sensitivity_assertion
              WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .map_err(|e| db_msg(&e))
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "patient");
    assert_eq!(row.get::<_, String>(1), "sensitive");
    assert_eq!(row.get::<_, String>(2), "human");
}
