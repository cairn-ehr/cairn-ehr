//! The db/048 structural floor. Every rule is a judgement about the SHAPE of the claim, so
//! it is safe at BOTH doors — a peer that produced one of these shapes produced something
//! no conformant door of any version could have minted.
mod common;
use cairn_event::sensitivity::*;
use cairn_event::{ClockGrade, EventBody, Hlc};
use common::{
    cs, db_msg, enroll_human, setup, submit_attested, submit_registration, submit_signed, EventSpec,
};
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
        source: cairn_event::sensitivity::Provenance::Human,
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
    //
    // The rationale is supplied because an unrecognised kind READS CHART-WIDE, and the local
    // door now demands a rationale from anything with chart-wide reach (see the test below).
    // That is a ceremony rule, not a structural one: this test is about the STRUCTURAL floor
    // admitting the unknown kind at all, so it satisfies the ceremony rather than exercising
    // it.
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
                "grade": "restricted", "source": "human",
                "rationale": "future peer's episode scope"
            }),
            plaintext_twin: Some("future kind".into()),
            wall: 10,
        },
    )
    .await
    .expect("an unknown subject_kind is admitted, not refused");
}

/// THE CEREMONY IS TIED TO BLAST RADIUS, NOT TO THE SPELLING OF ONE SUBJECT KIND.
///
/// db/048 section 11 gives chart-wide effect to EVERY subject kind it does not recognise.
/// While the ceremony's rationale rule was written as `subject_kind = 'patient'`, any other
/// string — `"chart"`, `"episode"`, anything — bought the full chart-wide blast radius with
/// no rationale and no ceremony, straight through the LOCAL door. The gate and the effect
/// were keyed on different things and only the gate was narrow.
///
/// This pins the inversion that closed it: the rationale is owed unless the kind is one of
/// the two we KNOW is narrowly scoped. A future kind therefore inherits the requirement for
/// free, which is the same safe-default-by-omission discipline `cairn_event_type_has_no_thread`
/// uses.
#[tokio::test]
async fn an_unrecognised_subject_kind_owes_a_rationale_because_it_reads_chart_wide() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    let err = submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: json!({
                "subject_kind": "chart", "subject_id": p.to_string(),
                "grade": "sequestered", "source": "human"
            }),
            plaintext_twin: Some("chart-wide by another name".into()),
            wall: 10,
        },
    )
    .await
    .expect_err("an unrecognised kind reads chart-wide, so it owes a rationale");
    let err = db_msg(&err);
    assert!(
        err.contains("rationale"),
        "the refusal must name what is missing: {err}"
    );

    // And the SAME body with a rationale is admitted — the rule asks for the justification,
    // it does not close the open vocabulary (ADR-0056).
    submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: json!({
                "subject_kind": "chart", "subject_id": p.to_string(),
                "grade": "sequestered", "source": "human",
                "rationale": "whole-record seal, patient request 2026-08-11"
            }),
            plaintext_twin: Some("chart-wide by another name".into()),
            wall: 11,
        },
    )
    .await
    .expect("an unrecognised kind WITH a rationale is still admitted");
}

/// THE CATEGORY MUST NEVER REACH THE WIRE, AND THE FLOOR IS WHAT ENFORCES THAT.
///
/// `cairn-event`'s builder has no `category` field, so nothing Cairn's own code authors can
/// carry one — but a builder is not a floor. ADR-0021 explicitly blesses bespoke UIs, and the
/// twelfth founding principle is that the DATABASE is the layer a client talking raw SQL
/// cannot walk past. These bodies are plaintext and replicate unconditionally, so a payload
/// naming `category` IS the disclosure the grade exists to prevent (ADR-0006 decision 4).
#[tokio::test]
async fn the_local_door_refuses_an_assertion_carrying_a_category() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

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
                "grade": "restricted", "source": "advisory",
                "category": "termination-of-pregnancy"
            }),
            plaintext_twin: Some("graded".into()),
            wall: 10,
        },
    )
    .await
    .expect_err("a body carrying the matched category must be refused at the local door");
    let err = db_msg(&err);
    assert!(
        err.contains("category"),
        "the refusal must name the offending field: {err}"
    );
}

/// The withdrawal's `rationale` is a STRUCTURAL rule, not a ceremony one — registered in the
/// ADR-0048 twin-check registry and dispatched through `cairn_event_twin`, which BOTH doors
/// call. ADR-0062's erratum E2 exists specifically to correct a passage that blurred this, so
/// the rule deserves a test rather than only prose: without one, deleting the check leaves the
/// whole workspace green while a peer's rationale-less withdrawal — the accountability record
/// for REMOVING protection — is admitted forever, unrepairable under append-only.
#[tokio::test]
async fn a_withdrawal_with_a_blank_rationale_is_refused_structurally() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    for (label, payload) in [
        (
            "blank",
            json!({ "withdraws": "aa".repeat(34), "rationale": "   " }),
        ),
        ("absent", json!({ "withdraws": "aa".repeat(34) })),
    ] {
        let err = submit_signed(
            &c,
            &sk,
            &kid,
            EventSpec {
                patient: p,
                event_type: WITHDRAWAL_EVENT_TYPE,
                schema_version: WITHDRAWAL_SCHEMA_VERSION,
                payload,
                plaintext_twin: Some("withdrawn".into()),
                wall: 10,
            },
        )
        .await
        .unwrap_err();
        let err = db_msg(&err);
        assert!(
            err.contains("rationale"),
            "a {label} rationale must be refused by name: {err}"
        );
    }
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
    //
    // Task 6's ceremony (db/048 `cairn_sensitivity_ceremony_ok`, called from db/005) added
    // a second local-door requirement on top of the structural floor's rationale: a
    // withdrawal now also needs a BOUND HUMAN AUTHOR — a contributor claiming
    // responsibility, verified by attestation (ADR-0053). That authorship gate is what
    // `sensitivity_ceremony.rs` exists to pin; this test is about arrival-order
    // independence, so it only needs to SATISFY the gate here, not exercise it — hence the
    // plain `submit_signed` (an un-attested `recorded` contributor) is swapped for
    // `submit_attested` with an enrolled human holding responsibility.
    let (sk_h, kid_h) = enroll_human(&c).await;

    // THE WITHDRAWAL NAMES THE REAL ASSERTION, AND ARRIVES BEFORE IT.
    //
    // An earlier version of this test pointed the withdrawal at a GHOST address no assertion
    // would ever carry, then submitted an unrelated assertion and checked both rows landed.
    // That proved only that no FK forbids the pair — it never checked the property the test
    // is named for, which is that the early withdrawal actually CANCELS the late assertion.
    // Under that version, making `cairn_sensitivity_standing` order-sensitive (an
    // `AND w.first_seen > a.first_seen`, or an apply-time "delete the assertion now that its
    // target exists" optimisation) left two honest nodes with equal custody computing
    // permanently different grades, with nothing red.
    //
    // So: sign the assertion first to LEARN its content address, submit the withdrawal naming
    // that address, and only then submit the assertion itself.
    let a = SensitivityAssertion {
        subject_kind: SubjectKind::Patient,
        subject_id: p,
        grade: "sensitive",
        source: cairn_event::sensitivity::Provenance::Human,
        rationale: Some("staff member treated here"),
    };
    let assertion_body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: SENSITIVITY_EVENT_TYPE.into(),
        schema_version: SENSITIVITY_SCHEMA_VERSION.into(),
        hlc: Hlc {
            wall: 11,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: json!([{"actor_id": kid, "role": "recorded"}]),
        payload: sensitivity_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: Some(render_sensitivity_twin(&a)),
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    };
    let signed_assertion = cairn_event::sign(&assertion_body, &sk).unwrap();
    let target_hex = hex::encode(cairn_event::event_address(&signed_assertion.signed_bytes));

    // The ceremony (db/048 section 12) requires a bound human author on a withdrawal —
    // `sensitivity_ceremony.rs` is what exercises that rule; here it is merely satisfied.
    let withdrawal_body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: WITHDRAWAL_EVENT_TYPE.into(),
        schema_version: WITHDRAWAL_SCHEMA_VERSION.into(),
        hlc: Hlc {
            wall: 10,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: json!([{"actor_id": kid_h, "role": "attested",
                              "responsibility": {"held_by": kid_h}}]),
        payload: json!({ "withdraws": target_hex, "rationale": "consent" }),
        attachments: vec![],
        plaintext_twin: Some("withdrawn".into()),
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    };
    submit_attested(&c, &sk, withdrawal_body, &sk_h, &kid_h)
        .await
        .expect(
            "a withdrawal naming an assertion that has not arrived yet must be accepted — \
             set-union sync has no ordering",
        );

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

    // NOW the assertion lands, second.
    c.execute("SELECT submit_event($1)", &[&signed_assertion.signed_bytes])
        .await
        .map_err(|e| db_msg(&e))
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

    // THE POINT OF THE TEST: both rows are present, and the earlier-arriving withdrawal has
    // nonetheless cancelled the later-arriving assertion. Standing is a set difference
    // evaluated at READ, so the order they landed in cannot matter.
    let standing: i64 = c
        .query_one(
            "SELECT count(*) FROM cairn_sensitivity_standing($1::text::uuid)",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        standing, 0,
        "the withdrawal must cancel the assertion it names even though it arrived FIRST"
    );

    let (grade, kind): (String, String) = {
        let r = c
            .query_one(
                "SELECT grade, subject_kind FROM cairn_effective_sensitivity($1::text::uuid)",
                &[&assertion_body.event_id],
            )
            .await
            .unwrap();
        (r.get(0), r.get(1))
    };
    assert_eq!(
        grade, "routine",
        "a withdrawn assertion contributes nothing, whichever order the pair arrived in"
    );
    assert_eq!(kind, "none", "and no subject won");
}
