//! §5.9 part B (ADR-0063) — the read model, and the three properties that make it safe:
//! it re-coarsens by the CURRENT grade, it is TOTAL over any stored shape, and it survives
//! a crypto-shred.
//!
//! # Why totality is the point of this file
//!
//! db/020 (the remote door) deliberately ADMITS a malformed or self-contradictory safety
//! signal rather than refusing it, because refusing would drop the clinical event the
//! signal rides on (ADR-0060 — a defect in a de-identified advisory field must never
//! cancel a clinical order). That leniency is only safe because the READER refuses to ACT
//! on a contradiction. Every test below pins one arm of that refusal, so the pair
//! (`safety_doors.rs` admits, `safety_read.rs` declines to act) is what makes the design
//! sound rather than merely lenient.
//!
//! Every test here self-skips without `$CAIRN_TEST_PG` (`cs()` returns `None`), and cargo
//! then reports the suite as passing while running nothing — a green run that prints no
//! test names is a SKIP, not a pass.
mod common;
use cairn_event::sensitivity::{SubjectKind, SENSITIVITY_EVENT_TYPE, SENSITIVITY_SCHEMA_VERSION};
use common::{cs, setup};
use uuid::Uuid;

/// Submit a `note.added` carrying a verbatim safety signal, returning its event id.
/// `note.added` is unsealed, so these tests exercise the read model without the seal path.
///
/// Authored through `apply_remote_event` on purpose: the LOCAL door (db/005) refuses a
/// self-contradictory signal, so a test that needs such a shape ON DISK can only get it
/// through the sync door — which is exactly the situation the read model exists for. As a
/// side effect these fixtures need no registration event: `apply_remote_event` never runs
/// db/005 step 8b's precedence rule (issue #345), by design.
async fn note_with_safety(
    c: &tokio_postgres::Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
    safety: serde_json::Value,
) -> Uuid {
    let body = cairn_event::EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: cairn_event::Hlc {
            wall,
            counter: 0,
            node_origin: "n1".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "a note"}),
        attachments: vec![],
        plaintext_twin: Some("a note".into()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: Some(safety),
    };
    let id: Uuid = body.event_id.parse().expect("uuid");
    let signed = cairn_event::sign(&body, sk).expect("signs");
    // apply_remote_event, so a test may put a shape on the wire that the local door refuses.
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("admitted");
    id
}

/// Assert one standing sensitivity grade through the real db/048 path.
///
/// Generalised over subject kind/id (rather than being chart-only) because the
/// prospective-vs-effective pins below need an EVENT-scoped assertion as well as a
/// chart-scoped one. Applied through the remote door for the same reason as
/// `note_with_safety`: db/048 section 12's authoring ceremony is a LOCAL-door rule, and
/// these fixtures are about what the READ model does with what is already on disk.
#[allow(clippy::too_many_arguments)] // mirrors sensitivity_ladder.rs's assert_grade
async fn assert_grade(
    c: &tokio_postgres::Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
    subject_kind: SubjectKind,
    subject_id: Uuid,
    grade: &str,
) {
    let a = cairn_event::sensitivity::SensitivityAssertion {
        subject_kind,
        subject_id,
        grade,
        source: "human",
        rationale: Some("test fixture"),
    };
    let body = cairn_event::EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: SENSITIVITY_EVENT_TYPE.into(),
        schema_version: SENSITIVITY_SCHEMA_VERSION.into(),
        hlc: cairn_event::Hlc {
            wall,
            counter: 0,
            node_origin: "n1".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: cairn_event::sensitivity::sensitivity_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: Some(cairn_event::sensitivity::render_sensitivity_twin(&a)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    let signed = cairn_event::sign(&body, sk).expect("signs");
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("grade applied");
}

/// The common case: one chart-wide grade on `patient`.
async fn grade_chart(
    c: &tokio_postgres::Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
    grade: &str,
) {
    assert_grade(
        c,
        sk,
        kid,
        patient,
        wall,
        SubjectKind::Patient,
        patient,
        grade,
    )
    .await;
}

/// The sensitivity overlay tables this suite writes to. They are NOT reached by
/// `setup`'s `TRUNCATE event_log ... CASCADE` (db/048 deliberately declares no foreign key
/// to `event_log` — a withdrawal may arrive before the assertion it withdraws), so they
/// are named explicitly or standing assertions leak between tests.
const OVERLAY_TABLES: &[&str] = &["sensitivity_assertion", "sensitivity_withdrawal"];

#[tokio::test]
async fn a_peers_finer_rung_is_coarsened_by_this_nodes_grade() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, OVERLAY_TABLES).await;
    let patient = Uuid::now_v7();

    // A peer emits `precise` — legitimately, because on ITS node the chart is routine.
    let id = note_with_safety(
        &c,
        &sk,
        &kid,
        patient,
        10,
        serde_json::json!({"rung": "precise", "class": "rh-sensitizing", "severity": "high"}),
    )
    .await;
    grade_chart(&c, &sk, &kid, patient, 11, "sequestered").await;

    let row = c
        .query_one(
            "SELECT rung, class, severity FROM cairn_event_safety($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .expect("read model");
    assert_eq!(
        row.get::<_, String>(0),
        "existence",
        "the grade this node holds must coarsen a peer's finer rung — emission cannot \
         control a peer's bytes, so read is the local defence"
    );
    assert!(
        row.get::<_, Option<String>>(1).is_none(),
        "no class survives"
    );
    assert!(
        row.get::<_, Option<String>>(2).is_none(),
        "no severity survives"
    );
}

#[tokio::test]
async fn a_self_contradictory_signal_never_surfaces_its_class() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, OVERLAY_TABLES).await;
    let patient = Uuid::now_v7();

    // The shape db/005 refuses and db/020 admits. Totality here is what makes that
    // leniency safe rather than merely lenient.
    let id = note_with_safety(
        &c,
        &sk,
        &kid,
        patient,
        12,
        serde_json::json!({"rung": "existence", "class": "rh-sensitizing"}),
    )
    .await;

    let row = c
        .query_one(
            "SELECT rung, class FROM cairn_event_safety($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .expect("read model");
    assert_eq!(row.get::<_, String>(0), "existence");
    assert!(
        row.get::<_, Option<String>>(1).is_none(),
        "a class is surfaced ONLY at rung 'precise', whatever the row holds"
    );
}

#[tokio::test]
async fn an_unrecognised_rung_reads_as_the_coarsest() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, OVERLAY_TABLES).await;
    let patient = Uuid::now_v7();

    let id = note_with_safety(
        &c,
        &sk,
        &kid,
        patient,
        13,
        serde_json::json!({"rung": "rung:from-a-future-peer", "severity": "critical"}),
    )
    .await;

    let row = c
        .query_one(
            "SELECT rung, severity FROM cairn_event_safety($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .expect("read model");
    assert_eq!(
        row.get::<_, String>(0),
        "existence",
        "unknown ⇒ disclose nothing — and NORMALISED to a rung every reader knows, never \
         echoed back as a value this node could not interpret"
    );
    assert!(row.get::<_, Option<String>>(1).is_none());
}

#[tokio::test]
async fn a_missing_rung_reads_as_the_coarsest() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, OVERLAY_TABLES).await;
    let patient = Uuid::now_v7();

    // MISSING, not merely unrecognised — a distinct arm of the same totality rule, and the
    // one a SQL `CASE` on the raw string is most likely to let through as NULL. db/005
    // refuses a rung-less signal outright, so only the sync door can produce this row.
    let id = note_with_safety(
        &c,
        &sk,
        &kid,
        patient,
        14,
        serde_json::json!({"severity": "critical"}),
    )
    .await;

    let row = c
        .query_one(
            "SELECT rung, class, severity FROM cairn_event_safety($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .expect("a signal with no rung still reads — TOTAL means no shape returns no answer");
    assert_eq!(
        row.get::<_, String>(0),
        "existence",
        "absent rung ⇒ disclose nothing"
    );
    assert!(row.get::<_, Option<String>>(1).is_none());
    assert!(row.get::<_, Option<String>>(2).is_none());
}

#[tokio::test]
async fn an_event_with_no_signal_yields_no_row() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, OVERLAY_TABLES).await;
    let patient = Uuid::now_v7();

    let body = cairn_event::EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: cairn_event::Hlc {
            wall: 15,
            counter: 0,
            node_origin: "n1".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "a note"}),
        attachments: vec![],
        plaintext_twin: Some("a note".into()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    let id: Uuid = body.event_id.parse().expect("uuid");
    let signed = cairn_event::sign(&body, &sk).expect("signs");
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("ok");

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM cairn_event_safety($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .expect("read model")
        .get(0);
    assert_eq!(
        n, 0,
        "no signal means no row — an existence marker on every uncoded event would \
         manufacture a warning from nothing (ADR-0059 decision 4's honest floor)"
    );
}

#[tokio::test]
async fn the_signal_survives_a_crypto_shred() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, OVERLAY_TABLES).await;
    let patient = Uuid::now_v7();

    let id = note_with_safety(
        &c,
        &sk,
        &kid,
        patient,
        16,
        serde_json::json!({"rung": "precise", "class": "rh-sensitizing", "severity": "high"}),
    )
    .await;

    // The rung-3 shred: custody and derived plaintext die; event_log never does.
    c.execute(
        "SELECT cairn_execute_shred($1::text::uuid, $1::text::uuid, 'test')",
        &[&id.to_string()],
    )
    .await
    .expect("shred");

    let row = c
        .query_one(
            "SELECT rung, class FROM cairn_event_safety($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .expect(
            "the safety projection outlives the body it protects — the Rh-after-termination \
             signal must reach a future antenatal clinician even after the episode is erased",
        );
    assert_eq!(row.get::<_, String>(0), "precise");
    assert_eq!(
        row.get::<_, Option<String>>(1).as_deref(),
        Some("rh-sensitizing")
    );
}

#[tokio::test]
async fn prospective_matches_effective_given_the_same_chart_and_thread() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, OVERLAY_TABLES).await;
    let patient = Uuid::now_v7();

    // The anti-drift pin. cairn_prospective_sensitivity duplicates cairn_effective_
    // sensitivity's arms minus the event arm, because at emission time the event does not
    // exist yet. If the two ever disagree for an event carrying no event-scoped assertion,
    // one of them has drifted.
    //
    // It compares the two FUNCTIONS against each other rather than either against a
    // hardcoded grade, because the drift this guards against would move BOTH away from a
    // literal at once (a new grade interposed in db/048's ladder, say). The trailing
    // literal assertion is a sanity check that the fixture is exercising something at all
    // — a green "routine == routine" would otherwise be indistinguishable from a pass.
    grade_chart(&c, &sk, &kid, patient, 20, "restricted").await;
    let id = note_with_safety(
        &c,
        &sk,
        &kid,
        patient,
        21,
        serde_json::json!({"rung": "existence"}),
    )
    .await;

    let eff: String = c
        .query_one(
            "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .expect("effective")
        .get(0);
    let pro: String = c
        .query_one(
            "SELECT grade FROM cairn_prospective_sensitivity($1::text::uuid, NULL)",
            &[&patient.to_string()],
        )
        .await
        .expect("prospective")
        .get(0);
    assert_eq!(
        pro, eff,
        "prospective and effective must agree when no event-scoped assertion stands"
    );
    assert_eq!(pro, "restricted");
}

#[tokio::test]
async fn a_dangling_event_scoped_assertion_coarsens_prospectively_too() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, OVERLAY_TABLES).await;
    let patient = Uuid::now_v7();

    // The second half of the anti-drift pin, and the one that is easy to get wrong.
    //
    // "Minus the event arm" means minus the PRECISELY-TARGETED event arm — an event that
    // does not exist yet cannot be named by an assertion. It does NOT mean minus the
    // CATCH-ALL's event clause: db/048 section 11 reads an event-scoped assertion whose
    // target is not on this chart (a wrong chart, a dangling id, or — most often — an
    // event that has simply not replicated here yet) as coarsening the WHOLE chart. That
    // condition is fully computable before the new event exists.
    //
    // Were it dropped from the prospective grade, emission would compute 'routine' and
    // publish a PRECISE class in the clear on the wire, while every read of that same
    // event on this same node said 'existence'. The bytes cannot be recalled; the read
    // model cannot un-publish them. So the prospective grade must carry this arm.
    let never_replicated = Uuid::now_v7();
    assert_grade(
        &c,
        &sk,
        &kid,
        patient,
        22,
        SubjectKind::Event,
        never_replicated,
        "sequestered",
    )
    .await;
    let id = note_with_safety(
        &c,
        &sk,
        &kid,
        patient,
        23,
        serde_json::json!({"rung": "precise", "class": "rh-sensitizing", "severity": "high"}),
    )
    .await;

    let eff = c
        .query_one(
            "SELECT grade, subject_kind FROM cairn_effective_sensitivity($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .expect("effective");
    let pro = c
        .query_one(
            "SELECT grade, subject_kind FROM cairn_prospective_sensitivity($1::text::uuid, NULL)",
            &[&patient.to_string()],
        )
        .await
        .expect("prospective");
    assert_eq!(
        (pro.get::<_, String>(0), pro.get::<_, String>(1)),
        (eff.get::<_, String>(0), eff.get::<_, String>(1)),
        "a dangling event-scoped assertion coarsens chart-wide at READ; it must coarsen \
         identically at EMISSION, or this node publishes what it will then refuse to show"
    );
    assert_eq!(eff.get::<_, String>(0), "sequestered");
    assert_eq!(
        eff.get::<_, String>(1),
        "coarsened",
        "'coarsened' — something applies chart-wide that we could not match to a subject"
    );

    // And the read side does its half: the peer's precise rung is pulled down.
    let read = c
        .query_one(
            "SELECT rung, class FROM cairn_event_safety($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .expect("read model");
    assert_eq!(read.get::<_, String>(0), "existence");
    assert!(read.get::<_, Option<String>>(1).is_none());
}

#[tokio::test]
async fn the_chart_report_names_the_winning_subject() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, OVERLAY_TABLES).await;
    let patient = Uuid::now_v7();

    let _ = note_with_safety(
        &c, &sk, &kid, patient, 30,
        serde_json::json!({"rung": "precise", "class": "statin-interaction", "severity": "moderate"}),
    ).await;
    grade_chart(&c, &sk, &kid, patient, 31, "sensitive").await;

    let rows = c
        .query(
            "SELECT rung, severity, grade, subject_kind FROM cairn_patient_safety($1::text::uuid)",
            &[&patient.to_string()],
        )
        .await
        .expect("chart report");
    assert_eq!(rows.len(), 1, "one signal on this chart");
    assert_eq!(
        rows[0].get::<_, String>(0),
        "kind",
        "'sensitive' coarsens to kind"
    );
    assert_eq!(
        rows[0].get::<_, Option<String>>(1).as_deref(),
        Some("moderate")
    );
    assert_eq!(rows[0].get::<_, String>(2), "sensitive");
    // ADR-0062 decision 8 control 3: never just the grade — a grade with no named source
    // cannot be fixed.
    assert_eq!(rows[0].get::<_, String>(3), "patient");
}
