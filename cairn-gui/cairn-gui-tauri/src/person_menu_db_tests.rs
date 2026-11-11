//! DB-gated tests for the person menu's commands (on `CAIRN_TEST_PG`, run locally: CI never runs
//! these). They reuse "Co-sign as reviewed"'s fixture: Dr Sue supervises Dr Bea, Bea has signed a
//! note, and who's here names Sue, so Bea's note sits in Sue's review group.
use super::*;
use crate::notes::review::db_tests::{credential, supervised_note};
use crate::notes::review::note_review_cosign_impl;
use crate::person_menu_view::PersonMenuView;

/// The rows of one menu, flattened: patient ids, group by group (own, co-sign, review).
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

/// A re-pick between draw and click: Sue's menu was drawn (her row admitted to `shown`), then who's
/// here names Zed, who has no rows. "Open note" with Sue's chart is refused, and no chart is set.
#[tokio::test]
async fn a_repicked_person_cannot_open_the_previous_persons_row() {
    let Some((_guard, _owner, state, chart, _part)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    *state.chart.lock().await = None;
    let drawn = person_menu_impl(&state).await.unwrap();
    assert_eq!(row_charts(&drawn), [chart.to_string()]);
    state
        .whos_here
        .lock()
        .await
        .pick(crate::whos_here::Claim::Named {
            staff_id: "zed".into(),
            display_name: "Dr Zed".into(),
        });
    let err = person_menu_open_impl(&state, &chart.to_string())
        .await
        .unwrap_err();
    assert_eq!(err, NO_LONGER_WAITS);
    assert!(state.chart.lock().await.is_none());
}

/// Batch charting: Sue's two unsigned medication lines on one chart are ONE waiting item, counted by
/// the bar and the picker alike, and "Open note" on it opens that chart.
#[tokio::test]
async fn unsigned_medication_lines_wait_in_the_bar_and_the_picker() {
    let Some((_guard, owner, state, chart, _part)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    {
        let db = state.db.as_ref().unwrap().lock().await.unwrap();
        for _ in 0..2 {
            let (line, chart) = (Uuid::now_v7().to_string(), chart.to_string());
            let saved: String = db
                .query_one(
                    "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'fixture-terminal', 'sue', 'started-here', 'amoxicillin', '[{\"term\":\"amoxicillin\",\"coding\":null}]'::jsonb, NULL, false, '[]'::jsonb, false, '', '')",
                    &[&line, &chart],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(saved, "saved");
        }
    }
    let PersonMenuView::Menu { count, own, .. } = person_menu_impl(&state).await.unwrap() else {
        panic!("Sue is named");
    };
    assert_eq!(count, 1, "two lines on one chart are one item");
    assert_eq!(own.rows[0].state, "2 unsigned medication line(s)");
    let picker = whos_here_counts_impl(&state).await.unwrap();
    assert_eq!(
        picker.iter().find(|c| c.staff_id == "sue").unwrap().count,
        1
    );
    *state.chart.lock().await = None;
    person_menu_open_impl(&state, &chart.to_string())
        .await
        .expect("the medication row names the chart");
    assert!(state.chart.lock().await.is_some());
    // The shared test database keeps working state across suites; leave none behind for them.
    owner
        .batch_execute("TRUNCATE medication_draft_line")
        .await
        .unwrap();
}
