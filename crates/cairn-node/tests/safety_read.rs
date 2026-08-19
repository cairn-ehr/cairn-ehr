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
use cairn_event::seal::{derive_unwrap_secret, seal_event_payload, seal_stub_twin, unwrap_public};
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
        source: cairn_event::sensitivity::Provenance::Human,
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

/// The overlay + custody tables this suite writes to. None is reached by `setup`'s
/// `TRUNCATE event_log ... CASCADE`, so each is named explicitly.
///
/// * the two sensitivity tables, because db/048 deliberately declares no foreign key to
///   `event_log` (a withdrawal may arrive before the assertion it withdraws) — without
///   naming them, standing assertions leak between tests;
/// * the three custody tables, because `the_signal_survives_a_crypto_shred` registers a
///   node unwrap key, and `cairn_register_unwrap_key` RAISES rather than rotating if a
///   DIFFERENT key is already registered (db/037 section 1) — a leftover row from another
///   suite would fail the fixture, not the assertion.
const OVERLAY_TABLES: &[&str] = &[
    "sensitivity_assertion",
    "sensitivity_withdrawal",
    "node_unwrap_key",
    "event_dek",
    "event_clear",
];

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
async fn the_middle_rung_keeps_the_severity_and_still_drops_the_class() {
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

    // THE POINT OF THIS TEST (2026-08-14 review finding C1). Every OTHER read-model test
    // that asserts "no class survives" does so at rung `existence`, where BOTH columns
    // gate off together. That left db/049 section 7's class gate — the one place the two
    // columns are gated DIFFERENTLY —
    //
    //     CASE WHEN g.eff_rung = 'precise'          THEN g.safety ->> 'class'    END
    //     CASE WHEN g.eff_rung IN ('precise','kind') THEN g.safety ->> 'severity' END
    //
    // pinned by nothing at the only rung that can tell the two apart. Widening the first
    // line to `IN ('precise','kind')` — the obvious "make it match the line below"
    // tidy-up — published a withheld drug class with the ENTIRE suite still green
    // (verified by mutation). This test is the one that fails.
    //
    // `sensitive` is the grade that lands on `kind` (db/049 section 3: rank <= 10), so it
    // is the ONLY grade at which this distinction is observable at all.
    let id = note_with_safety(
        &c,
        &sk,
        &kid,
        patient,
        13,
        serde_json::json!({"rung": "precise", "class": "rh-sensitizing", "severity": "high"}),
    )
    .await;
    grade_chart(&c, &sk, &kid, patient, 14, "sensitive").await;

    let row = c
        .query_one(
            "SELECT rung, class, severity FROM cairn_event_safety($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .expect("read model");
    assert_eq!(
        row.get::<_, String>(0),
        "kind",
        "a `sensitive` chart licenses the middle rung"
    );
    assert!(
        row.get::<_, Option<String>>(1).is_none(),
        "THE ASSERTION THIS TEST EXISTS FOR: the class must NOT survive at rung `kind`. \
         The severity below does. If this fails, db/049's class gate has been widened and \
         a withheld drug class is being published in the clear."
    );
    assert_eq!(
        row.get::<_, Option<String>>(2),
        Some("high".to_string()),
        "the severity DOES survive the middle rung — otherwise this test would pass for \
         the wrong reason (both columns gated off, i.e. the `existence` shape already \
         covered elsewhere)"
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

    // THE BODY IS GENUINELY SEALED, and that is the whole point of the fixture.
    //
    // The obvious cheap version of this test uses an unsealed `note.added` like every
    // other test here. It is worthless: db/020 populates event_clear/event_dek ONLY on
    // the sealed-with-custody arm, so on a plaintext note the rung-3 scrub has nothing
    // to destroy and `cairn_execute_shred` could regress to an empty function body with
    // this test still green. The assertions below therefore establish that custody
    // EXISTED and then that the shred DESTROYED it — otherwise "survives a crypto-shred"
    // is a claim about a shred that never happened.
    //
    // The seal is a payload-level container (`payload.sealed`, db/020 step 7), not a
    // property of the event type, so `note.added` can carry one exactly as a medication
    // verb does — which keeps this fixture free of the medication projections.
    let event_id = Uuid::now_v7().to_string();
    let id: Uuid = event_id.parse().expect("uuid");
    let (container, dek) = seal_event_payload(
        &serde_json::json!({"text": "termination of pregnancy"}),
        "termination of pregnancy",
        &event_id,
    )
    .expect("seals");
    // The node's unwrap key is what lets the apply door wrap this event's DEK into
    // custody. Derived at runtime from the signing key — never a literal (house rule 6).
    let secret = derive_unwrap_secret(&sk.to_bytes());
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&unwrap_public(&secret).as_slice()],
    )
    .await
    .expect("unwrap key registered");

    let body = cairn_event::EventBody {
        event_id,
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: cairn_event::Hlc {
            wall: 16,
            counter: 0,
            node_origin: "n1".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: container,
        attachments: vec![],
        plaintext_twin: Some(seal_stub_twin("note.added")),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        // The clear signal rides on the ENVELOPE, outside the seal — that is the entire
        // §5.9 mechanism: a node with no custody still reads it.
        safety: Some(
            serde_json::json!({"rung": "precise", "class": "rh-sensitizing", "severity": "high"}),
        ),
    };
    let signed = cairn_event::sign(&body, &sk).expect("signs");
    c.execute(
        "SELECT apply_remote_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_slice()],
    )
    .await
    .expect("the sealed body is admitted WITH custody");

    // Custody exists BEFORE the shred. Without this the post-shred zero below would also
    // be satisfied by custody that was never created — a test that passes for the wrong
    // reason in precisely the direction this one is about.
    let (clear_before, dek_before) = custody_counts(&c, id).await;
    assert_eq!(clear_before, 1, "the sealed body has a derived clear view");
    assert_eq!(dek_before, 1, "and a wrapped DEK");

    // The rung-3 shred: custody and derived plaintext die; event_log never does.
    c.execute(
        "SELECT cairn_execute_shred($1::text::uuid, $1::text::uuid, 'test')",
        &[&id.to_string()],
    )
    .await
    .expect("shred");

    let (clear_after, dek_after) = custody_counts(&c, id).await;
    assert_eq!(
        clear_after, 0,
        "the shred destroyed the derived plaintext — if this is non-zero the test below \
         is asserting that a signal survived an erasure that never occurred"
    );
    assert_eq!(
        dek_after, 0,
        "and the wrapped DEK: the body is unreadable now"
    );

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

/// `(event_clear rows, event_dek rows)` for one event — the two things a rung-3 shred is
/// required to destroy (db/037 section 7). Read as counts rather than existence flags so a
/// failure message says how many rows were actually there.
async fn custody_counts(c: &tokio_postgres::Client, event: Uuid) -> (i64, i64) {
    let row = c
        .query_one(
            "SELECT (SELECT count(*) FROM event_clear WHERE event_id = $1::text::uuid), \
                    (SELECT count(*) FROM event_dek   WHERE event_id = $1::text::uuid)",
            &[&event.to_string()],
        )
        .await
        .expect("custody counts");
    (row.get(0), row.get(1))
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

#[tokio::test]
async fn a_withheld_severity_sorts_above_a_known_critical() {
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

    // THE `ELSE` IS THE DECISION, AND THIS IS ITS PIN (db/049 section 1's own discipline,
    // applied to the one ELSE in this file that reaches an ORDER BY).
    //
    // cairn_safety_severity_rank ranks an unrecognised severity MAX, and a coarsened row's
    // severity is SQL NULL — which lands on that same ELSE. So a signal whose severity this
    // reader is not cleared to see sorts ABOVE a known 'critical'. Burying unknowns instead
    // would hide exactly the warnings whose content is unknown BECAUSE they were protected,
    // and it is a one-token change (`NULLS LAST`, a COALESCE to -1) that nothing else here
    // would catch.
    //
    // The fixture is built so the assertion cannot be satisfied by accident:
    //   * `visible` is authored FIRST, so its UUIDv7 sorts BEFORE `withheld` — a plain
    //     `ORDER BY event_id` would put them the other way round;
    //   * `visible` carries 'critical' and `withheld` carries the LOWEST severity, 'low',
    //     so a rank comparison on the RAW stored severity would also invert the order.
    // Only ranking the COARSENED severity, with unknown ranking MAX, produces this order.
    let visible = note_with_safety(
        &c,
        &sk,
        &kid,
        patient,
        40,
        serde_json::json!({"rung": "precise", "class": "statin-interaction", "severity": "critical"}),
    )
    .await;
    let withheld = note_with_safety(
        &c,
        &sk,
        &kid,
        patient,
        41,
        serde_json::json!({"rung": "precise", "class": "rh-sensitizing", "severity": "low"}),
    )
    .await;
    assert!(
        visible < withheld,
        "fixture precondition: the visible signal must sort FIRST by event_id, so the \
         assertion below cannot pass on the tiebreaker alone"
    );

    // EVENT-scoped, so it coarsens `withheld` ALONE and leaves `visible` readable. A
    // chart-wide grade would coarsen both and the ordering would prove nothing.
    assert_grade(
        &c,
        &sk,
        &kid,
        patient,
        42,
        SubjectKind::Event,
        withheld,
        "sequestered",
    )
    .await;

    let rows = c
        .query(
            // `event_id::text`, never a bound/read `Uuid`: this crate does not enable
            // tokio-postgres's `with-uuid-1` feature, so `Uuid` implements neither ToSql
            // nor FromSql here. Same reason every parameter above is `$1::text::uuid`.
            "SELECT event_id::text, rung, severity FROM cairn_patient_safety($1::text::uuid)",
            &[&patient.to_string()],
        )
        .await
        .expect("chart report");
    assert_eq!(rows.len(), 2, "two signals on this chart");

    assert_eq!(
        rows[0].get::<_, String>(0),
        withheld.to_string(),
        "the signal whose severity is WITHHELD sorts first — a reader must not have the \
         one warning it cannot read pushed below the ones it can"
    );
    assert_eq!(rows[0].get::<_, String>(1), "existence");
    assert!(
        rows[0].get::<_, Option<String>>(2).is_none(),
        "and it is withheld because the row is coarsened, not because it had no severity"
    );

    assert_eq!(rows[1].get::<_, String>(0), visible.to_string());
    assert_eq!(rows[1].get::<_, String>(1), "precise");
    assert_eq!(
        rows[1].get::<_, Option<String>>(2).as_deref(),
        Some("critical"),
        "the known-critical signal is the one that got sorted BELOW it"
    );
}

#[tokio::test]
async fn the_prospective_thread_arms_match_coarsen_and_bound() {
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

    // The other two arms of cairn_prospective_sensitivity. The anti-drift tests above
    // exercise only the chart and dangling-event arms, because both of their fixtures are
    // `note.added` — a type db/048 section 10b's gate declares thread-free, so the thread
    // arms are switched off for them entirely and could be deleted with those tests green.
    //
    // WHAT THIS PINS AND WHAT IT DOES NOT. `p_thread` is a PARAMETER, so all three arms are
    // reachable directly and are pinned here. This does NOT pin agreement with db/048
    // section 11 for a thread-BEARING event, which would need a sealed medication verb with
    // read-custody so `cairn_event_thread` resolves — see the report's Concerns.
    let thread = Uuid::now_v7();
    assert_grade(
        &c,
        &sk,
        &kid,
        patient,
        50,
        SubjectKind::Thread,
        thread,
        "restricted",
    )
    .await;

    let prospective = |arg: Option<Uuid>| {
        let c = &c;
        async move {
            let row = c
                .query_one(
                    "SELECT grade, subject_kind FROM cairn_prospective_sensitivity(\
                     $1::text::uuid, $2::text::uuid)",
                    &[&patient.to_string(), &arg.map(|t| t.to_string())],
                )
                .await
                .expect("prospective");
            (row.get::<_, String>(0), row.get::<_, String>(1))
        }
    };

    // 1. THIS thread — the precisely-targeted arm. The grade applies and names its own kind.
    assert_eq!(
        prospective(Some(thread)).await,
        ("restricted".into(), "thread".into()),
        "an assertion on the thread we are about to write on applies precisely"
    );

    // 2. A DIFFERENT thread, whose subject this node CANNOT RESOLVE — stays silent (#404).
    //
    //    This arm asserted the opposite until #404: that any thread assertion not naming
    //    our thread "coarsens chart-wide, never evaporates", attributed to db/048 section
    //    11. That is NOT section 11's rule for threads, and the misattribution is what let
    //    the bug through. db/048 asks the POSITIVE question — is the named thread known
    //    here AND demonstrably on a DIFFERENT chart — precisely because
    //    `medication_statement` is custody-gated and absent from the cairn-sync subset, so
    //    "not found" is the NORMAL state and carries no information at all. Using the
    //    'event' arm's ABSENCE shape here would coarsen every chart on every custody-less
    //    node, which is what section 10b's type gate exists to prevent.
    //
    //    `thread` above is a bare UUID with no medication_statement row, so
    //    `cairn_thread_patient` cannot resolve it and the arm correctly stays silent.
    //
    //    THE OTHER HALF — a thread that DOES resolve, on ANOTHER chart, which must coarsen
    //    — cannot be built here: this suite has no custody, so no thread ever resolves. It
    //    is pinned in safety_emission.rs's
    //    `a_grade_on_another_thread_of_the_same_chart_does_not_coarsen_this_one`, which has
    //    `medication_setup` and therefore real, resolvable threads.
    assert_eq!(
        prospective(Some(Uuid::now_v7())).await,
        ("routine".into(), "none".into()),
        "a thread assertion this node cannot resolve carries no information — coarsening \
         on it would blur every chart on every custody-less node (db/048's own argument)"
    );

    // 3. NO thread — decision 9's conservative bound. At emission time an unresolved thread
    //    is the honest reading of "this event MAY be on that thread", and the safe answer to
    //    a maybe is the protected one. A caller passing NULL must not be handed 'routine'.
    assert_eq!(
        prospective(None).await,
        ("restricted".into(), "coarsened".into()),
        "an unknown thread takes the bound — the failure direction here must be \
         over-coarsening, never disclosure"
    );

    // And the floor under all three: a chart with no assertion at all still reads 'routine',
    // so the bound above is a real answer rather than this function coarsening everything.
    let clean = Uuid::now_v7();
    let row = c
        .query_one(
            "SELECT grade, subject_kind FROM cairn_prospective_sensitivity(\
             $1::text::uuid, NULL)",
            &[&clean.to_string()],
        )
        .await
        .expect("prospective");
    assert_eq!(
        (row.get::<_, String>(0), row.get::<_, String>(1)),
        ("routine".to_string(), "none".to_string()),
        "absence is not unknown (principle 4) — no assertion reads routine, never a bound"
    );
}
