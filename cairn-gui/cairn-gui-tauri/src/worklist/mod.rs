//! The front door's possible-duplicate tray (repair path R5b, #680): its two commands. Every
//! sentence is in `view.rs`; every DB rule in `cairn_node::duplicate_review::worklist` (DB-tested
//! there). This module orders the reads, builds each side's person row through the SAME candidate
//! read the search uses (`candidate_read::candidates_by_id`), and admits every shown chart to
//! `AppState::shown` — the worklist IS a list on screen, so the funnel's "only a chart a list
//! showed can be opened" rule holds unchanged. Review is the existing `open_chart`.
//!
//! LOCKING: one hold of `state.db` per command; nothing here calls `read_chart_of` /
//! `chart_set_of` (which take the lock themselves).
pub mod view;

use crate::duplicates::view::EntryFlags;
use crate::funnel::rows::person_row_view;
use crate::state::AppState;
use cairn_node::db_diagnosis::operator_chain;
use cairn_node::duplicate_check::{classify, read_snapshot, CheckState, STALLED_AFTER_SECS};
use cairn_node::duplicate_review::worklist::{worklist, worklist_count};
use cairn_node::patient::candidate_read::candidates_by_id;
use cairn_patient_search::{Candidate, PersonRow};
use view::{entry_view, tray_count_view, worklist_view, TrayCountView, WorklistView, MAX_SHOWN};

pub async fn tray_count_impl(state: &AppState) -> TrayCountView {
    let Some(db) = state.db.as_ref() else {
        return tray_count_view(Ok(1), Ok(CheckState::NeverRun { waiting: 0 }));
    };
    let db = db.lock().await;
    let count = worklist_count(&*db).await.map_err(|e| operator_chain(&e));
    let status = read_snapshot(&db)
        .await
        .map(|s| classify(&s, STALLED_AFTER_SECS))
        .map_err(|e| operator_chain(&e));
    tray_count_view(count, status)
}

pub async fn worklist_impl(state: &AppState) -> WorklistView {
    let Some(db) = state.db.as_ref() else {
        let (newer, older) = fixture_pair();
        admit(state, newer.iter().chain(older.iter())).await;
        let entry = entry_view(
            EntryFlags::default(),
            "review",
            Ok(PersonRow::new(newer.clone()).map(|r| person_row_view(&r))),
            Ok(PersonRow::new(older).map(|r| person_row_view(&r))),
            Some(newer[0].patient_id.to_string()),
        );
        return worklist_view(Ok((vec![entry], 1)));
    };
    let db = db.lock().await;
    let read = async {
        let today: String = db.query_one("SELECT current_date::text", &[]).await?.get(0);
        let list = worklist(&*db, MAX_SHOWN).await?;
        let mut entries = vec![];
        let mut admitted: Vec<Candidate> = vec![];
        for item in &list.items {
            let newer = read_side(&db, &item.newer, &today)
                .await
                .map_err(|e| operator_chain(&e));
            let older = read_side(&db, &item.older, &today)
                .await
                .map_err(|e| operator_chain(&e));
            let open_chart = newer.is_ok().then(|| item.entry.open_chart.to_string());
            for cands in [&newer, &older].into_iter().flatten() {
                admitted.extend(cands.iter().cloned());
            }
            let flags = EntryFlags {
                vetoed: item.entry.vetoed,
                accepted: item.entry.accepted,
                disputed: item.entry.disputed,
            };
            entries.push(entry_view(
                flags,
                &item.entry.band,
                newer.map(|c| PersonRow::new(c).map(|r| person_row_view(&r))),
                older.map(|c| PersonRow::new(c).map(|r| person_row_view(&r))),
                open_chart,
            ));
        }
        Ok::<_, anyhow::Error>((entries, list.total, admitted))
    }
    .await;
    drop(db);
    match read {
        Err(e) => worklist_view(Err(operator_chain(&e))),
        Ok((entries, total, admitted)) => {
            admit(state, admitted.iter()).await;
            worklist_view(Ok((entries, total)))
        }
    }
}

/// One side of an entry: every chart of that record, as the search would show it. A failure is
/// that side's alone — the caller words it, and the other side and the other entries stand.
async fn read_side(
    db: &tokio_postgres::Client,
    set: &cairn_medication_view::ChartSet,
    today: &str,
) -> anyhow::Result<Vec<Candidate>> {
    candidates_by_id(db, set.members(), today).await
}

/// Put the tray's charts on the list of charts `open_chart` will open.
async fn admit<'a>(state: &AppState, cands: impl Iterator<Item = &'a Candidate>) {
    let mut shown = state.shown.lock().await;
    for c in cands {
        shown.insert(c.patient_id, c.clone());
    }
}

/// `--mock`: one entry from the fixture population — FIXTURE_UUID as the newer record (it opens to
/// the fixture chart), the next fixture as the one on file. The mock has no link model (#722).
pub(crate) fn fixture_pair() -> (Vec<Candidate>, Vec<Candidate>) {
    let pop = cairn_gui_data::mock::fixtures::starting_population();
    let cand = |i: usize| Candidate {
        patient_id: pop[i].uuid,
        display_name: pop[i].display_name.clone(),
        age: None,
        trust: pop[i].trust,
        last_activity: None,
        locale: None,
        photo_ref: None,
    };
    (vec![cand(0)], vec![cand(1)])
}

#[tauri::command]
pub async fn duplicate_tray_count(state: tauri::State<'_, AppState>) -> Result<TrayCountView, ()> {
    Ok(tray_count_impl(&state).await)
}

#[tauri::command]
pub async fn duplicate_worklist(state: tauri::State<'_, AppState>) -> Result<WorklistView, ()> {
    Ok(worklist_impl(&state).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixture-mode window (`--mock`): no database, so every command takes its fixture arm.
    fn fixture_state() -> AppState {
        AppState::mock(Some(
            cairn_gui_data::mock::fixtures::FIXTURE_UUID
                .parse()
                .unwrap(),
        ))
    }

    #[tokio::test]
    async fn fixture_mode_counts_one_and_says_the_check_never_ran() {
        let state = fixture_state();
        let v = tray_count_impl(&state).await;
        assert_eq!(v.summary.as_deref(), Some("Possible duplicates (1)"));
        assert!(v.status_line.is_some());
    }

    /// Review Focus 5: the list re-admits its charts every time it is read — `close_chart`
    /// cleared `shown`, and the tray's Review must still open the newer record.
    #[tokio::test]
    async fn reading_the_list_admits_review_s_chart_again_after_close() {
        let state = fixture_state();
        let list = worklist_impl(&state).await;
        let open = list.entries[0]
            .open_chart
            .clone()
            .expect("fixture newer side is read");
        crate::funnel::commands::close_chart_impl(&state).await; // clears `shown`
        assert!(crate::funnel::commands::open_chart_impl(&state, &open)
            .await
            .is_err());
        worklist_impl(&state).await;
        assert!(crate::funnel::commands::open_chart_impl(&state, &open)
            .await
            .is_ok());
    }
}
