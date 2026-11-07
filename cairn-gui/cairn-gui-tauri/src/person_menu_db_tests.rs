//! DB-gated tests for the person menu's commands (on `CAIRN_TEST_PG`, run locally: CI never runs
//! these). They reuse "Co-sign as reviewed"'s fixture: Dr Sue supervises Dr Bea, Bea has signed a
//! note, and who's here names Sue, so Bea's note sits in Sue's review group.
use super::*;
use crate::notes::review::db_tests::{credential, supervised_note};
use crate::notes::review::note_review_cosign_impl;
use crate::person_menu_view::PersonMenuView;

/// The rows of one menu, flattened: (group, patient id).
fn row_charts(v: &PersonMenuView) -> Vec<String> {
    match v {
        PersonMenuView::Menu {
            own,
            cosign,
            review,
            ..
        } => [own, cosign, review]
            .iter()
            .flat_map(|g| g.rows.iter().map(|r| r.patient_id.clone()))
            .collect(),
        PersonMenuView::Nobody => vec![],
    }
}

/// Review Focus 5: a row drawn, then cleared by someone's act (Sue co-signs it as reviewed) before
/// the click. "Open note" re-reads, finds no such row, says so in the exact words, and sets no
/// chart.
#[tokio::test]
async fn a_row_that_vanished_is_refused_and_opens_no_chart() {
    let Some((_guard, _owner, state, chart, part)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let drawn = person_menu_impl(&state).await.unwrap();
    assert_eq!(
        row_charts(&drawn),
        [chart.to_string()],
        "Bea's note is listed"
    );

    let (p, root) = (chart.to_string(), part.to_string());
    note_review_cosign_impl(
        &state,
        &p,
        &root,
        std::slice::from_ref(&root),
        credential("sue"),
    )
    .await
    .expect("Sue reviews it (the fixture has the chart open, as a review needs)");
    *state.chart.lock().await = None;

    let err = person_menu_open_impl(&state, &p).await.unwrap_err();
    assert_eq!(err, NO_LONGER_WAITS);
    assert!(state.chart.lock().await.is_none(), "no chart was set");
}

/// A row that still waits opens its chart, through `open_chart_impl` (it was admitted to `shown`).
#[tokio::test]
async fn open_note_opens_the_rows_chart() {
    let Some((_guard, _owner, state, chart, _part)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    *state.chart.lock().await = None;
    person_menu_impl(&state).await.unwrap();
    person_menu_open_impl(&state, &chart.to_string())
        .await
        .expect("the row still waits");
    assert!(state.chart.lock().await.is_some(), "the chart is open");
}

/// Review Focus 4: Sue has one draft of her own waiting (counted) and one review row (not
/// counted); the bar says 1 and the picker's count for Sue says 1.
#[tokio::test]
async fn the_pickers_count_equals_the_bars() {
    let Some((_guard, _owner, state, chart, _part)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    {
        let db = state.db.as_ref().unwrap().lock().await.unwrap();
        cairn_node::note::draft::save_part(
            &*db,
            Uuid::now_v7(),
            chart,
            "fixture-terminal",
            Some("sue"),
            "will sign later",
            None,
            None,
        )
        .await
        .unwrap();
    }
    let bar = match person_menu_impl(&state).await.unwrap() {
        PersonMenuView::Menu { count, review, .. } => {
            assert_eq!(
                review.rows.len(),
                1,
                "the review row is listed, not counted"
            );
            count
        }
        PersonMenuView::Nobody => panic!("Sue is named"),
    };
    let picker = whos_here_counts_impl(&state).await.unwrap();
    let sue = picker
        .iter()
        .find(|c| c.staff_id == "sue")
        .expect("Sue is in the picker");
    assert_eq!(bar, 1);
    assert_eq!(sue.count, bar);
    assert_eq!(sue.suffix, " · 1 waiting");
}
