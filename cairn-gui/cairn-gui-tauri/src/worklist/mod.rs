//! The front door's possible-duplicate tray (repair path R5b, #680): its two commands. Every
//! sentence is in `view.rs`; every DB rule in `cairn_node::duplicate_review::worklist` (DB-tested
//! there). This module orders the reads, builds each side's person row through the SAME candidate
//! read the search uses (`candidate_read::candidates_by_id`), and admits every shown chart to
//! `AppState::shown` — the worklist IS a list on screen, so the funnel's "only a chart a list
//! showed can be opened" rule holds unchanged. Review is the existing `open_chart`.
//!
//! LOCKING: one hold of `state.db` per command; nothing here calls `read_chart_of` /
//! `chart_set_of` (which take the lock themselves). `state.db` is the connection the funnel's
//! search shares, and the list is re-read on every return to the front door while the tray is
//! open, so its statement count is bounded: the node's few reads plus ONE candidate read for
//! every chart of every shown entry (`charts_of`), split per side afterwards
//! (`entry_of_item`, pure) — never a read per side.
pub mod view;

use crate::duplicates::view::EntryFlags;
use crate::funnel::rows::person_row_view;
use crate::state::AppState;
use cairn_medication_view::ChartSet;
use cairn_node::db_diagnosis::operator_chain;
use cairn_node::duplicate_check::{classify, read_snapshot, CheckState, STALLED_AFTER_SECS};
use cairn_node::duplicate_review::worklist::{worklist, worklist_count, WorklistItem};
use cairn_node::patient::candidate_read::candidates_by_id;
use cairn_patient_search::{Candidate, PersonRow};
use std::collections::HashMap;
use uuid::Uuid;
use view::{
    entry_view, tray_count_view, worklist_view, TrayCountView, WorklistEntryView, WorklistView,
    MAX_SHOWN,
};

/// The ONE candidate read's result, by chart id — or why it failed (worded, operator chain).
type Found = Result<HashMap<Uuid, Candidate>, String>;

/// The tray's `<summary>` count and the duplicate check's status line, in one hold of `state.db`.
/// `--mock` (no database) counts the fixture's one entry and says the check never ran, so the
/// tray is visible in a fixture walk. Every failed read is worded by `tray_count_view`.
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

/// The tray's open list: the newest `MAX_SHOWN` entries, each side as the search would show it.
///
/// Flow: under ONE hold of `state.db`, the node's worklist read and ONE `candidates_by_id` for all
/// shown charts; then, with the lock released, each entry is built (`entry_of_item`, pure) and
/// its charts admitted to `AppState::shown` so Review can open them. A failed LIST read is the
/// list's error line; a failed RECORD or candidate read is a side's worded error.
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
        let found: Found = candidates_by_id(&*db, &charts_of(&list.items), &today)
            .await
            .map(|cands| cands.into_iter().map(|c| (c.patient_id, c)).collect())
            .map_err(|e| operator_chain(&e));
        Ok::<_, anyhow::Error>((list, found))
    }
    .await;
    drop(db);
    match read {
        Err(e) => worklist_view(Err(operator_chain(&e))),
        Ok((list, found)) => {
            let mut entries = vec![];
            let mut admitted: Vec<Candidate> = vec![];
            for item in &list.items {
                let (entry, cands) = entry_of_item(item, &found);
                entries.push(entry);
                admitted.extend(cands);
            }
            admit(state, admitted.iter()).await;
            worklist_view(Ok((entries, list.total)))
        }
    }
}

/// Every chart of every READ side of `items`, sorted, once each — the ids of the ONE candidate
/// read. A side whose record could not be read contributes nothing. **Pure.**
fn charts_of(items: &[WorklistItem]) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = items
        .iter()
        .flat_map(|i| [&i.newer, &i.older])
        .filter_map(|side| side.as_ref().ok())
        .flat_map(|set| set.members().iter().copied())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// One side's candidates, taken from the shared read: the record's error if the node could not
/// read it, else the candidate read's error, else every member chart found. **Pure.**
fn side_candidates(
    side: &Result<ChartSet, String>,
    found: &Found,
) -> Result<Vec<Candidate>, String> {
    let set = side.as_ref().map_err(Clone::clone)?;
    let found = found.as_ref().map_err(Clone::clone)?;
    Ok(set
        .members()
        .iter()
        .filter_map(|id| found.get(id).cloned())
        .collect())
}

/// One entry's view and the charts it puts on screen (to admit). **Pure.** `open_chart` is `Some`
/// only when that chart is among the newer side's candidates — i.e. it will be in
/// `AppState::shown` — so Review is never offered for a chart `open_chart` would refuse.
fn entry_of_item(item: &WorklistItem, found: &Found) -> (WorklistEntryView, Vec<Candidate>) {
    let newer = side_candidates(&item.newer, found);
    let older = side_candidates(&item.older, found);
    let open = item.entry.open_chart;
    let open_chart = newer
        .as_ref()
        .is_ok_and(|c| c.iter().any(|c| c.patient_id == open))
        .then(|| open.to_string());
    let admitted: Vec<Candidate> = [&newer, &older]
        .into_iter()
        .flatten()
        .flatten()
        .cloned()
        .collect();
    let flags = EntryFlags {
        vetoed: item.entry.vetoed,
        accepted: item.entry.accepted,
        disputed: item.entry.disputed,
    };
    let row = |c: Vec<Candidate>| PersonRow::new(c).map(|r| person_row_view(&r));
    let view = entry_view(
        flags,
        &item.entry.band,
        newer.map(row),
        older.map(row),
        open_chart,
    );
    (view, admitted)
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

/// Tauri forwarder for [`tray_count_impl`]; never `Err` — every failure is a worded line.
#[tauri::command]
pub async fn duplicate_tray_count(state: tauri::State<'_, AppState>) -> Result<TrayCountView, ()> {
    Ok(tray_count_impl(&state).await)
}

/// Tauri forwarder for [`worklist_impl`]; never `Err` — every failure is a worded line.
#[tauri::command]
pub async fn duplicate_worklist(state: tauri::State<'_, AppState>) -> Result<WorklistView, ()> {
    Ok(worklist_impl(&state).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_node::duplicate_review::worklist::WorklistEntry;

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

    /// An entry built from `--mock`'s two fixture charts, the newer side as `newer`.
    fn item_of(
        newer: Result<cairn_medication_view::ChartSet, String>,
        older: Uuid,
        open_chart: Uuid,
    ) -> WorklistItem {
        let entry = WorklistEntry {
            newer_record: open_chart,
            older_record: older,
            open_chart,
            older_chart: older,
            pairs: vec![(open_chart.min(older), open_chart.max(older))],
            band: "review".into(),
            vetoed: false,
            disputed: false,
            accepted: false,
            newest_ms: 1,
        };
        WorklistItem {
            entry,
            newer,
            older: Ok(cairn_medication_view::ChartSet::new([older]).unwrap()),
        }
    }

    /// M8: a record the node could not read is that side's worded error — never a failed list,
    /// never a blank side — and Review is withheld, because that chart was never admitted.
    #[test]
    fn an_unreadable_newer_record_is_worded_and_review_is_withheld() {
        let (newer, older) = fixture_pair();
        let (n, o) = (newer[0].patient_id, older[0].patient_id);
        let found: Found = Ok(older.iter().map(|c| (c.patient_id, c.clone())).collect());
        let why = format!("reading the record of chart {n}: boom");
        let (view, admitted) = entry_of_item(&item_of(Err(why.clone()), o, n), &found);
        assert_eq!(view.newer.row, None);
        assert_eq!(
            view.newer.error,
            Some(format!("This record could not be read here: {why}"))
        );
        assert_eq!(view.open_chart, None);
        assert!(view.older.row.is_some(), "the other side still stands");
        assert_eq!(
            admitted.iter().map(|c| c.patient_id).collect::<Vec<_>>(),
            vec![o]
        );
    }

    /// I5: ONE candidate read serves every side — each side takes its own record's charts from
    /// it, and Review opens the newer record's chart.
    #[test]
    fn one_candidate_read_is_split_per_side() {
        let (newer, older) = fixture_pair();
        let (n, o) = (newer[0].patient_id, older[0].patient_id);
        let item = item_of(Ok(cairn_medication_view::ChartSet::new([n]).unwrap()), o, n);
        assert_eq!(charts_of(std::slice::from_ref(&item)), {
            let mut v = vec![n, o];
            v.sort();
            v
        });
        let found: Found = Ok(newer
            .iter()
            .chain(older.iter())
            .map(|c| (c.patient_id, c.clone()))
            .collect());
        let (view, admitted) = entry_of_item(&item, &found);
        assert_eq!(view.open_chart, Some(n.to_string()));
        assert!(view.newer.row.is_some() && view.older.row.is_some());
        assert_eq!(admitted.len(), 2);
        // A failed candidate read words BOTH sides and withholds Review.
        let (failed, none) = entry_of_item(&item, &Err("down".into()));
        assert_eq!(
            failed.newer.error.as_deref(),
            Some("This record could not be read here: down")
        );
        assert_eq!(failed.open_chart, None);
        assert!(none.is_empty());
    }

    /// worklist.js is untyped: a Rust field rename would draw an empty tray, not break the build.
    #[test]
    fn worklist_js_reads_no_field_the_backend_does_not_send() {
        use crate::commands::tests::fields_read_in;
        let js = include_str!("../../src-ui/worklist.js");
        let keys = |v: serde_json::Value| -> std::collections::BTreeSet<String> {
            v.as_object().unwrap().keys().cloned().collect()
        };
        let (newer, older) = fixture_pair();
        let row = crate::funnel::rows::person_row_view(&PersonRow::new(newer.clone()).unwrap());
        let entry = view::entry_view(
            EntryFlags::default(),
            "review",
            Ok(Some(row.clone())),
            Ok(PersonRow::new(older).map(|r| person_row_view(&r))),
            None,
        );
        let counted = tray_count_view(Ok(1), Ok(CheckState::NeverRun { waiting: 0 }));
        let list = worklist_view(Ok((vec![entry.clone()], 1)));
        let member = crate::funnel::view::candidate_view(&newer[0]);
        for (binding, available) in [
            ("counted", keys(serde_json::to_value(&counted).unwrap())),
            ("list", keys(serde_json::to_value(&list).unwrap())),
            ("entry", keys(serde_json::to_value(&entry).unwrap())),
            ("side", keys(serde_json::to_value(&entry.newer).unwrap())),
            ("row", keys(serde_json::to_value(&row).unwrap())),
            ("member", keys(serde_json::to_value(&member).unwrap())),
        ] {
            let read = fields_read_in(js, binding);
            assert!(
                !read.is_empty(),
                "worklist.js no longer reads `{binding}` — rename it here"
            );
            for field in read {
                assert!(
                    available.contains(&field),
                    "worklist.js reads `{binding}.{field}`, not sent"
                );
            }
        }
    }
}
