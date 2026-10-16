//! The front door's possible-duplicate tray (repair path R5b, #680): its two commands. Every
//! sentence is in `view.rs`; every DB rule in `cairn_node::duplicate_review::worklist` (DB-tested
//! there). This module orders the reads, builds each side's person row through
//! `candidate_read::candidates_by_id` — the same display read (`read_display_facts`) and the same
//! rendering (`DisplayFacts::candidate`) the search uses — and admits every shown chart to
//! `AppState::shown` — the worklist IS a list on screen, so the funnel's "only a chart a list
//! showed can be opened" rule holds unchanged. Review is the existing `open_chart`.
//!
//! LOCKING: one hold of `state.db` per command; nothing here calls `read_chart_of` /
//! `chart_set_of` (which take the lock themselves). `state.db` is the connection the funnel's
//! search shares, and the list is re-read on every return to the front door while the tray is
//! open, so its statement count is bounded by the number of SHOWN entries, never the backlog:
//! the node's list read and flags read (one statement each), two record reads per shown entry
//! (at most `2 × MAX_SHOWN`), then ONE candidate read (`read_display_facts`, a fixed handful of
//! statements) for every chart of every shown entry (`charts_of`), split per side afterwards
//! (`entry_of_item`, pure) — never a candidate read per side. The one per-side read is db/054's
//! doubt boolean (`doubted_sides`), asked only of a LINKED side (also at most `2 × MAX_SHOWN`):
//! a side whose links are in doubt never reads "One person" (#723, PR #777 review).
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

/// Whether db/054 doubts a link among each LINKED side's charts, keyed by that side's member
/// charts (sorted, as `ChartSet` holds them) — or why it could not be read. A single-chart side
/// has no entry: it holds no link.
type Doubted = HashMap<Vec<Uuid>, Result<bool, String>>;

/// The tray's `<summary>` count and the duplicate check's status line, in one hold of `state.db`.
/// `--mock` (no database) counts the fixture's one entry and says the check never ran, so the
/// tray is visible in a fixture walk. Every failed read is worded by `tray_count_view`.
pub async fn tray_count_impl(state: &AppState) -> TrayCountView {
    let Some(db) = state.db.as_ref() else {
        return tray_count_view(Ok(1), Ok(CheckState::NeverRun { waiting: 0 }));
    };
    let db = db.lock().await;
    // Status BEFORE count (PR #788 review; pinned by duplicates/read_order_tests.rs): the worker,
    // from its own process, never deletes a notice before the proposals it caused are committed.
    // Counted first, a pair can fall between the reads — a count of none, then "nothing waiting".
    let status = read_snapshot(&db)
        .await
        .map(|s| classify(&s, STALLED_AFTER_SECS))
        .map_err(|e| operator_chain(&e));
    let count = worklist_count(&*db).await.map_err(|e| operator_chain(&e));
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
            Ok(PersonRow::new(older.clone()).map(|r| person_row_view(&r))),
            (newer[0].patient_id, older[0].patient_id),
            Some(newer[0].patient_id.to_string()),
        );
        return worklist_view(Ok((vec![entry], 1)));
    };
    let db = db.lock().await;
    let read = async {
        let today: String = db.query_one("SELECT current_date::text", &[]).await?.get(0);
        let list = worklist(&*db, MAX_SHOWN).await?;
        let doubted = doubted_sides(&*db, &list.items).await;
        let found: Found = candidates_by_id(&*db, &charts_of(&list.items), &today)
            .await
            .map(|cands| cands.into_iter().map(|c| (c.patient_id, c)).collect())
            .map_err(|e| operator_chain(&e));
        Ok::<_, anyhow::Error>((list, found, doubted))
    }
    .await;
    drop(db);
    match read {
        Err(e) => worklist_view(Err(operator_chain(&e))),
        Ok((list, found, doubted)) => {
            let mut entries = vec![];
            let mut admitted: Vec<Candidate> = vec![];
            for item in &list.items {
                let (entry, cands) = entry_of_item(item, &found, &doubted);
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

/// db/054's doubt boolean for every linked read side of `items`, once per distinct record. A
/// failed read is kept per side, worded (operator chain), so it becomes that side's error —
/// never "not doubted".
async fn doubted_sides<C: tokio_postgres::GenericClient + Sync>(
    client: &C,
    items: &[WorklistItem],
) -> Doubted {
    let mut doubted = Doubted::new();
    let linked = items
        .iter()
        .flat_map(|i| [&i.newer, &i.older])
        .filter_map(|side| side.as_ref().ok())
        .filter(|set| set.is_linked());
    for set in linked {
        if doubted.contains_key(set.members()) {
            continue;
        }
        let read = cairn_node::patient::edges::record_is_doubted(client, set)
            .await
            .map_err(|e| operator_chain(&e));
        doubted.insert(set.members().to_vec(), read);
    }
    doubted
}

/// One side's candidates, taken from the shared read, and whether its links are in doubt: the
/// record's error if the node could not read it, else the candidate read's error, else the doubt
/// read's error (a linked side whose doubt is unread is an error, never "not doubted"), else
/// every member chart found. **Pure.**
fn side_candidates(
    side: &Result<ChartSet, String>,
    found: &Found,
    doubted: &Doubted,
) -> Result<(Vec<Candidate>, bool), String> {
    let set = side.as_ref().map_err(Clone::clone)?;
    let found = found.as_ref().map_err(Clone::clone)?;
    let in_doubt = if set.is_linked() {
        match doubted.get(set.members()) {
            Some(read) => read.clone()?,
            None => return Err("whether a link among its charts is in doubt was not read".into()),
        }
    } else {
        false
    };
    let cands = set
        .members()
        .iter()
        .filter_map(|id| found.get(id).cloned())
        .collect();
    Ok((cands, in_doubt))
}

/// One entry's view and the charts it puts on screen (to admit). **Pure.** `open_chart` is `Some`
/// only when that chart is among the newer side's candidates — i.e. it will be in
/// `AppState::shown` — so Review is never offered for a chart `open_chart` would refuse.
fn entry_of_item(
    item: &WorklistItem,
    found: &Found,
    doubted: &Doubted,
) -> (WorklistEntryView, Vec<Candidate>) {
    let newer = side_candidates(&item.newer, found, doubted);
    let older = side_candidates(&item.older, found, doubted);
    let open = item.entry.open_chart;
    let open_chart = newer
        .as_ref()
        .is_ok_and(|(c, _)| c.iter().any(|c| c.patient_id == open))
        .then(|| open.to_string());
    let admitted: Vec<Candidate> = [&newer, &older]
        .into_iter()
        .flatten()
        .flat_map(|(c, _)| c)
        .cloned()
        .collect();
    let flags = EntryFlags {
        vetoed: item.entry.vetoed,
        accepted: item.entry.accepted,
        disputed: item.entry.disputed,
    };
    let row = |(c, in_doubt): (Vec<Candidate>, bool)| {
        PersonRow::new(c).map(|r| person_row_view(&r.with_doubt(in_doubt)))
    };
    let view = entry_view(
        flags,
        &item.entry.band,
        newer.map(row),
        older.map(row),
        (item.entry.open_chart, item.entry.older_chart),
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

    /// The list re-admits its charts every time it is read — `close_chart`
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

    /// A record the node could not read is that side's worded error — never a failed list,
    /// never a blank side — and Review is withheld, because that chart was never admitted.
    #[test]
    fn an_unreadable_newer_record_is_worded_and_review_is_withheld() {
        let (newer, older) = fixture_pair();
        let (n, o) = (newer[0].patient_id, older[0].patient_id);
        let found: Found = Ok(older.iter().map(|c| (c.patient_id, c.clone())).collect());
        let why = format!("reading the record of chart {n}: boom");
        let (view, admitted) =
            entry_of_item(&item_of(Err(why.clone()), o, n), &found, &Doubted::new());
        assert_eq!(view.newer.row, None);
        assert_eq!(
            view.newer.error,
            Some(format!("This record could not be read here: {why}"))
        );
        assert_eq!(view.open_chart, None);
        // The unread side is headed by ITS OWN chart, the newer one: a swapped
        // `(open_chart, older_chart)` at the call would name the other patient's chart here.
        assert!(
            view.heading
                .starts_with(&format!("Chart {n} — possibly the same person as "))
                && view.heading.ends_with(&format!("chart {o}")),
            "{}",
            view.heading
        );
        assert!(view.older.row.is_some(), "the other side still stands");
        assert_eq!(
            admitted.iter().map(|c| c.patient_id).collect::<Vec<_>>(),
            vec![o]
        );
    }

    /// ONE candidate read serves every side — each side takes its own record's charts from
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
        let (view, admitted) = entry_of_item(&item, &found, &Doubted::new());
        assert_eq!(view.open_chart, Some(n.to_string()));
        assert!(view.newer.row.is_some() && view.older.row.is_some());
        assert_eq!(admitted.len(), 2);
        // A failed candidate read words BOTH sides and withholds Review.
        let (failed, none) = entry_of_item(&item, &Err("down".into()), &Doubted::new());
        assert_eq!(
            failed.newer.error.as_deref(),
            Some("This record could not be read here: down")
        );
        assert_eq!(failed.open_chart, None);
        assert!(none.is_empty());
    }

    /// A side of two linked charts, both found: the newer record `n`+`o` (Review opens `n`),
    /// against an older record `x` the candidate read did not return (so its row is empty —
    /// these tests are about the newer side).
    fn linked_newer() -> (WorklistItem, Found, Vec<Uuid>) {
        let (newer, older) = fixture_pair();
        let (n, o) = (newer[0].patient_id, older[0].patient_id);
        let x = Uuid::from_u128(77);
        let set = cairn_medication_view::ChartSet::new([n, o]).unwrap();
        let members = set.members().to_vec();
        let found: Found = Ok(newer
            .iter()
            .chain(older.iter())
            .map(|c| (c.patient_id, c.clone()))
            .collect());
        (item_of(Ok(set), x, n), found, members)
    }

    /// #723 on the duplicate-review tray (PR #777 review): a side whose links db/054 doubts never
    /// reads "One person — N linked charts". A reviewer deciding whether two records are the same
    /// person must not be told, as fact, that a disputed set already is one.
    #[test]
    fn a_doubted_linked_side_never_says_one_person() {
        let (item, found, members) = linked_newer();
        let doubted: Doubted = [(members.clone(), Ok(true))].into_iter().collect();
        let (view, _) = entry_of_item(&item, &found, &doubted);
        let label = view.newer.row.expect("read").label;
        assert_eq!(label, Some(crate::funnel::rows::doubted_row_label(2)));

        let undoubted: Doubted = [(members, Ok(false))].into_iter().collect();
        let (view, _) = entry_of_item(&item, &found, &undoubted);
        let label = view.newer.row.expect("read").label;
        assert_eq!(label, Some(crate::funnel::rows::linked_row_label(2)));
    }

    /// An unread doubt is the side's worded error — never "not doubted", which would put "One
    /// person" back on a disputed set — and Review is withheld with it.
    #[test]
    fn an_unread_doubt_is_worded_never_assumed_absent() {
        let (item, found, members) = linked_newer();
        let failed: Doubted = [(members, Err("doubt read: boom".into()))]
            .into_iter()
            .collect();
        let (view, _) = entry_of_item(&item, &found, &failed);
        assert_eq!(view.newer.row, None);
        assert_eq!(
            view.newer.error.as_deref(),
            Some("This record could not be read here: doubt read: boom")
        );
        assert_eq!(view.open_chart, None);

        let (view, _) = entry_of_item(&item, &found, &Doubted::new());
        assert_eq!(view.newer.row, None, "a linked side nobody asked about");
        assert!(view.newer.error.is_some());
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
            Ok(PersonRow::new(older.clone()).map(|r| person_row_view(&r))),
            (newer[0].patient_id, older[0].patient_id),
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
        // #796: losing either read draws a nameless heading or hides the entry's status.
        let entry_reads = fields_read_in(js, "entry");
        assert!(entry_reads.contains("heading") && entry_reads.contains("status"));
    }
}
