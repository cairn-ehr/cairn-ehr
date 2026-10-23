//! #848 — a medication event recorded for THIS chart can sit on a line of ANOTHER chart, and then
//! this chart's list has no line to show it on.
//!
//! Two ways in, both converge-and-flag by design (a node-local veto would fork the event set):
//! - a later-HLC assert of the same thread on another chart wins the overlay (db/031
//!   `medication_statement_apply`), so the thread MOVES: its statement, and so its line, is now
//!   on the other chart. db/031's guard flags it (`medication_patient_conflict_flag`, standing =
//!   this chart), but the list read conflicts only for lines it shows, so the drug vanished
//!   from this chart's list with nothing said — an anticoagulant gone without a word.
//! - a dose, correction or release recorded for this chart on another chart's thread (db/059's
//!   `medication_thread_stray_chart` names this chart as `asserted_patient`).
//!
//! The other chart's own list already shows such a line as a `patient_conflict` hazard (#836,
//! #846). These tests pin the half that was missing: the list of the chart the event was
//! recorded FOR says so (`PatientMedicationList::elsewhere`), and a line already on the list is
//! never repeated there.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_medication_view::targeting::withheld_reasons;
use cairn_medication_view::{ChartSet, ElsewhereLine, MedicationStatus};
use cairn_node::db;
use cairn_node::medication::elsewhere::read_elsewhere;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::{
    build_assert_body, build_cease_body, build_dose_change_body, build_withhold_body,
    CeaseMedicationInput, ChangeDoseInput,
};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;

/// The scenario #848 names: thread `med` asserted on chart A, then a peer's assert of the same
/// thread on chart B with a later HLC. The drug must stay visible from A.
#[tokio::test]
async fn a_thread_moved_by_a_later_assert_is_said_on_the_chart_it_left() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (a, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (b, _) = chart_with_one_med(&mut c, &sk, &kid).await;

    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    apply_peer_assert(&c, &sk, &kid, b, med, wall).await;

    let list_a = list_patient_medications(&c, a).await.unwrap();
    assert!(
        !list_a
            .rows
            .iter()
            .any(|r| r.members.iter().any(|m| m.medication_id == med)),
        "precondition: the overlay moved the thread off A's list"
    );
    assert_eq!(
        list_a.elsewhere,
        vec![metoprolol_on(med, b)],
        "A's list must say its drug now sits on B's line"
    );

    // B's side was already covered (#836): the line is there, a conflict hazard, and it is
    // never repeated as "elsewhere" on the very list that shows it.
    let list_b = list_patient_medications(&c, b).await.unwrap();
    let line = list_b
        .rows
        .iter()
        .find(|r| r.group_id == med)
        .expect("the thread's line is on B's list");
    assert!(line.wrong_chart.patient_conflict, "{:?}", line.wrong_chart);
    assert!(withheld_reasons(line).is_some());
    assert_eq!(list_b.elsewhere, vec![], "a listed line is not elsewhere");
}

/// The flag's standing side ALONE: A's claim on the thread was a stop, not an assert, so no
/// point-0 dose row names A and the stray view cannot see the move. A peer's stop recorded for
/// chart A arrives first (an orphan cessation claims the thread for A, db/031), then a peer's
/// assert claims it for chart B; db/031's guard flags it with A as `standing_patient`.
#[tokio::test]
async fn a_thread_claimed_here_by_a_stop_then_asserted_elsewhere_is_said_here() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (a, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (b, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    let moved = Uuid::now_v7();

    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let stop = CeaseMedicationInput {
        stopped: None,
        stopped_precision: None,
        reason: None,
    };
    let body = build_cease_body(Uuid::now_v7(), moved, a, &stop, &kid, peer_hlc(wall));
    apply_remote(&c, &sk, &body)
        .await
        .expect("an orphan stop is admitted");
    apply_peer_assert(&c, &sk, &kid, b, moved, wall + 1).await;

    let list_a = list_patient_medications(&c, a).await.unwrap();
    assert_eq!(
        list_a.elsewhere,
        vec![ElsewhereLine {
            status: Some(MedicationStatus::Ceased),
            ..metoprolol_on(moved, b)
        }],
        "A's stop sits on B's line, which A's stop also ended"
    );
}

/// A thread no chart lists (no assert has arrived anywhere) is not "on another chart" (PR review
/// B): A's own stop claims it, and a peer's dose for chart B on it is flagged (A, B), but the
/// line is on NO chart's list. Saying "on another chart" would be a precise untruth; it stays
/// unsaid until its assert arrives, the pre-#848 limit `PatientMedicationList` documents.
#[tokio::test]
async fn a_thread_no_chart_lists_is_not_said_to_be_elsewhere() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (a, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (b, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    let unasserted = Uuid::now_v7();

    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let stop = CeaseMedicationInput {
        stopped: None,
        stopped_precision: None,
        reason: None,
    };
    let body = build_cease_body(Uuid::now_v7(), unasserted, a, &stop, &kid, peer_hlc(wall));
    apply_remote(&c, &sk, &body)
        .await
        .expect("an orphan stop is admitted");
    apply_peer_dose(&c, &sk, &kid, unasserted, b, wall + 1).await;

    assert_eq!(
        list_patient_medications(&c, a).await.unwrap().elsewhere,
        vec![]
    );
    assert_eq!(
        list_patient_medications(&c, b).await.unwrap().elsewhere,
        vec![]
    );
}

/// The view's path, and read as the runtime role (every other test here runs as the schema
/// owner, so a missing grant would pass them all): a peer's dose recorded for chart A lands on a
/// thread that a later assert claims for chart B. It arrived first, so db/031's guard never
/// flagged it; only db/059's `medication_thread_stray_chart` sees it.
#[tokio::test]
async fn a_dose_for_this_chart_on_another_charts_thread_is_said_here() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (a, own) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (b, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    let theirs = Uuid::now_v7();

    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    apply_peer_dose(&c, &sk, &kid, theirs, a, wall).await;
    apply_peer_assert(&c, &sk, &kid, b, theirs, wall + 1).await;

    let txn = c.transaction().await.unwrap();
    txn.batch_execute("SET ROLE cairn_agent").await.unwrap();
    let list_a = list_patient_medications(&txn, a)
        .await
        .expect("the runtime role can read every relation the list reads");
    assert_eq!(list_a.elsewhere, vec![metoprolol_on(theirs, b)]);
    assert!(
        list_a.rows.iter().any(|r| r.group_id == own),
        "A's own line is untouched"
    );
}

/// The flag's other side: a W recorded for chart A on B's thread, after B's claim, is flagged
/// with A as `asserted_patient` (the stray view never sees a W).
#[tokio::test]
async fn a_w_for_this_chart_on_another_charts_thread_is_said_here() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (a, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (b, theirs) = chart_with_one_med(&mut c, &sk, &kid).await;

    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let body = effective_at_wall(
        &c,
        build_withhold_body(
            Uuid::now_v7(),
            theirs,
            a,
            &until_reviewed("BP 85/50"),
            &kid,
            peer_hlc(wall),
        ),
    )
    .await;
    apply_remote(&c, &sk, &body)
        .await
        .expect("the sync door admits it and flags it");

    let list_a = list_patient_medications(&c, a).await.unwrap();
    assert_eq!(list_a.elsewhere, vec![metoprolol_on(theirs, b)]);
}

/// A line the list SHOWS is never repeated as elsewhere, even when the event recorded for A sits
/// on a member thread the list does not show. Here A's and B's threads are one cross-chart line
/// (a peer's reconciliation, inserted as in `medication_read.rs`'s cross-patient test), and A's
/// dose lands on B's member: the line is on A's list already, a hazard said on the line itself.
#[tokio::test]
async fn a_line_on_the_list_is_not_said_again_through_another_member() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (a, own) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (_, theirs) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (key, other) = (own.min(theirs), own.max(theirs));
    c.execute(
        "INSERT INTO medication_group_member (medication_id, group_id) VALUES \
         ($1::text::uuid, $1::text::uuid), ($2::text::uuid, $1::text::uuid)",
        &[&key.to_string(), &other.to_string()],
    )
    .await
    .unwrap();

    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    apply_peer_dose(&c, &sk, &kid, theirs, a, wall).await;

    let list_a = list_patient_medications(&c, a).await.unwrap();
    assert!(
        list_a.rows.iter().any(|r| r.group_id == key),
        "precondition: the cross-chart line is on A's list"
    );
    assert_eq!(list_a.elsewhere, vec![], "said on the line, not again");
}

/// A clean chart carries nothing elsewhere: the read is one query that finds no row.
#[tokio::test]
async fn a_clean_chart_has_nothing_elsewhere() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (a, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (b, theirs) = chart_with_one_med(&mut c, &sk, &kid).await;
    // B's own dose on B's own thread names no other chart.
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    apply_peer_dose(&c, &sk, &kid, theirs, b, wall).await;

    assert_eq!(
        list_patient_medications(&c, a).await.unwrap().elsewhere,
        vec![]
    );
    assert_eq!(
        list_patient_medications(&c, b).await.unwrap().elsewhere,
        vec![]
    );
}

/// #851 review: "on another chart" must hold inside the one statement, not depend on the
/// earlier member read seeing the same data (the list's reads are separate READ COMMITTED
/// statements). Here the thread now sits on B; reading B's elsewhere with EMPTY listed
/// arguments — an earlier read that ran before the move landed — must still not call B's own
/// line "on chart B", which would tell the clinician to record a second line.
#[tokio::test]
async fn a_line_on_this_charts_set_is_never_said_elsewhere_even_from_a_stale_list_read() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (_a, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (b, _) = chart_with_one_med(&mut c, &sk, &kid).await;

    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    apply_peer_assert(&c, &sk, &kid, b, med, wall).await;

    let stale = read_elsewhere(&c, &ChartSet::single(b), &[], &[])
        .await
        .unwrap();
    assert_eq!(stale, vec![], "B's own line is never B's 'elsewhere'");
}

/// A linked set opened from its OTHER member (#851 review): A1's drug moved to B must be said on
/// the combined list opened from A2 too, or the #848 silence comes back through that door.
#[tokio::test]
async fn a_moved_thread_is_said_on_a_linked_set_opened_from_the_other_member() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (a1, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (a2, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (b, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    common::submit_link_event(&c, &sk, &kid, a1, a2, 10, true).await;

    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    apply_peer_assert(&c, &sk, &kid, b, med, wall).await;

    let list = list_patient_medications(&c, a2).await.unwrap();
    assert_eq!(
        list.charts,
        ChartSet::new([a1, a2]).unwrap(),
        "precondition"
    );
    assert_eq!(list.elsewhere, vec![metoprolol_on(med, b)]);
}

/// The other half: a thread moved BETWEEN two linked charts never left the combined list, so it
/// is not "elsewhere" — a false report there would ask for a second line of a listed drug.
#[tokio::test]
async fn a_thread_moved_within_a_linked_set_is_not_said_elsewhere() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (a1, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (a2, _) = chart_with_one_med(&mut c, &sk, &kid).await;
    common::submit_link_event(&c, &sk, &kid, a1, a2, 10, true).await;

    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    apply_peer_assert(&c, &sk, &kid, a2, med, wall).await;

    for opened in [a1, a2] {
        let list = list_patient_medications(&c, opened).await.unwrap();
        assert!(
            list.rows.iter().any(|r| r.group_id == med),
            "the line is still on the combined list (opened {opened})"
        );
        assert_eq!(list.elsewhere, vec![], "opened {opened}");
    }
}

/// The kit's drug (`sample_input`), current, as an elsewhere line `group` on `chart`.
fn metoprolol_on(group: Uuid, chart: Uuid) -> ElsewhereLine {
    ElsewhereLine {
        group_id: group,
        name: Some("metoprolol".into()),
        status: Some(MedicationStatus::Active),
        charts: vec![chart],
    }
}

/// A peer's dose change on thread `med`, written against `chart`, through the sync door.
async fn apply_peer_dose(
    c: &Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    med: Uuid,
    chart: Uuid,
    wall: i64,
) {
    let dose = ChangeDoseInput {
        dose_amount: Some("50"),
        dose_unit: Some("mg"),
        effective: Some("2025-01-01"),
        effective_precision: Some("day"),
        info_source: "clinician-observed",
        reason: None,
    };
    let body = build_dose_change_body(Uuid::now_v7(), med, chart, &dose, kid, peer_hlc(wall));
    apply_remote(c, sk, &body)
        .await
        .expect("a dose is admitted, and flagged when it names another chart");
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
        .expect("the assert is admitted (converge-and-flag)");
}

/// `medication-list`'s two text branches (rows, and "no medications recorded" — joined by "but",
/// or it stands as an absence claim) each print the report, and `--json` says it on stderr as
/// well as in `.elsewhere`. No CLI test drives the verb's text, so this pins the calls: drop one,
/// and an empty chart reads as one with nothing on it.
#[test]
fn both_text_branches_of_medication_list_print_the_report() {
    let main = include_str!("../src/main.rs");
    assert_eq!(
        main.matches("print_elsewhere(&list, \"! but \");").count(),
        1
    );
    assert_eq!(main.matches("print_elsewhere(&list, \"! \");").count(), 1);
    assert_eq!(main.matches("eprintln!(\"warning: {report}\");").count(), 1);
}
