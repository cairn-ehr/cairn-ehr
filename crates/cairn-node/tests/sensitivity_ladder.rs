//! The §5.9 sensitivity ladder (ADR-0062).
//!
//! The one thing to understand before editing: an UNRECOGNISED grade ranks MAX here,
//! which is the exact opposite of `cairn_clock_grade_rank`'s `ELSE 0`. See the comment
//! on `cairn_sensitivity_rank` in db/048 — a "fix" that aligns the two is a leak.
mod common;
use cairn_event::sensitivity::*;
use common::{
    cs, db_msg, setup, submit_registration, submit_signed, submit_signed_with_id, EventSpec,
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

    submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: WITHDRAWAL_EVENT_TYPE,
            schema_version: WITHDRAWAL_SCHEMA_VERSION,
            payload: serde_json::json!({ "withdraws": ca_hex, "rationale": "patient consent" }),
            plaintext_twin: Some("withdrawn".into()),
            wall: 12,
        },
    )
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
