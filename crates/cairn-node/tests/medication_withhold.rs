//! ADR-0081 (#803) — a medication withhold ("W" on the paper drug chart) and its early
//! release, through db/059's floor, projections and local-door refusals.
//!
//! The refusals split in two, and these tests pin each half at the door it belongs to:
//!   * STRUCTURAL (both doors, `cairn_check_medication_withhold`): an empty reason, a
//!     missing or malformed `until`, a malformed (or null) `from`, `until <= from` (pinned
//!     at the sync door too), an empty `releases`, a null/blank/non-string release `reason`,
//!     a bad `medication_id`, and a missing `t_effective` where it is the hold's start or
//!     the release's instant (pinned at the sync door too) — one table per verb below.
//!   * CLINICAL STATE (local door only, in the apply fns): a withhold on a ceased line, a
//!     hold whose span has already ended once `from` is resolved, a release naming
//!     something that is not a withhold on the same thread, a thread on another chart
//!     (#192). Sync apply never refuses on
//!     state — a node-local veto would fork the event set — so the last test drives a
//!     withhold on a ceased line through `apply_remote_event` and sees it admitted.
//!   * The standing chart of an orphan thread claimed by several withholds is the EARLIEST
//!     claim, whatever the arrival order (#192).
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` (shared-DB +
//! TRUNCATE pattern, like medication.rs). Key material is derived at runtime.
use cairn_event::medication::HoldUntil;
use cairn_event::{sign, Hlc, SigningKey};
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::{
    build_assert_body, build_release_body, build_withhold_body, reconcile_medications,
    release_medication_holds, withhold_medication, ReconcileInput, WithholdMedicationInput,
};
use tokio_postgres::Client;
use uuid::Uuid;

// `submit_registration`: since #345 the first event on a chart must be its registration.
mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;

/// Withhold through the orchestrator and return the error text (the test expects a refusal).
async fn withhold_err(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    med: Uuid,
    input: &WithholdMedicationInput<'_>,
) -> String {
    let err = withhold_medication(c, sk, kid, "test-node", patient, med, input, None)
        .await
        .expect_err("the withhold must be refused");
    format!("{err:#}")
}

/// Release through the orchestrator and return the error text (the test expects a refusal).
async fn release_err(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    p: Uuid,
    med: Uuid,
    ids: &[Uuid],
) -> String {
    let err = release_medication_holds(c, sk, kid, "test-node", p, med, ids, None, None)
        .await
        .expect_err("the release must be refused");
    format!("{err:#}")
}

async fn withhold_rows(c: &Client, med: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM medication_withhold WHERE medication_id = $1::text::uuid",
        &[&med.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn a_withhold_is_recorded_with_its_resolved_span() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;

    let wid = withhold(&mut c, &sk, &kid, patient, med, "BP 85/50").await;

    // One row, keyed to the returned event id; the open end is "reviewed", and an absent
    // `from` resolved to the event's own time (its t_effective: the node's physical clock).
    let row = c
        .query_one(
            "SELECT until_reviewed, until_at IS NULL, reason, \
                    abs(extract(epoch FROM starts_at - now())) < 60, patient_id::text \
             FROM medication_withhold WHERE event_id = $1::text::uuid",
            &[&wid.to_string()],
        )
        .await
        .unwrap();
    assert!(row.get::<_, bool>(0), "until_reviewed");
    assert!(row.get::<_, bool>(1), "until_at is NULL for an open hold");
    assert_eq!(row.get::<_, String>(2), "BP 85/50");
    assert!(row.get::<_, bool>(3), "starts_at resolves to about now");
    assert_eq!(row.get::<_, String>(4), patient.to_string());
}

#[tokio::test]
async fn an_empty_reason_is_refused_by_the_floor() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;

    let err = withhold_err(&mut c, &sk, &kid, patient, med, &until_reviewed("   ")).await;
    assert!(err.contains("medication withhold:"), "got: {err}");
    assert!(err.contains("reason"), "got: {err}");
    assert_eq!(withhold_rows(&c, med).await, 0);
}

#[tokio::test]
async fn an_offset_less_until_is_refused() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;

    let input = WithholdMedicationInput {
        reason: "NBM for theatre",
        from: None,
        until: HoldUntil::At("2099-01-01T20:00:00"),
    };
    let err = withhold_err(&mut c, &sk, &kid, patient, med, &input).await;
    assert!(err.contains("medication withhold:"), "got: {err}");
    assert!(
        err.contains("until"),
        "the refusal names the field, got: {err}"
    );
    assert_eq!(withhold_rows(&c, med).await, 0);
}

#[tokio::test]
async fn until_not_after_from_is_refused() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;

    // Same instant written in two offsets: `until` == `from`, so the span is empty.
    let input = WithholdMedicationInput {
        reason: "NBM for theatre",
        from: Some("2099-01-01T20:00:00+10:00"),
        until: HoldUntil::At("2099-01-01T10:00:00Z"),
    };
    let err = withhold_err(&mut c, &sk, &kid, patient, med, &input).await;
    assert!(err.contains("medication withhold:"), "got: {err}");
    assert!(err.contains("after"), "got: {err}");
    assert_eq!(withhold_rows(&c, med).await, 0);
}

#[tokio::test]
async fn a_missing_until_is_refused() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;

    // The orchestrator always writes `until`; strip it by hand to prove the FLOOR refuses a
    // payload that never said how the hold ends (principle 4: no silent "forever").
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let mut body = build_withhold_body(
        Uuid::now_v7(),
        med,
        patient,
        &until_reviewed("NBM"),
        &kid,
        hlc,
    );
    body.payload.as_object_mut().unwrap().remove("until");
    let err = db_msg(&seal_and_submit(&c, &sk, body).await.unwrap_err());
    assert!(err.contains("medication withhold:"), "got: {err}");
    assert!(err.contains("until"), "got: {err}");
    assert_eq!(withhold_rows(&c, med).await, 0);
}

#[tokio::test]
async fn an_until_already_past_is_refused_at_the_local_door() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;

    // No `from`, so the hold starts at the event's own time — and `until` is long gone.
    let input = WithholdMedicationInput {
        reason: "NBM for theatre",
        from: None,
        until: HoldUntil::At("2001-01-01T00:00:00Z"),
    };
    let err = withhold_err(&mut c, &sk, &kid, patient, med, &input).await;
    assert!(err.contains("medication withhold:"), "got: {err}");
    assert!(err.contains("after"), "got: {err}");
    assert_eq!(withhold_rows(&c, med).await, 0);
}

#[tokio::test]
async fn a_withhold_on_a_ceased_line_is_refused_at_the_local_door() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    cease(&mut c, &sk, &kid, patient, med, None).await;

    let err = withhold_err(&mut c, &sk, &kid, patient, med, &until_reviewed("BP 85/50")).await;
    assert!(
        err.contains("medication withhold: this line is ceased"),
        "got: {err}"
    );
    assert_eq!(withhold_rows(&c, med).await, 0);
}

#[tokio::test]
async fn a_line_with_one_ceased_member_still_admits_a_withhold() {
    // Review Focus 3: two statements of one drug, reconciled into one line; the OLDER one
    // is ceased with a stop date before the other's start. The line's group status is
    // latest-effective-wins (ADR-0047), so it reads ACTIVE and a W on it must be admitted
    // even though the member thread it is written on is itself ceased.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, older) = chart_with_one_med(&mut c, &sk, &kid).await;
    let newer = add_med(&mut c, &sk, &kid, patient).await;
    let edge = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    reconcile_medications(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        older,
        newer,
        &edge,
        None,
        None,
    )
    .await
    .unwrap();
    // Stopped 2020, before the newer member's 2024 start: the line stays active.
    cease(&mut c, &sk, &kid, patient, older, Some("2020")).await;
    let status: String = c
        .query_one(
            "SELECT s.status FROM medication_thread_group g \
             JOIN medication_group_status s ON s.group_id = g.group_id \
             WHERE g.medication_id = $1::text::uuid",
            &[&older.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(status, "active", "precondition: the line reads active");

    withhold(&mut c, &sk, &kid, patient, older, "HR 48").await;
    assert_eq!(withhold_rows(&c, older).await, 1);
}

#[tokio::test]
async fn a_release_of_a_standing_withhold_is_recorded() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let wid = withhold(&mut c, &sk, &kid, patient, med, "BP 85/50").await;

    let rid = release_medication_holds(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med,
        &[wid],
        Some("BP recovered"),
        None,
    )
    .await
    .unwrap();
    let row = c
        .query_one(
            "SELECT released_event_ids = ARRAY[$2::text::uuid], reason, medication_id::text \
             FROM medication_withhold_release WHERE event_id = $1::text::uuid",
            &[&rid.to_string(), &wid.to_string()],
        )
        .await
        .unwrap();
    assert!(row.get::<_, bool>(0), "the release names exactly the hold");
    assert_eq!(
        row.get::<_, Option<String>>(1).as_deref(),
        Some("BP recovered")
    );
    assert_eq!(row.get::<_, String>(2), med.to_string());
}

#[tokio::test]
async fn an_empty_release_is_refused_by_the_floor() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;

    let err = release_err(&mut c, &sk, &kid, patient, med, &[]).await;
    assert!(err.contains("medication release:"), "got: {err}");
    assert!(err.contains("releases"), "got: {err}");
}

#[tokio::test]
async fn a_release_naming_an_unknown_event_is_refused() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;

    let err = release_err(&mut c, &sk, &kid, patient, med, &[Uuid::now_v7()]).await;
    assert!(
        err.contains("medication release:") && err.contains("is not a withhold on this thread"),
        "got: {err}"
    );
}

#[tokio::test]
async fn a_release_naming_another_lines_withhold_is_refused() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med_a) = chart_with_one_med(&mut c, &sk, &kid).await;
    let med_b = add_med(&mut c, &sk, &kid, patient).await;
    let wid_a = withhold(&mut c, &sk, &kid, patient, med_a, "BP 85/50").await;

    // A real withhold, but on a DIFFERENT thread: a release can only end holds on its own,
    // and the refusal says where the hold IS, so the clinician can release it there.
    let err = release_err(&mut c, &sk, &kid, patient, med_b, &[wid_a]).await;
    assert!(
        err.contains("medication release:")
            && err.contains(&format!(
                "{wid_a} is a withhold on thread {med_a}, not {med_b}: release it there"
            )),
        "got: {err}"
    );
}

#[tokio::test]
async fn a_sync_applied_withhold_on_a_ceased_line_is_admitted() {
    // The local-door refusal must NOT run at sync apply: a peer already holds this validly
    // signed W (its node may not have seen the cessation yet), and a local veto would fork
    // the event set. The fold decides what it means (cessation wins, ADR-0081 decision 5).
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    cease(&mut c, &sk, &kid, patient, med, None).await;

    // A peer-minted clock, ahead of ours, so the only thing under test is the state veto.
    let local = db::next_hlc(&c, "test-node").await.unwrap();
    let peer = Hlc {
        wall: local.wall + 1_000,
        counter: 0,
        node_origin: "peer".into(),
    };
    let w = until_reviewed("HR 44 on the ward");
    let body = effective_at_wall(
        &c,
        build_withhold_body(Uuid::now_v7(), med, patient, &w, &kid, peer),
    )
    .await;
    c.execute(
        "SELECT apply_remote_event($1)",
        &[&sign(&body, &sk).unwrap().signed_bytes],
    )
    .await
    .expect("sync apply admits a withhold on a ceased line");
    assert_eq!(withhold_rows(&c, med).await, 1);
}

#[tokio::test]
async fn an_orphan_withhold_claims_its_thread_for_its_chart() {
    // #192 via cairn_medication_thread_patient's fourth (withhold) arm (db/031): a W that
    // arrives before the assert it names files its row under ITS chart, so a later assert of
    // the same thread under a DIFFERENT chart must be refused at the local door — otherwise
    // the statement and the withhold would disagree about whose drug this is.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient_a = Uuid::now_v7();
    let patient_b = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, patient_a, 0).await;
    common::submit_registration(&c, &sk, &kid, patient_b, 0).await;
    let med = Uuid::now_v7(); // never asserted here: the withhold is an orphan
    withhold(&mut c, &sk, &kid, patient_a, med, "NBM").await;

    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body = build_assert_body(
        Uuid::now_v7(),
        med,
        patient_b,
        &sample_input(),
        &kid,
        hlc,
        None,
    );
    let err = db_msg(&seal_and_submit(&c, &sk, body).await.unwrap_err());
    assert!(err.contains("patient cannot change"), "got: {err}");
}

#[tokio::test]
async fn an_empty_span_is_refused_by_the_floor_at_sync_apply_too() {
    // until == from (one instant, two offsets) is STRUCTURAL: malformed everywhere, so the
    // floor refuses it at the sync door as well. The local-door test above cannot tell the
    // floor's branch from the apply fn's state check; this one can — the apply fn's checks
    // are switched off at this door.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;

    let local = db::next_hlc(&c, "test-node").await.unwrap();
    let input = WithholdMedicationInput {
        reason: "NBM for theatre",
        from: Some("2099-01-01T20:00:00+10:00"),
        until: HoldUntil::At("2099-01-01T10:00:00Z"),
    };
    let body = build_withhold_body(
        Uuid::now_v7(),
        med,
        patient,
        &input,
        &kid,
        peer_hlc(local.wall + 1_000),
    );
    let err = db_msg(&apply_remote(&c, &sk, &body).await.unwrap_err());
    assert!(
        err.contains("medication withhold:") && err.contains("after"),
        "got: {err}"
    );
    assert_eq!(withhold_rows(&c, med).await, 0);
}

#[tokio::test]
async fn a_null_from_is_refused_by_the_floor() {
    // Optional fields are OMITTED, never null: a JSON null `from` is malformed, not "now".
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;

    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let mut body = build_withhold_body(
        Uuid::now_v7(),
        med,
        patient,
        &until_reviewed("NBM"),
        &kid,
        hlc,
    );
    body.payload
        .as_object_mut()
        .unwrap()
        .insert("from".into(), serde_json::Value::Null);
    let err = db_msg(&seal_and_submit(&c, &sk, body).await.unwrap_err());
    assert!(
        err.contains("medication withhold:") && err.contains("from"),
        "got: {err}"
    );
    assert_eq!(withhold_rows(&c, med).await, 0);
}

#[tokio::test]
async fn a_blank_or_null_release_reason_is_refused_by_the_floor() {
    // The release's `reason` is optional: omitted, or a non-blank string — never null, never
    // blank, never another JSON type.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let wid = withhold(&mut c, &sk, &kid, patient, med, "BP 85/50").await;

    let err = release_medication_holds(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med,
        &[wid],
        Some("  "),
        None,
    )
    .await
    .expect_err("a blank release reason is refused");
    let err = format!("{err:#}");
    assert!(
        err.contains("medication release:") && err.contains("reason"),
        "got: {err}"
    );

    for bad in [serde_json::Value::Null, serde_json::json!(42)] {
        let hlc = db::next_hlc(&c, "test-node").await.unwrap();
        let mut body = build_release_body(Uuid::now_v7(), med, patient, &[wid], None, &kid, hlc);
        body.payload
            .as_object_mut()
            .unwrap()
            .insert("reason".into(), bad.clone());
        let err = db_msg(&seal_and_submit(&c, &sk, body).await.unwrap_err());
        assert!(
            err.contains("medication release:") && err.contains("reason"),
            "{bad}: got: {err}"
        );
    }
    let n: i64 = c
        .query_one("SELECT count(*) FROM medication_withhold_release", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0);
}

#[tokio::test]
async fn the_earliest_orphan_withhold_claim_stands_whatever_the_arrival_order() {
    // Two orphan withholds of one thread, for DIFFERENT charts, arrive by sync — the LATER one
    // (by HLC) first. The sync door converges-and-flags rather than refusing (#192), so both
    // rows exist. The thread's standing chart must be the EARLIEST claim, on every call and on
    // every node, never whichever row a scan happens to meet first.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (early, late) = (Uuid::now_v7(), Uuid::now_v7());
    let med = Uuid::now_v7();
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;

    let w = until_reviewed("NBM");
    let late_body = effective_at_wall(
        &c,
        build_withhold_body(Uuid::now_v7(), med, late, &w, &kid, peer_hlc(wall + 10)),
    )
    .await;
    let early_body = effective_at_wall(
        &c,
        build_withhold_body(Uuid::now_v7(), med, early, &w, &kid, peer_hlc(wall)),
    )
    .await;
    apply_remote(&c, &sk, &late_body)
        .await
        .expect("later claim applies");
    apply_remote(&c, &sk, &early_body)
        .await
        .expect("earlier claim applies (flagged)");
    assert_eq!(
        withhold_rows(&c, med).await,
        2,
        "both rows stored (converge-and-flag)"
    );

    for _ in 0..5 {
        let standing: String = c
            .query_one(
                "SELECT cairn_medication_thread_patient($1::text::uuid)::text",
                &[&med.to_string()],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(standing, early.to_string(), "the earliest claim stands");
    }
}

/// One row of a structural-refusal table: what the case is, how it breaks the payload, and
/// the word the floor's refusal must contain.
type StructuralCase = (
    &'static str,
    Box<dyn Fn(&mut serde_json::Map<String, serde_json::Value>)>,
    &'static str,
);

/// A well-formed withhold body ("until reviewed", no `from`, stamped `t_effective`) for the
/// structural tables to break one field at a time.
async fn valid_withhold_body(
    c: &Client,
    kid: &str,
    patient: Uuid,
    med: Uuid,
) -> cairn_event::EventBody {
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let body = build_withhold_body(
        Uuid::now_v7(),
        med,
        patient,
        &until_reviewed("NBM"),
        kid,
        hlc,
    );
    effective_at_wall(c, body).await
}

#[tokio::test]
async fn every_structural_withhold_refusal_holds_at_the_local_door() {
    // ADR-0081 decision 8: the floor's promise against a raw-SQL client, field by field.
    // Each case breaks ONE thing in an otherwise valid body; the refusal must name it.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    use serde_json::{json, Value};

    // (what to do to the payload, the word the refusal must contain)
    let cases: Vec<StructuralCase> = vec![
        (
            "medication_id not a string",
            Box::new(|p| {
                p.insert("medication_id".into(), json!(42));
            }),
            "medication_id",
        ),
        (
            "medication_id not a uuid",
            Box::new(|p| {
                p.insert("medication_id".into(), json!("nope"));
            }),
            "medication_id",
        ),
        (
            "reason absent",
            Box::new(|p| {
                p.remove("reason");
            }),
            "reason",
        ),
        (
            "reason not a string",
            Box::new(|p| {
                p.insert("reason".into(), json!(5));
            }),
            "reason",
        ),
        (
            "until null",
            Box::new(|p| {
                p.insert("until".into(), Value::Null);
            }),
            "until",
        ),
        (
            "until a number",
            Box::new(|p| {
                p.insert("until".into(), json!(7));
            }),
            "until",
        ),
        (
            "until garbage",
            Box::new(|p| {
                p.insert("until".into(), json!("soon"));
            }),
            "until",
        ),
        // The literal string "null" is refused, never read as "from now" (db/059's special case).
        (
            "from the string null",
            Box::new(|p| {
                p.insert("from".into(), json!("null"));
            }),
            "from",
        ),
        (
            "from offset-less",
            Box::new(|p| {
                p.insert("from".into(), json!("2030-01-01T00:00:00"));
            }),
            "from",
        ),
        (
            "from garbage",
            Box::new(|p| {
                p.insert("from".into(), json!("tonight"));
            }),
            "from",
        ),
    ];
    for (what, break_it, word) in &cases {
        let mut body = valid_withhold_body(&c, &kid, patient, med).await;
        break_it(body.payload.as_object_mut().unwrap());
        let err = db_msg(&seal_and_submit(&c, &sk, body).await.unwrap_err());
        assert!(
            err.contains("medication withhold:") && err.contains(word),
            "{what}: got: {err}"
        );
    }

    // No `from` and no t_effective: the start would be the HLC wall, which a fast peer can
    // push hours ahead — so the floor refuses rather than guess.
    let mut body = valid_withhold_body(&c, &kid, patient, med).await;
    body.t_effective = None;
    let err = db_msg(&seal_and_submit(&c, &sk, body).await.unwrap_err());
    assert!(
        err.contains("medication withhold:") && err.contains("t_effective"),
        "got: {err}"
    );
    assert_eq!(withhold_rows(&c, med).await, 0, "nothing was admitted");
}

/// A well-formed release of `wid` (stamped `t_effective`), for the release table to break.
async fn valid_release_body(
    c: &Client,
    kid: &str,
    patient: Uuid,
    med: Uuid,
    wid: Uuid,
) -> cairn_event::EventBody {
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let body = build_release_body(Uuid::now_v7(), med, patient, &[wid], None, kid, hlc);
    effective_at_wall(c, body).await
}

#[tokio::test]
async fn every_structural_release_refusal_holds_at_the_local_door() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let wid = withhold(&mut c, &sk, &kid, patient, med, "BP 85/50").await;
    use serde_json::json;

    let cases: Vec<StructuralCase> = vec![
        (
            "medication_id not a uuid",
            Box::new(|p| {
                p.insert("medication_id".into(), json!("nope"));
            }),
            "medication_id",
        ),
        (
            "releases not an array",
            Box::new(|p| {
                p.insert("releases".into(), json!("x"));
            }),
            "releases",
        ),
        (
            "releases holding null",
            Box::new(|p| {
                p.insert("releases".into(), json!([null]));
            }),
            "releases",
        ),
        (
            "releases holding a number",
            Box::new(|p| {
                p.insert("releases".into(), json!([5]));
            }),
            "releases",
        ),
        (
            "releases holding a non-uuid",
            Box::new(|p| {
                p.insert("releases".into(), json!(["nope"]));
            }),
            "releases",
        ),
    ];
    for (what, break_it, word) in &cases {
        let mut body = valid_release_body(&c, &kid, patient, med, wid).await;
        break_it(body.payload.as_object_mut().unwrap());
        let err = db_msg(&seal_and_submit(&c, &sk, body).await.unwrap_err());
        assert!(
            err.contains("medication release:") && err.contains(word),
            "{what}: got: {err}"
        );
    }

    // A release with no t_effective: its instant would be the HLC wall — refused.
    let mut body = valid_release_body(&c, &kid, patient, med, wid).await;
    body.t_effective = None;
    let err = db_msg(&seal_and_submit(&c, &sk, body).await.unwrap_err());
    assert!(
        err.contains("medication release:") && err.contains("t_effective"),
        "got: {err}"
    );
    let n: i64 = c
        .query_one("SELECT count(*) FROM medication_withhold_release", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "nothing was admitted");
}

#[tokio::test]
async fn a_missing_t_effective_is_refused_at_the_sync_door_too() {
    // Structural, so the same verdict on every node: a peer's (or a raw client's) body that
    // leaves the start to the HLC wall is refused at sync apply as well — forking nothing,
    // because every node refuses it.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;

    let w = build_withhold_body(
        Uuid::now_v7(),
        med,
        patient,
        &until_reviewed("NBM"),
        &kid,
        peer_hlc(wall),
    );
    let err = db_msg(&apply_remote(&c, &sk, &w).await.unwrap_err());
    assert!(
        err.contains("medication withhold:") && err.contains("t_effective"),
        "got: {err}"
    );
    let r = build_release_body(
        Uuid::now_v7(),
        med,
        patient,
        &[Uuid::now_v7()],
        None,
        &kid,
        peer_hlc(wall + 1),
    );
    let err = db_msg(&apply_remote(&c, &sk, &r).await.unwrap_err());
    assert!(
        err.contains("medication release:") && err.contains("t_effective"),
        "got: {err}"
    );
    assert_eq!(withhold_rows(&c, med).await, 0);
}

#[tokio::test]
async fn a_withhold_naming_another_charts_thread_is_refused_at_the_local_door() {
    // #192 via the shared patient guard: a thread belongs to one chart for life, so a W on
    // patient B naming patient A's drug is the wrong-chart error, refused where it is typed.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (_patient_a, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let patient_b = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, patient_b, 0).await;

    let err = withhold_err(&mut c, &sk, &kid, patient_b, med, &until_reviewed("NBM")).await;
    assert!(err.contains("patient cannot change"), "got: {err}");
    assert_eq!(withhold_rows(&c, med).await, 0);
}

#[tokio::test]
async fn a_withhold_synced_before_its_prescription_shows_once_the_prescription_lands() {
    // Offline-first order: the W reaches this node before the assert it names. While the
    // thread is an orphan it is on no line; once the assert lands the line must read
    // WITHHELD — never "givable" because the W came first.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
    let med = Uuid::now_v7(); // asserted only AFTER the withhold arrives

    // Started at a FIXED past instant, not at the HLC wall: earlier tests in this binary
    // push the shared node clock ahead of the physical one (peer events at wall + 1 s), and
    // a start stamped from that wall can be a moment in the future — pending, not in force.
    let local = db::next_hlc(&c, "test-node").await.unwrap();
    let mut w = build_withhold_body(
        Uuid::now_v7(),
        med,
        patient,
        &until_reviewed("K+ 6.1"),
        &kid,
        peer_hlc(local.wall),
    );
    w.t_effective = Some("2020-01-01T00:00:00Z".into());
    apply_remote(&c, &sk, &w)
        .await
        .expect("the orphan withhold applies");
    assert!(
        list_patient_medications(&c, patient)
            .await
            .unwrap()
            .rows
            .is_empty(),
        "no line yet: the assert has not arrived"
    );

    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let assert_body = build_assert_body(
        Uuid::now_v7(),
        med,
        patient,
        &sample_input(),
        &kid,
        hlc,
        None,
    );
    seal_and_submit(&c, &sk, assert_body)
        .await
        .expect("the prescription lands");

    let rows = list_patient_medications(&c, patient).await.unwrap().rows;
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].is_withheld(),
        "the earlier W holds the line: {:?}",
        rows[0].holds
    );
    assert_eq!(rows[0].holds[0].reason, "K+ 6.1");
}
