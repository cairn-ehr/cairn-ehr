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
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::{build_release_body, build_withhold_body};
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
