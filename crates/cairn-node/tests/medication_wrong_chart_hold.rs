//! #836 — a medication event that arrives by SYNC naming a different chart than its thread's
//! own is admitted (converge-and-flag, db/031's `cairn_guard_medication_patient`: a node-local
//! veto would fork the event set). It is projected onto the thread anyway. For a withhold or a
//! release that means another person's W can show on this patient's drug, or this patient's
//! W can be ended by a release written for someone else. The flag
//! (`medication_patient_conflict_flag`) records it; these tests pin that the medication list
//! READS it. The line is a wrong-chart hazard (`WrongChartReasons::patient_conflict`), so it is
//! left out of sign-off and every renderer says why.
//!
//! The holds' display is deliberately not asserted: whether a peer-stamped instant is already
//! in force depends on how far earlier tests pushed the shared HLC (`effective_at_wall`). The
//! flag does not depend on it.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_medication_view::targeting::withheld_reasons;
use cairn_medication_view::MedicationRow;
use cairn_medication_view::MedicationStatus;
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::{
    build_assert_body, build_cease_body, build_dose_change_body, build_dose_correction_body,
    build_release_body, build_withhold_body, reconcile_medications, CeaseMedicationInput,
    ChangeDoseInput, CorrectDoseInput, ReconcileInput,
};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;

/// The line on `p`'s list whose members include thread `med`.
async fn line_of(c: &Client, p: Uuid, med: Uuid) -> MedicationRow {
    let list = list_patient_medications(c, p).await.unwrap();
    list.rows
        .into_iter()
        .find(|r| r.members.iter().any(|m| m.medication_id == med))
        .expect("the thread's line is on its own chart's list")
}

/// The line is a hazard for the conflict reason alone, and sign-off leaves it out.
fn assert_conflict_hazard(row: &MedicationRow) {
    assert!(row.wrong_chart.patient_conflict, "{:?}", row.wrong_chart);
    assert!(!row.wrong_chart.outside_set && !row.wrong_chart.doubted_link);
    assert!(row.is_wrong_chart_hazard() && row.cross_patient);
    assert!(
        withheld_reasons(row).is_some(),
        "an unsigned active conflict line is withheld from sign-off"
    );
}

#[tokio::test]
async fn a_synced_withhold_naming_another_chart_marks_the_line() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (other, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    assert!(
        !line_of(&c, p, med).await.is_wrong_chart_hazard(),
        "precondition: a clean line"
    );

    // A peer's W on p's thread, but authored against `other`'s chart.
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let body = effective_at_wall(
        &c,
        build_withhold_body(
            Uuid::now_v7(),
            med,
            other,
            &until_reviewed("BP 85/50"),
            &kid,
            peer_hlc(wall),
        ),
    )
    .await;
    apply_remote(&c, &sk, &body)
        .await
        .expect("the sync door admits it and flags it");

    assert_conflict_hazard(&line_of(&c, p, med).await);
}

#[tokio::test]
async fn a_synced_release_naming_another_chart_marks_the_line() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (other, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    let w = withhold(&mut c, &sk, &kid, p, med, "BP 85/50").await;

    // A peer's release of p's own W, but authored against `other`'s chart: without the flag
    // read, p's hold would end with nothing on the line to say a stranger's release did it.
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let body = effective_at_wall(
        &c,
        build_release_body(Uuid::now_v7(), med, other, &[w], None, &kid, peer_hlc(wall)),
    )
    .await;
    apply_remote(&c, &sk, &body)
        .await
        .expect("the sync door admits it and flags it");

    assert_conflict_hazard(&line_of(&c, p, med).await);
}

/// In a RECONCILED line the flag is on one member thread while the line is keyed by the group's
/// min-UUID representative (db/033 `medication_group_member`). Flag the NON-representative
/// member, so the read's group mapping is what is tested, not the unreconciled fallback.
#[tokio::test]
async fn a_conflict_on_a_reconciled_lines_other_member_marks_the_line() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, a) = chart_with_one_med(&mut c, &sk, &kid).await;
    let b = add_med(&mut c, &sk, &kid, p).await;
    let edge = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    reconcile_medications(&mut c, &sk, &kid, "test-node", p, a, b, &edge, None, None)
        .await
        .unwrap();
    let (other, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    let flagged = a.max(b); // the group is keyed by the smaller id
    let row = line_of(&c, p, flagged).await;
    assert_eq!(row.members.len(), 2, "precondition: one reconciled line");
    assert_ne!(
        row.group_id, flagged,
        "precondition: not the representative"
    );

    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let body = effective_at_wall(
        &c,
        build_withhold_body(
            Uuid::now_v7(),
            flagged,
            other,
            &until_reviewed("BP 85/50"),
            &kid,
            peer_hlc(wall),
        ),
    )
    .await;
    apply_remote(&c, &sk, &body)
        .await
        .expect("admitted and flagged");

    assert_conflict_hazard(&line_of(&c, p, flagged).await);
}

/// The most dangerous variant (PR #845 review): another person's STOP lands on this patient's
/// current drug. The line reads as ceased, so nothing withholds it — the flag is the only thing
/// left to say the stop may not be this patient's, and the ceased-line remedy is what speaks.
#[tokio::test]
async fn a_synced_stop_naming_another_chart_marks_the_ceased_line() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (other, _) = chart_with_one_med(&mut c, &sk, &kid).await;

    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let input = CeaseMedicationInput {
        stopped: None,
        stopped_precision: None,
        reason: None,
    };
    let body = build_cease_body(Uuid::now_v7(), med, other, &input, &kid, peer_hlc(wall));
    apply_remote(&c, &sk, &body)
        .await
        .expect("the sync door admits it and flags it");

    let row = line_of(&c, p, med).await;
    assert_eq!(row.status, MedicationStatus::Ceased, "the stray stop took");
    assert!(row.wrong_chart.patient_conflict, "{:?}", row.wrong_chart);
    assert!(row.is_wrong_chart_hazard());
    assert_eq!(
        withheld_reasons(&row),
        None,
        "a ceased line needs no signature"
    );
}

/// #846 — the guard flags an event only when its thread's chart is ALREADY known, so a stray
/// event that arrives FIRST (before anything ties its thread to a chart) is never flagged, and
/// the claim that arrives later does not look back at it. A dose (or a release, below) is not a
/// claim (db/031 `cairn_medication_thread_patient`), so this is the order the guard cannot see.
/// The list must find the contradiction whichever order the two arrived in.
#[tokio::test]
async fn a_stray_dose_that_arrived_before_its_threads_claim_marks_the_line() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (other, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    let med = Uuid::now_v7();

    // 1. A peer's dose change on thread `med`, written against `other`, arrives first.
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let dose = ChangeDoseInput {
        dose_amount: Some("50"),
        dose_unit: Some("mg"),
        effective: Some("2025-01-01"),
        effective_precision: Some("day"),
        info_source: "clinician-observed",
        reason: None,
    };
    let body = build_dose_change_body(Uuid::now_v7(), med, other, &dose, &kid, peer_hlc(wall));
    apply_remote(&c, &sk, &body)
        .await
        .expect("an orphan dose is admitted");

    // 2. The thread's own assert, for p, arrives later.
    apply_peer_assert(&c, &sk, &kid, p, med, wall + 1).await;

    assert_conflict_hazard(&line_of(&c, p, med).await);
}

/// #846, the release half: db/059 lets a release arrive before the W it names, so a stray
/// release can be the first thing on its thread too.
#[tokio::test]
async fn a_stray_release_that_arrived_before_its_threads_claim_marks_the_line() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (other, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    let med = Uuid::now_v7();

    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let body = effective_at_wall(
        &c,
        build_release_body(
            Uuid::now_v7(),
            med,
            other,
            &[Uuid::now_v7()],
            None,
            &kid,
            peer_hlc(wall),
        ),
    )
    .await;
    apply_remote(&c, &sk, &body)
        .await
        .expect("an orphan release is admitted");

    apply_peer_assert(&c, &sk, &kid, p, med, wall + 1).await;

    assert_conflict_hazard(&line_of(&c, p, med).await);
}

/// #846, the dose-correction arm: a correction may arrive before the dose it corrects (db/032
/// section 6), so it can be the first thing on its thread as well.
#[tokio::test]
async fn a_stray_dose_correction_that_arrived_before_its_threads_claim_marks_the_line() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (other, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    let med = Uuid::now_v7();

    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let fix = CorrectDoseInput {
        dose_amount: Some("50"),
        dose_unit: Some("mg"),
        effective: None,
        effective_precision: None,
        reason: Some("transcription error"),
        strike: &[],
        note: None,
        info_source: None,
    };
    let body = build_dose_correction_body(
        Uuid::now_v7(),
        med,
        other,
        Uuid::now_v7(),
        &fix,
        &kid,
        peer_hlc(wall),
    );
    apply_remote(&c, &sk, &body)
        .await
        .expect("an orphan dose correction is admitted");

    apply_peer_assert(&c, &sk, &kid, p, med, wall + 1).await;

    assert_conflict_hazard(&line_of(&c, p, med).await);
}

/// A peer's assert of thread `med` on chart `p`, through the sync door.
async fn apply_peer_assert(
    c: &Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    p: Uuid,
    med: Uuid,
    wall: i64,
) {
    let body = build_assert_body(
        Uuid::now_v7(),
        med,
        p,
        &sample_input(),
        kid,
        peer_hlc(wall),
        None,
    );
    apply_remote(c, sk, &body)
        .await
        .expect("the thread's assert is admitted");
}
