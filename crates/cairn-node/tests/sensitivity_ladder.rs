//! The §5.9 sensitivity ladder (ADR-0062).
//!
//! The one thing to understand before editing: an UNRECOGNISED grade ranks MAX here,
//! which is the exact opposite of `cairn_clock_grade_rank`'s `ELSE 0`. See the comment
//! on `cairn_sensitivity_rank` in db/048 — a "fix" that aligns the two is a leak.
mod common;
use cairn_event::medication::CodingClaim;
use cairn_event::sensitivity::*;
use cairn_node::medication::{
    assert_medication, code_medication, correct_medication_coding, AssertMedicationInput,
    CodeMedicationInput, CorrectCodingInput, SubstanceCoding,
};
use common::{
    cs, db_msg, enroll_human, medication_setup, setup, submit_attested, submit_registration,
    submit_signed, submit_signed_with_id, EventSpec,
};

#[tokio::test]
async fn the_ladder_orders_the_named_grades_and_ranks_the_unknown_maximum() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    let rank = |g: &'static str| {
        let c = &c;
        async move {
            c.query_one("SELECT cairn_sensitivity_rank($1)", &[&g])
                .await
                .map(|r| r.get::<_, i32>(0))
                .map_err(|e| db_msg(&e))
                .unwrap()
        }
    };

    assert_eq!(rank("routine").await, 0, "no protection asserted");
    assert!(rank("sensitive").await < rank("restricted").await);
    assert!(rank("restricted").await < rank("sequestered").await);

    // The inverted unknown. A future peer's grade must coarsen, never expose.
    assert_eq!(
        rank("grade:protected-witness").await,
        i32::MAX,
        "an unrecognised grade must rank MAX: ranking it 0 would let an older node read a \
         peer's newer grade as 'not sensitive' and render the body in the clear"
    );

    // NULL lands on the same safe side (a NOT NULL column makes this unreachable, but the
    // function is public API and must not have an unsafe corner).
    let null_rank: i32 = c
        .query_one("SELECT cairn_sensitivity_rank(NULL)", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(null_rank, i32::MAX);
}

/// Helper: author one assertion and return nothing. Kept local — it is only meaningful
/// with this file's fixtures.
#[allow(clippy::too_many_arguments)] // mirrors the same-shaped helpers in match_veto.rs / demographics_names.rs
async fn assert_grade(
    c: &tokio_postgres::Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    p: uuid::Uuid,
    kind: SubjectKind,
    subject: uuid::Uuid,
    grade: &str,
    wall: i64,
) {
    let a = SensitivityAssertion {
        subject_kind: kind,
        subject_id: subject,
        grade,
        source: "human",
        rationale: Some("test fixture"),
    };
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: sensitivity_assertion_body(&a),
            plaintext_twin: Some(render_sensitivity_twin(&a)),
            wall,
        },
    )
    .await
    .expect("assertion accepted");
}

#[tokio::test]
async fn the_effective_grade_is_the_max_over_event_thread_and_chart() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // A plain event on this chart with no assertion of its own. Named via
    // submit_signed_with_id (not submit_signed, which mints its own opaque event id) because
    // the assertions below need to name this exact event.
    let target = uuid::Uuid::now_v7();
    submit_signed_with_id(
        &c,
        &sk,
        &kid,
        target,
        EventSpec {
            patient: p,
            event_type: "note.added",
            schema_version: "note.added/1",
            payload: serde_json::json!({ "text": "routine note" }),
            plaintext_twin: Some("routine note".into()),
            wall: 10,
        },
    )
    .await
    .expect("note accepted");

    let effective = |ev: uuid::Uuid| {
        let c = &c;
        async move {
            c.query_one(
                "SELECT grade, subject_kind FROM cairn_effective_sensitivity($1::text::uuid)",
                &[&ev.to_string()],
            )
            .await
            .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
            .map_err(|e| db_msg(&e))
            .unwrap()
        }
    };

    // No assertions anywhere: absence reads as routine, NOT as unknown.
    assert_eq!(effective(target).await.0, "routine");

    // A chart-wide grade reaches an event that carries none of its own.
    assert_grade(&c, &sk, &kid, p, SubjectKind::Patient, p, "sensitive", 11).await;
    assert_eq!(
        effective(target).await,
        ("sensitive".into(), "patient".into())
    );

    // An event-scoped grade outranks the chart-wide one: max, and the winner is named.
    assert_grade(
        &c,
        &sk,
        &kid,
        p,
        SubjectKind::Event,
        target,
        "restricted",
        12,
    )
    .await;
    assert_eq!(
        effective(target).await,
        ("restricted".into(), "event".into())
    );
}

#[tokio::test]
async fn a_withdrawal_lowers_the_effective_grade_and_the_assertion_survives() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let target = uuid::Uuid::now_v7();
    submit_signed_with_id(
        &c,
        &sk,
        &kid,
        target,
        EventSpec {
            patient: p,
            event_type: "note.added",
            schema_version: "note.added/1",
            payload: serde_json::json!({ "text": "n" }),
            plaintext_twin: Some("n".into()),
            wall: 10,
        },
    )
    .await
    .unwrap();

    assert_grade(&c, &sk, &kid, p, SubjectKind::Patient, p, "sequestered", 11).await;
    let ca_hex: String = c
        .query_one(
            "SELECT encode(content_address, 'hex') FROM sensitivity_assertion
              WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);

    // Task 6's ceremony (db/048 `cairn_sensitivity_ceremony_ok`) added a second local-door
    // requirement on top of the structural floor's rationale: a withdrawal now also needs a
    // BOUND HUMAN AUTHOR — a contributor claiming responsibility, verified by attestation
    // (ADR-0053). This test is about the effective-grade projection, so it only needs to
    // satisfy the gate, not exercise it — hence `submit_attested` with an enrolled human in
    // place of the plain `submit_signed` this test used before the ceremony existed.
    let (sk_h, kid_h) = enroll_human(&c).await;
    let withdrawal_body = cairn_event::EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: WITHDRAWAL_EVENT_TYPE.into(),
        schema_version: WITHDRAWAL_SCHEMA_VERSION.into(),
        hlc: cairn_event::Hlc {
            wall: 12,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid_h, "role": "attested",
                                          "responsibility": {"held_by": kid_h}}]),
        payload: serde_json::json!({ "withdraws": ca_hex, "rationale": "patient consent" }),
        attachments: vec![],
        plaintext_twin: Some("withdrawn".into()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    submit_attested(&c, &sk, withdrawal_body, &sk_h, &kid_h)
        .await
        .expect("withdrawal accepted");

    let g: String = c
        .query_one(
            "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
            &[&target.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(g, "routine", "the withdrawn assertion no longer stands");

    // Nothing was erased — the assertion is still on the record, still re-assertable.
    let still: i64 = c
        .query_one(
            "SELECT count(*) FROM sensitivity_assertion WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(still, 1, "declassification is an overlay, never an erasure");
}

#[tokio::test]
async fn an_unknown_subject_kind_is_read_as_chart_wide_and_never_crosses_charts() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = uuid::Uuid::now_v7();
    let other = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    submit_registration(&c, &sk, &kid, other, 1).await;

    // mine/theirs are named via submit_signed_with_id so the closure below can query
    // cairn_effective_sensitivity for these exact events afterwards.
    let mine = uuid::Uuid::now_v7();
    submit_signed_with_id(
        &c,
        &sk,
        &kid,
        mine,
        EventSpec {
            patient: p,
            event_type: "note.added",
            schema_version: "note.added/1",
            payload: serde_json::json!({"text":"n"}),
            plaintext_twin: Some("n".into()),
            wall: 10,
        },
    )
    .await
    .unwrap();
    let theirs = uuid::Uuid::now_v7();
    submit_signed_with_id(
        &c,
        &sk,
        &kid,
        theirs,
        EventSpec {
            patient: other,
            event_type: "note.added",
            schema_version: "note.added/1",
            payload: serde_json::json!({"text":"n"}),
            plaintext_twin: Some("n".into()),
            wall: 10,
        },
    )
    .await
    .unwrap();

    submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: serde_json::json!({
                "subject_kind": "episode", "subject_id": uuid::Uuid::now_v7().to_string(),
                "grade": "restricted", "source": "human"
            }),
            plaintext_twin: Some("future kind".into()),
            wall: 11,
        },
    )
    .await
    .expect("admitted");

    let g = |ev: uuid::Uuid| {
        let c = &c;
        async move {
            c.query_one(
                "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
                &[&ev.to_string()],
            )
            .await
            .unwrap()
            .get::<_, String>(0)
        }
    };
    assert_eq!(
        g(mine).await,
        "restricted",
        "unknown kind is read conservatively, chart-wide"
    );
    assert_eq!(
        g(theirs).await,
        "routine",
        "and the envelope bounds it to ITS OWN chart"
    );
}

#[tokio::test]
async fn recall_marks_an_assertion_but_never_lowers_the_grade() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let target = uuid::Uuid::now_v7();
    submit_signed_with_id(
        &c,
        &sk,
        &kid,
        target,
        EventSpec {
            patient: p,
            event_type: "note.added",
            schema_version: "note.added/1",
            payload: serde_json::json!({"text":"n"}),
            plaintext_twin: Some("n".into()),
            wall: 10,
        },
    )
    .await
    .unwrap();
    assert_grade(&c, &sk, &kid, p, SubjectKind::Patient, p, "restricted", 11).await;

    // cairn-node does not enable tokio-postgres's uuid feature, so the event id is read
    // back as text (::text in the SELECT) rather than bound to a Uuid FromSql impl that
    // does not exist here — same idiom as medication_authorship.rs.
    let assertion_event: String = c
        .query_one(
            "SELECT event_id::text FROM sensitivity_assertion WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);

    // Recall the assertion's own event. recall_overlay MARKS; it must never remove the
    // assertion from the standing set — otherwise recalling a bad actor would silently
    // strip protection from every patient they graded.
    // recall_overlay is (recall_id PK DEFAULT gen_random_uuid(), target_event_id, reason,
    // recorded_at) — db/006. target_event_id carries an FK to event_log, which is why the
    // assertion's own event_id is fetched above rather than invented.
    c.execute(
        "INSERT INTO recall_overlay (target_event_id, reason)
         VALUES ($1::text::uuid, 'test recall')",
        &[&assertion_event],
    )
    .await
    .map_err(|e| db_msg(&e))
    .expect("recall_overlay insert");

    let g: String = c
        .query_one(
            "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
            &[&target.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(g, "restricted", "recall marks; it must never lower a grade");
}

// ===========================================================================
// Review round 1 fixes.
// ===========================================================================

#[tokio::test]
async fn f1_a_withdrawal_authored_on_a_different_chart_does_not_lower_this_chart_s_grade() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let victim = uuid::Uuid::now_v7();
    let attacker = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, victim, 1).await;
    submit_registration(&c, &sk, &kid, attacker, 1).await;

    // The chart being protected: a chart-wide 'sequestered' grade.
    assert_grade(
        &c,
        &sk,
        &kid,
        victim,
        SubjectKind::Patient,
        victim,
        "sequestered",
        2,
    )
    .await;
    let ca_hex: String = c
        .query_one(
            "SELECT encode(content_address, 'hex') FROM sensitivity_assertion
              WHERE patient_id = $1::text::uuid",
            &[&victim.to_string()],
        )
        .await
        .unwrap()
        .get(0);

    // A withdrawal authored on a DIFFERENT chart, naming the victim's content_address,
    // arriving as a PEER's event (`node_origin: "peer"`, applied through `apply_remote_event`
    // rather than `submit_signed`). Task 6's local-door-only human-author ceremony (db/048's
    // header) now refuses a rationale-only, unattested withdrawal at the LOCAL door — so this
    // shape can only be exercised through the remote door, exactly the one ADR-0060 keeps
    // lenient (a peer's honestly-authored-but-locally-non-conformant act must not fork the
    // event set). Same content_address (globally unique, so unambiguous about WHICH assertion
    // it names), but authored under a different envelope's patient_id.
    let cross_chart_withdrawal = cairn_event::EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: attacker.to_string(),
        event_type: WITHDRAWAL_EVENT_TYPE.into(),
        schema_version: WITHDRAWAL_SCHEMA_VERSION.into(),
        hlc: cairn_event::Hlc {
            wall: 3,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({ "withdraws": ca_hex, "rationale": "cross-chart" }),
        attachments: vec![],
        plaintext_twin: Some("withdrawn".into()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    let signed = cairn_event::sign(&cross_chart_withdrawal, &sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("the remote door admits this leniently — the ceremony is local-only");

    // Review finding F2: `apply_remote_event` is lenient enough to have a deferred/
    // unclassified path that admits an event while writing NO projection rows and raising
    // no error — so the grade-stayed-`sequestered` assertion below would be equally true
    // if this withdrawal never projected at all, and the F1 `patient_id` pin in
    // `cairn_sensitivity_standing` this test exists to cover would go unexercised while the
    // test still read green. Pin that the withdrawal actually LANDED before trusting what
    // it did or didn't do to the grade.
    let landed: i64 = c
        .query_one(
            "SELECT count(*) FROM sensitivity_withdrawal WHERE withdraws = decode($1,'hex')",
            &[&ca_hex],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        landed, 1,
        "the cross-chart withdrawal must actually project"
    );

    let target = uuid::Uuid::now_v7();
    submit_signed_with_id(
        &c,
        &sk,
        &kid,
        target,
        EventSpec {
            patient: victim,
            event_type: "note.added",
            schema_version: "note.added/1",
            payload: serde_json::json!({ "text": "n" }),
            plaintext_twin: Some("n".into()),
            wall: 4,
        },
    )
    .await
    .expect("note accepted");

    let g: String = c
        .query_one(
            "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
            &[&target.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        g, "sequestered",
        "a withdrawal from a DIFFERENT chart must never strip this chart's protection (F1)"
    );
}

#[tokio::test]
async fn f2_a_mis_targeted_known_subject_kind_coarsens_instead_of_evaporating() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    // Case i: a 'patient'-kind assertion authored on chart A whose subject_id names a
    // DIFFERENT patient (a typo, a UI bug, a hostile peer). Before F2 this matched NO arm
    // of cairn_effective_sensitivity and contributed nothing — a silent fail-open on
    // exactly the field most likely to be mis-set.
    let chart_a = uuid::Uuid::now_v7();
    let elsewhere = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, chart_a, 1).await;
    submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: chart_a,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            // Task 6's ceremony requires a rationale on every CHART-WIDE ('patient') raise —
            // this is still a raise (grade going up), never a withdrawal, so no attestation
            // is needed, only the rationale string.
            payload: serde_json::json!({
                "subject_kind": "patient", "subject_id": elsewhere.to_string(),
                "grade": "restricted", "source": "human", "rationale": "test fixture (F2i)"
            }),
            plaintext_twin: Some("mis-targeted patient assertion".into()),
            wall: 2,
        },
    )
    .await
    .expect("structurally well-formed, so admitted");

    let note_a = uuid::Uuid::now_v7();
    submit_signed_with_id(
        &c,
        &sk,
        &kid,
        note_a,
        EventSpec {
            patient: chart_a,
            event_type: "note.added",
            schema_version: "note.added/1",
            payload: serde_json::json!({ "text": "n" }),
            plaintext_twin: Some("n".into()),
            wall: 3,
        },
    )
    .await
    .expect("note accepted");
    let g_a: String = c
        .query_one(
            "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
            &[&note_a.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        g_a, "restricted",
        "a 'patient' assertion naming a DIFFERENT patient must coarsen its own chart, not evaporate (F2i)"
    );

    // Case ii: an 'event'-kind assertion whose subject_id names no event on THIS chart
    // (invalid/dangling — could equally be a real event on someone else's chart).
    let chart_b = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, chart_b, 1).await;
    submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: chart_b,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: serde_json::json!({
                "subject_kind": "event", "subject_id": uuid::Uuid::now_v7().to_string(),
                "grade": "sensitive", "source": "human"
            }),
            plaintext_twin: Some("mis-targeted event assertion".into()),
            wall: 2,
        },
    )
    .await
    .expect("structurally well-formed, so admitted");

    let note_b = uuid::Uuid::now_v7();
    submit_signed_with_id(
        &c,
        &sk,
        &kid,
        note_b,
        EventSpec {
            patient: chart_b,
            event_type: "note.added",
            schema_version: "note.added/1",
            payload: serde_json::json!({ "text": "n" }),
            plaintext_twin: Some("n".into()),
            wall: 3,
        },
    )
    .await
    .expect("note accepted");
    let g_b: String = c
        .query_one(
            "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
            &[&note_b.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        g_b, "sensitive",
        "an 'event' assertion naming no event on THIS chart must coarsen the chart, not evaporate (F2ii)"
    );
}

/// Shared by the F5 thread-coverage tests: the minimal input `assert_medication` needs to
/// mint a fresh thread. `term` is the only field the tests vary.
fn med_input(term: &'static str) -> AssertMedicationInput<'static> {
    AssertMedicationInput {
        term,
        coding: None,
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "patient-reported",
        started: None,
        started_precision: None,
    }
}

/// Code a thread once, then immediately correct it (strike). Returns the FIRST coding
/// event's id — its content_address is now superseded in `medication_coding` (an
/// `ON CONFLICT (medication_id) DO UPDATE`, HLC-overlaid table — db/042), so
/// `cairn_event_thread` can no longer resolve it even though the correction, and the
/// thread's own assert event, remain fully resolvable. This is the real-door way to
/// produce the "unresolved despite full custody" case F4/F5(b)/F5(c) need, matching
/// exactly the mechanism db/048's F4 comment now documents.
async fn supersede_a_coding_event(
    c: &mut tokio_postgres::Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    p: uuid::Uuid,
    thread: uuid::Uuid,
) -> uuid::Uuid {
    let coding = CodeMedicationInput {
        coding: SubstanceCoding {
            system: "drugref-moiety",
            code: "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01",
            display: "test substance",
        },
    };
    let first = code_medication(c, sk, kid, "test-node", p, thread, &coding, None, None)
        .await
        .expect("initial coding accepted");
    let correction = CorrectCodingInput {
        corrects: first,
        claim: CodingClaim::Strike,
        note: Some("test correction — forces supersession for the F5 fixture"),
    };
    correct_medication_coding(c, sk, kid, "test-node", p, thread, &correction, None, None)
        .await
        .expect("correction accepted");
    first
}

#[tokio::test]
async fn f5a_a_resolved_thread_carries_its_own_grade_and_not_a_different_threads() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    // A real medication-thread fixture, authored through the actual clinical door
    // (assert_medication) rather than a hand-seeded row — this three-way rule (resolved /
    // bounded / contributes-nothing) had ZERO test coverage before this fix.
    let (sk, kid, _sk_h, _kid_h) = medication_setup(&c).await;
    let p = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 0).await;

    let thread_a = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        p,
        &med_input("A"),
        None,
        None,
    )
    .await
    .expect("thread A asserted");
    let thread_b = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        p,
        &med_input("B"),
        None,
        None,
    )
    .await
    .expect("thread B asserted");

    // Each thread's own (only, so far) assert event, found via medication_statement's
    // content_address — precise here because neither thread has been re-asserted yet.
    let assert_event_of = |med: uuid::Uuid| {
        let c = &c;
        async move {
            let s: String = c
                .query_one(
                    "SELECT e.event_id::text FROM medication_statement m
                       JOIN event_log e ON e.content_address = m.content_address
                      WHERE m.medication_id = $1::text::uuid",
                    &[&med.to_string()],
                )
                .await
                .unwrap()
                .get(0);
            s
        }
    };
    let event_a = assert_event_of(thread_a).await;
    let event_b = assert_event_of(thread_b).await;

    assert_grade(
        &c,
        &sk,
        &kid,
        p,
        SubjectKind::Thread,
        thread_a,
        "restricted",
        10,
    )
    .await;

    let grade_of = |ev: String| {
        let c = &c;
        async move {
            c.query_one(
                "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
                &[&ev],
            )
            .await
            .unwrap()
            .get::<_, String>(0)
        }
    };
    assert_eq!(
        grade_of(event_a).await,
        "restricted",
        "a resolved thread's own grade applies to its events"
    );
    assert_eq!(
        grade_of(event_b).await,
        "routine",
        "a DIFFERENT thread's grade must never leak onto this one"
    );
}

#[tokio::test]
async fn f5bd_unresolved_thread_takes_the_bounded_max_but_a_threadless_event_type_does_not() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, _sk_h, _kid_h) = medication_setup(&c).await;
    let p = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 0).await;

    let thread = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        p,
        &med_input("Warfarin"),
        None,
        None,
    )
    .await
    .expect("thread asserted");
    let first_coding_event = supersede_a_coding_event(&mut c, &sk, &kid, p, thread).await;

    // A note on the SAME chart: cairn_event_type_has_no_thread('note.added') is TRUE, so
    // F3's gate keeps a note OUT of the bound no matter what thread-scoped assertions the
    // chart carries (a note can never be on a medication thread).
    let note = uuid::Uuid::now_v7();
    submit_signed_with_id(
        &c,
        &sk,
        &kid,
        note,
        EventSpec {
            patient: p,
            event_type: "note.added",
            schema_version: "note.added/1",
            payload: serde_json::json!({ "text": "unrelated note" }),
            plaintext_twin: Some("unrelated note".into()),
            wall: 30,
        },
    )
    .await
    .expect("note accepted");

    // The thread-scoped assertion that gives this chart something to bound TO.
    assert_grade(
        &c,
        &sk,
        &kid,
        p,
        SubjectKind::Thread,
        thread,
        "sequestered",
        31,
    )
    .await;

    let grade_of = |ev: uuid::Uuid| {
        let c = &c;
        async move {
            c.query_one(
                "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
                &[&ev.to_string()],
            )
            .await
            .unwrap()
            .get::<_, String>(0)
        }
    };
    assert_eq!(
        grade_of(first_coding_event).await,
        "sequestered",
        "unresolved + the chart HAS a thread-scoped assertion -> the bounded max applies (F5b)"
    );
    assert_eq!(
        grade_of(note).await,
        "routine",
        "a note can never be on a medication thread, so the SAME chart's thread-scoped \
         'sequestered' assertion must not coarsen it (F3's maintainer ruling, F5d)"
    );
}

#[tokio::test]
async fn f5c_unresolved_thread_contributes_nothing_when_the_chart_has_no_thread_assertions() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, _sk_h, _kid_h) = medication_setup(&c).await;
    let p = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 0).await;

    let thread = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        p,
        &med_input("Metformin"),
        None,
        None,
    )
    .await
    .expect("thread asserted");
    let first_coding_event = supersede_a_coding_event(&mut c, &sk, &kid, p, thread).await;

    // NO thread-scoped (or any) assertion anywhere on this chart, so `standing` is EMPTY
    // and no arm of cairn_effective_sensitivity's `applicable` CTE can fire regardless of
    // the thread logic — this test is NOT discriminating between the fixed and pre-fix
    // code (it passes identically against either): with no thread-scoped rows there is
    // nothing to bound OR to leave unbounded, because "the chart has none" is not a
    // separate clause anywhere in the SQL — it is EMERGENT from the bound arm matching
    // zero standing rows. Resolution itself (that a real thread-scoped grade DOES apply
    // when it exists) is what f5a pins, and that is the test that actually fails if
    // thread resolution regresses.
    //
    // What THIS test guards (review round 2, R2-2): a future rewrite that turned "nothing
    // applies" into a SENTINEL max-coarsening grade instead of contributing nothing at
    // all — e.g. someone "simplifying" the unresolved-thread branch into an unconditional
    // maximal grade whenever `cairn_event_thread` returns NULL, rather than only when the
    // chart ALSO carries a thread-scoped assertion to bound to. That regression would
    // coarsen every medication event on every custody-less node, chart-wide, with no
    // sensitivity assertion anywhere — and this is the test that would catch it, by
    // asserting 'routine' rather than merely asserting "not wrong in some way".
    let g: String = c
        .query_one(
            "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
            &[&first_coding_event.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        g, "routine",
        "unresolved + no thread assertions anywhere on the chart -> contributes nothing (F5c)"
    );
}

#[tokio::test]
async fn f5e_a_tie_between_two_equally_ranked_grades_resolves_deterministically() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // Two DIFFERENT chart-wide assertions at the SAME rank ('sensitive' both times) — a
    // genuine tie for cairn_effective_sensitivity's ORDER BY to break. Before this test the
    // tie-break line (`ORDER BY rank DESC, content_address ASC`) had never been exercised
    // by a real tie.
    assert_grade(&c, &sk, &kid, p, SubjectKind::Patient, p, "sensitive", 10).await;
    assert_grade(&c, &sk, &kid, p, SubjectKind::Patient, p, "sensitive", 11).await;

    let target = uuid::Uuid::now_v7();
    submit_signed_with_id(
        &c,
        &sk,
        &kid,
        target,
        EventSpec {
            patient: p,
            event_type: "note.added",
            schema_version: "note.added/1",
            payload: serde_json::json!({ "text": "n" }),
            plaintext_twin: Some("n".into()),
            wall: 12,
        },
    )
    .await
    .expect("note accepted");

    let expected_ca: Vec<u8> = c
        .query_one(
            "SELECT content_address FROM sensitivity_assertion
              WHERE patient_id = $1::text::uuid
              ORDER BY content_address ASC LIMIT 1",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);

    let row = c
        .query_one(
            "SELECT grade, content_address FROM cairn_effective_sensitivity($1::text::uuid)",
            &[&target.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "sensitive");
    let got_ca: Vec<u8> = row.get(1);
    assert_eq!(
        got_ca, expected_ca,
        "a tie in rank must resolve to the assertion with the SMALLER content_address, deterministically (F5e)"
    );
}

// ===========================================================================
// Task 8: the operator surface (crates/cairn-node/src/sensitivity.rs). The projection
// itself is fully covered above; these tests are about the ORCHESTRATOR — does it build
// the right wire shape, submit through the real door, and (for the report) NAME the
// winning subject rather than just the grade.
// ===========================================================================

/// The `db_msg` idiom (common/mod.rs), one layer further out: `assert_sensitivity` /
/// `withdraw_sensitivity` return `anyhow::Result`, so a door refusal arrives here as an
/// `anyhow::Error` wrapping the original `tokio_postgres::Error` (via `?`). `anyhow::Error`'s
/// `Display` — and therefore `{err}` / `err.to_string()` — renders only
/// `tokio_postgres::Error`'s OWN generic wrapper text ("db error"), never the actual
/// `RAISE EXCEPTION` message; that message lives in the `DbError` payload underneath, the
/// exact trap `db_msg`'s own doc comment warns about. Downcasting back to
/// `tokio_postgres::Error` and reusing `db_msg` is what actually gets at it.
fn anyhow_db_msg(err: &anyhow::Error) -> String {
    err.downcast_ref::<tokio_postgres::Error>()
        .map(db_msg)
        .unwrap_or_else(|| err.to_string())
}

#[tokio::test]
async fn the_chart_report_names_the_winning_subject_for_every_graded_thread() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    assert_grade(&c, &sk, &kid, p, SubjectKind::Patient, p, "sensitive", 11).await;

    let report = cairn_node::sensitivity::chart_sensitivity(&mut c, p)
        .await
        .unwrap();
    assert_eq!(report.chart_grade, "sensitive");
    assert_eq!(
        report.chart_source, "chart-wide",
        "the report must name WHICH subject won — otherwise nobody can tell why a whole \
         chart is blurred, and therefore nobody can fix it"
    );
}

#[tokio::test]
async fn a_chart_with_no_assertions_reports_routine_and_names_no_winner() {
    // The other half of "names the winning subject": a chart with nothing graded must
    // report the honest absence ('routine' / 'none'), not merely omit the field or crash.
    // Exercises chart_sensitivity's zero-assertion path, which
    // the_chart_report_names_the_winning_subject_for_every_graded_thread never reaches.
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    let report = cairn_node::sensitivity::chart_sensitivity(&mut c, p)
        .await
        .unwrap();
    assert_eq!(report.chart_grade, "routine");
    assert_eq!(report.chart_source, "none");
    assert!(
        report.threads.is_empty(),
        "no medication threads exist on this chart"
    );
}

#[tokio::test]
async fn assert_sensitivity_writes_a_well_formed_event_and_still_needs_a_rationale_chart_wide() {
    // The orchestrator must not quietly bypass the db/048 ceremony it wraps: a thread
    // raise with no rationale succeeds, a chart-wide raise with no rationale is refused —
    // exactly the asymmetry sensitivity_ceremony.rs pins at the door, now proven to survive
    // being routed through assert_sensitivity rather than a hand-built EventBody.
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    let thread = uuid::Uuid::now_v7();
    let event_id = cairn_node::sensitivity::assert_sensitivity(
        &mut c,
        &sk,
        &kid,
        "test-node",
        p,
        SubjectKind::Thread,
        thread,
        "restricted",
        None,
    )
    .await
    .expect("a thread raise carries no ceremony");

    let row = c
        .query_one(
            "SELECT subject_kind, subject_id::text, grade, source
               FROM sensitivity_assertion WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "thread");
    assert_eq!(row.get::<_, String>(1), thread.to_string());
    assert_eq!(row.get::<_, String>(2), "restricted");
    assert_eq!(row.get::<_, String>(3), "human");

    // The event the orchestrator returns is the SAME one that landed — not an opaque
    // internal id the caller has no way to correlate with what actually happened.
    let landed_id: String = c
        .query_one(
            "SELECT event_id::text FROM event_log WHERE content_address =
                (SELECT content_address FROM sensitivity_assertion WHERE patient_id = $1::text::uuid)",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(landed_id, event_id.to_string());

    // A chart-wide raise with no rationale must still be refused — the orchestrator is a
    // thin builder, not a second, laxer door.
    let err = cairn_node::sensitivity::assert_sensitivity(
        &mut c,
        &sk,
        &kid,
        "test-node",
        p,
        SubjectKind::Patient,
        p,
        "restricted",
        None,
    )
    .await
    .expect_err("a chart-wide raise with no rationale must be refused locally");
    let msg = anyhow_db_msg(&err);
    assert!(
        msg.contains("chart-wide"),
        "the refusal names what would repair it: {msg}"
    );
}

#[tokio::test]
async fn withdraw_sensitivity_requires_the_human_key_and_then_lowers_the_grade() {
    // Mirrors sensitivity_ceremony.rs's two withdrawal tests, but through the real
    // orchestrator rather than a hand-built EventBody: an un-enrolled-as-human signer is
    // refused (the ceremony's bound-human-author rule, ADR-0053), and an enrolled human
    // succeeds and the standing grade actually drops.
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    assert_grade(&c, &sk, &kid, p, SubjectKind::Patient, p, "sequestered", 2).await;

    // The hex `content_address` an operator would actually have to copy off
    // `patient-sensitivity`'s own printed output to fill in `--withdraws` — sourced from
    // `chart_sensitivity` itself (not a direct table query) so this test also proves
    // `chart_content_address` round-trips a real, usable value end to end.
    let ca_hex = cairn_node::sensitivity::chart_sensitivity(&mut c, p)
        .await
        .unwrap()
        .chart_content_address
        .expect("a standing chart-wide assertion must carry a withdrawable content_address");

    // The plain device/agent key `setup` enrolled is NOT a human actor — withdraw_sensitivity
    // must refuse it, proving this orchestrator carries no separate, laxer path to this event
    // type. `withdraw_sensitivity` always mints an attestation token (it treats its sk/kid as
    // THE human by design — see the function's own doc), so passing a non-human key trips
    // db/005 step 4b's "is the attester actually an enrolled human" check BEFORE db/048's
    // ceremony (`cairn_sensitivity_ceremony_ok`) ever gets a chance to run — an earlier, more
    // fundamental refusal than the plain-un-attested shape
    // `the_local_door_requires_a_bound_human_author_for_a_withdrawal` in sensitivity_ceremony.rs
    // exercises, but the same underlying rule: no bound human, no withdrawal.
    let err = cairn_node::sensitivity::withdraw_sensitivity(
        &mut c,
        &sk,
        &kid,
        "test-node",
        p,
        &ca_hex,
        "patient consent",
    )
    .await
    .expect_err("a non-human signer must be refused");
    let msg = anyhow_db_msg(&err);
    assert!(
        msg.contains("enrolled human actor"),
        "the refusal names what would repair it: {msg}"
    );

    let (sk_h, kid_h) = enroll_human(&c).await;
    let event_id = cairn_node::sensitivity::withdraw_sensitivity(
        &mut c,
        &sk_h,
        &kid_h,
        "test-node",
        p,
        &ca_hex,
        "patient consent",
    )
    .await
    .expect("an enrolled human's withdrawal is accepted");

    let landed: i64 = c
        .query_one(
            "SELECT count(*) FROM sensitivity_withdrawal WHERE withdraws = decode($1,'hex')",
            &[&ca_hex],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(landed, 1, "the withdrawal actually projected");

    let report = cairn_node::sensitivity::chart_sensitivity(&mut c, p)
        .await
        .unwrap();
    assert_eq!(
        report.chart_grade, "routine",
        "the withdrawn assertion no longer stands: event {event_id}"
    );
}

#[tokio::test]
async fn the_chart_report_lists_each_medication_thread_with_its_own_winning_subject() {
    // threads is the per-thread half of the report — proven separately from chart_grade
    // because a thread's OWN standing grade can still be OUTRANKED by a chart-wide one, and
    // the report must say so rather than parroting the thread's own assertion.
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, _sk_h, _kid_h) = medication_setup(&c).await;
    let p = uuid::Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 0).await;

    let thread_a = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        p,
        &med_input("A"),
        None,
        None,
    )
    .await
    .expect("thread A asserted");
    let thread_b = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        p,
        &med_input("B"),
        None,
        None,
    )
    .await
    .expect("thread B asserted");

    // thread_a carries its own, thread-scoped grade.
    assert_grade(
        &c,
        &sk,
        &kid,
        p,
        SubjectKind::Thread,
        thread_a,
        "restricted",
        10,
    )
    .await;

    let report = cairn_node::sensitivity::chart_sensitivity(&mut c, p)
        .await
        .unwrap();
    assert_eq!(report.threads.len(), 2, "both threads are reported");
    let a = report
        .threads
        .iter()
        .find(|t| t.thread_id == thread_a)
        .expect("thread A present");
    assert_eq!(a.grade, "restricted");
    assert_eq!(a.source, "this thread");
    assert!(
        a.content_address.is_some(),
        "a real winning assertion must carry a withdrawable content_address"
    );
    let b = report
        .threads
        .iter()
        .find(|t| t.thread_id == thread_b)
        .expect("thread B present");
    assert_eq!(
        b.grade, "routine",
        "thread B's own grade must not pick up thread A's"
    );
    assert_eq!(b.source, "none");
    assert!(
        b.content_address.is_none(),
        "nothing applies to thread B, so there is nothing to withdraw"
    );

    // Now a chart-wide raise OUTRANKS thread_a's own 'restricted' — the report must follow
    // the true winner, not the thread's own standing row.
    assert_grade(&c, &sk, &kid, p, SubjectKind::Patient, p, "sequestered", 11).await;
    let report = cairn_node::sensitivity::chart_sensitivity(&mut c, p)
        .await
        .unwrap();
    let a = report
        .threads
        .iter()
        .find(|t| t.thread_id == thread_a)
        .expect("thread A present");
    assert_eq!(
        a.grade, "sequestered",
        "a higher chart-wide grade must win over the thread's own"
    );
    assert_eq!(
        a.source, "chart-wide",
        "and the report must say the CHART is what actually won, not the thread"
    );
}
