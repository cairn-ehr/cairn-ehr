//! The tray commands' tests (R5b): the pure entry and admission rules, in fixture mode. Moved
//! out of `mod.rs` (#752) to keep it under 500 lines.
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
    let (view, admitted) = entry_of_item(&item_of(Err(why.clone()), o, n), &found, &Doubted::new());
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
        // Never echo the heading: it carries patient names, and CodeQL's rust/cleartext-logging
        // flags a test message that prints one (PR #795's alert).
        "the unread side's heading names the wrong chart"
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

/// #752 point 3: the ONE candidate read serves every shown chart, so when it fails every side
/// shows its error — which must say what was being read, not only the operator chain. **Pure.**
#[test]
fn a_failed_names_read_says_what_was_being_read() {
    let text = names_read_error(anyhow::anyhow!("connection reset"));
    assert!(
        text.starts_with("reading the names of the shown charts"),
        "{text}"
    );
    assert!(text.ends_with("connection reset"), "{text}");
}

/// …and the shared read actually goes through it: reverting the call site to a bare
/// `operator_chain` passes the test above, so the source is read (the window has no DB-backed
/// tests, #738).
#[test]
fn the_names_read_is_worded_through_names_read_error() {
    let body = crate::source_guard::rust_fn(include_str!("mod.rs"), "worklist_impl");
    crate::source_guard::position(body, ".map_err(names_read_error)");
}

/// #752 points 1 and 2, the webview's half. The tray's two reads are ordered by their own
/// counters, an answer that is no longer the newest is dropped BEFORE anything is drawn, and the
/// stale list goes as soon as a read starts. A return to the front door empties an open tray's
/// list while the count is read — and must also make any list read still in flight stale, or that
/// older answer lands during the count's await and redraws the entries the clear just removed,
/// with Reviews `close_chart` has made unopenable (PR #798 review). Reading worklist.js is the
/// only way to pin this: the webview has no test rig (no npm, by design).
#[test]
fn the_trays_reads_are_ordered_and_its_stale_list_never_returns() {
    use crate::source_guard::{js_fn, position};
    let js = include_str!("../../src-ui/worklist.js");

    let refresh = js_fn(js, "refreshTray");
    let count = position(refresh, "await invoke(\"duplicate_tray_count\")");
    let open = &refresh[..count];
    let open = &open[position(open, "if (el(\"duplicate-tray\").open)")..];
    assert!(
        position(open, "trayListRevision += 1;") < position(open, "clearTrayList();"),
        "{refresh}"
    );
    assert!(
        position(refresh, "if (revision !== trayCountRevision) return;") > count,
        "{refresh}"
    );

    let load = js_fn(js, "loadTray");
    let read = position(load, "await invoke(\"duplicate_worklist\")");
    assert!(position(load, "clearTrayList();") < read, "{load}");
    let dropped = position(load, "if (revision !== trayListRevision) return;");
    assert!(
        dropped > read && dropped < position(load, ".map(trayItem)"),
        "{load}"
    );
    // The tray's drawing fault reaches the console too, with its stack.
    assert!(load.contains("console.error(failure)"), "{load}");
}
