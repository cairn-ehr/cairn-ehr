//! Batch charting slice 2a, Task 4 — the draft list's source, why, set-aside mark and "From:" row (db/078).
//!
//! Split from `medication_draft_list_store.rs` (size cap). Same rules: the window's own login,
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
mod common;

use cairn_node::db;
use common::{note_setup, owner_and_window};
use serde_json::json;
use tokio_postgres::Client;
use uuid::Uuid;

/// (owner, window, chart) with a clean list table.
async fn fixture(base: &str) -> (Client, Client, Uuid) {
    let (owner, window) = owner_and_window(base).await;
    let (_, _, _, _, chart) = note_setup(&owner).await;
    owner
        .batch_execute("TRUNCATE medication_draft_line, medication_draft_from")
        .await
        .unwrap();
    (owner, window, chart)
}

// ---- Batch charting slice 2a, Task 4: the source, the why, the set-aside mark, the "From:" row ----

/// Save with the four slice-2a columns (and `held`) named; everything else as `save` defaults.
#[allow(clippy::too_many_arguments)]
async fn save_v(
    c: &Client,
    line: Uuid,
    chart: Uuid,
    text: &str,
    ticked: bool,
    held: bool,
    extra: [Option<&str>; 4], // info_source, source_detail, why, set_aside_of
) -> String {
    let parts = json!([{"term": text, "coding": null}]).to_string();
    let against =
        json!([{"source": "allergy", "ref": "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01"}]).to_string();
    let (line, chart) = (line.to_string(), chart.to_string());
    c.query_one(
        "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', 'started-here', $3, $4::text::jsonb, NULL, $5, $6::text::jsonb, false, '', '', $7, $8, $9, $10, $11)",
        &[&line, &chart, &text, &parts, &ticked, &against, &held, &extra[0], &extra[1], &extra[2], &extra[3]],
    )
    .await
    .expect("the save door answers")
    .get(0)
}

/// One stored line's (hand_ticked, held, info_source, source_detail, why, set_aside_of), read by door.
async fn row_of(
    c: &Client,
    chart: Uuid,
    line: Uuid,
) -> (
    bool,
    bool,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let r = c
        .query_one(
            "SELECT hand_ticked, held, info_source, source_detail, why, set_aside_of \
               FROM cairn_med_draft_lines($1::text::uuid, 'notewriter') WHERE line_id = $2::text::uuid",
            &[&chart.to_string(), &line.to_string()],
        )
        .await
        .unwrap();
    (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5))
}

#[tokio::test]
async fn sources_and_why_read_back_byte_for_byte() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_o, w, chart) = fixture(&base).await;
    let l = Uuid::now_v7();
    let detail = "  Dr \u{00e9} Smith <b>\n";
    let why = " for the cough\t ";
    assert_eq!(
        save_v(
            &w,
            l,
            chart,
            "a",
            false,
            false,
            [Some("external-record"), Some(detail), Some(why), None]
        )
        .await,
        "saved"
    );
    let r = row_of(&w, chart, l).await;
    assert_eq!(r.2.as_deref(), Some("external-record"));
    assert_eq!(r.3.as_deref(), Some(detail));
    assert_eq!(r.4.as_deref(), Some(why));
    assert_eq!(r.5, None);
    // An unknown category is refused by the floor.
    let bad = w
        .query_one(
            "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', 'started-here', 'x', '[]'::jsonb, NULL, false, '[]'::jsonb, false, '', '', false, 'hearsay')",
            &[&Uuid::now_v7().to_string(), &chart.to_string()],
        )
        .await;
    assert!(bad.is_err(), "an unknown info_source must be refused");
}

#[tokio::test]
async fn a_source_or_why_edit_clears_a_hand_tick_in_the_door() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_o, w, chart) = fixture(&base).await;
    let base_x: [Option<&str>; 4] = [Some("patient-reported"), Some("d"), Some("w"), None];
    let companion = Uuid::now_v7();
    save_v(&w, companion, chart, "c", false, false, [None; 4]).await;
    let set_aside = format!("line:{companion}");
    let edits: [[Option<&str>; 4]; 4] = [
        [Some("clinician-observed"), Some("d"), Some("w"), None],
        [Some("patient-reported"), Some("d2"), Some("w"), None],
        [Some("patient-reported"), Some("d"), Some("w2"), None],
        [
            Some("patient-reported"),
            Some("d"),
            Some("w"),
            Some(set_aside.as_str()),
        ],
    ];
    for edit in edits {
        let l = Uuid::now_v7();
        save_v(&w, l, chart, "a", false, false, base_x).await;
        assert_eq!(
            save_v(&w, l, chart, "a", true, false, base_x).await,
            "saved"
        );
        assert!(
            row_of(&w, chart, l).await.0,
            "the tick stands on unchanged content"
        );
        // A veto stands too, on a fresh line, and a provenance edit keeps it (ruling R15).
        let h = Uuid::now_v7();
        save_v(&w, h, chart, "a", false, true, base_x).await;
        assert!(row_of(&w, chart, h).await.1);
        // Now the edit arrives with the tick re-asserted by a stale client.
        save_v(&w, l, chart, "a", true, false, edit).await;
        save_v(&w, h, chart, "a", false, true, edit).await;
        let (ticked, ..) = row_of(&w, chart, l).await;
        assert!(!ticked, "{edit:?} must clear the tick in the door");
        assert!(
            row_of(&w, chart, h).await.1,
            "{edit:?} must keep the veto (ruling R15)"
        );
    }
}

#[tokio::test]
async fn a_set_aside_line_is_never_hand_ticked() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_o, w, chart) = fixture(&base).await;
    let (l, m) = (Uuid::now_v7(), format!("med:{}", Uuid::now_v7()));
    // Inserted already set aside, ticked: the tick is refused.
    save_v(
        &w,
        l,
        chart,
        "a",
        true,
        false,
        [None, None, None, Some(m.as_str())],
    )
    .await;
    assert!(!row_of(&w, chart, l).await.0);
    // Re-saved unchanged, ticked again: still no tick.
    save_v(
        &w,
        l,
        chart,
        "a",
        true,
        false,
        [None, None, None, Some(m.as_str())],
    )
    .await;
    assert!(!row_of(&w, chart, l).await.0);
}

/// Controller ruling R1: un-setting-aside through the SAVE door leaves the line held, as the delete
/// door's unmark does, so a version the clinician thought wrong never becomes default-ticked.
#[tokio::test]
async fn clearing_a_set_aside_mark_leaves_the_line_held() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_o, w, chart) = fixture(&base).await;
    let (l, m) = (Uuid::now_v7(), format!("med:{}", Uuid::now_v7()));
    save_v(
        &w,
        l,
        chart,
        "a",
        false,
        false,
        [None, None, None, Some(m.as_str())],
    )
    .await;
    // The clearing save even asks for a tick: the door still leaves it held and unticked.
    save_v(&w, l, chart, "a", true, false, [None; 4]).await;
    let r = row_of(&w, chart, l).await;
    assert_eq!((r.0, r.1, r.5), (false, true, None));
}

#[tokio::test]
async fn deleting_a_companion_clears_its_versions_marks() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_o, w, chart) = fixture(&base).await;
    let (x, v, other) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    save_v(&w, x, chart, "x", false, false, [None; 4]).await;
    let mark = format!("line:{x}");
    save_v(
        &w,
        v,
        chart,
        "v",
        false,
        false,
        [None, None, None, Some(mark.as_str())],
    )
    .await;
    save_v(
        &w,
        other,
        chart,
        "o",
        false,
        false,
        [
            None,
            None,
            None,
            Some("med:0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01"),
        ],
    )
    .await;
    let del: String = w
        .query_one("SELECT cairn_med_draft_line_delete($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', (SELECT s.saved_at FROM cairn_med_draft_lines($2::text::uuid, 'notewriter') s WHERE s.line_id = $1::text::uuid))", &[&x.to_string(), &chart.to_string()])
        .await
        .unwrap()
        .get(0);
    assert_eq!(del, "deleted");
    let r = row_of(&w, chart, v).await;
    assert_eq!((r.1, r.5), (true, None), "the version is unmarked and held");
    assert!(
        row_of(&w, chart, other).await.5.is_some(),
        "an unrelated mark is untouched"
    );
}

#[tokio::test]
async fn the_from_row_survives_and_is_per_writer() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_o, w, chart) = fixture(&base).await;
    let c = chart.to_string();
    for (who, preset, detail) in [
        (Some("notewriter"), "Patient", Some("  said so\n")),
        (Some("nurse-b"), "GP letter", None),
        (None, "Other", Some("x")),
    ] {
        let a: String = w
            .query_one(
                "SELECT cairn_med_draft_from_save($1::text::uuid, $2, $3, $4)",
                &[&c, &who, &preset, &detail],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(a, "saved");
    }
    // A second save replaces, not appends.
    w.query_one(
        "SELECT cairn_med_draft_from_save($1::text::uuid, ' Notewriter ', 'Patient', 'again')",
        &[&c],
    )
    .await
    .unwrap();
    let read = |who: Option<&'static str>| {
        let (w, c) = (&w, &c);
        async move {
            w.query(
                "SELECT preset, detail FROM cairn_med_draft_from($1::text::uuid, $2)",
                &[c, &who],
            )
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get::<_, Option<String>>(0), r.get::<_, Option<String>>(1)))
            .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        read(Some("notewriter")).await,
        vec![(Some("Patient".into()), Some("again".into()))]
    );
    assert_eq!(
        read(Some("nurse-b")).await,
        vec![(Some("GP letter".into()), None)]
    );
    assert_eq!(
        read(None).await,
        vec![(Some("Other".into()), Some("x".into()))]
    );
}

#[tokio::test]
async fn the_window_role_reaches_the_from_row_only_through_its_doors() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, w, _chart) = fixture(&base).await;
    let err = w
        .query("SELECT * FROM medication_draft_from", &[])
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );
    let n: i64 = owner
        .query_one(
            "SELECT count(*) FROM pg_proc p WHERE p.proname IN ('cairn_med_draft_from_save', 'cairn_med_draft_from') \
             AND has_function_privilege('cairn_window', p.oid, 'EXECUTE') \
             AND NOT has_function_privilege('public', p.oid, 'EXECUTE')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 2);
}

/// Ruling R6: the preset is the EXACT label of the list, or NULL; anything else is refused by the floor.
#[tokio::test]
async fn the_from_row_refuses_an_unknown_preset() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_o, w, chart) = fixture(&base).await;
    let c = chart.to_string();
    for bad in ["patient", "Hearsay", ""] {
        let r = w
            .query_one(
                "SELECT cairn_med_draft_from_save($1::text::uuid, 'notewriter', $2, NULL)",
                &[&c, &bad],
            )
            .await;
        assert!(r.is_err(), "{bad:?} must be refused");
    }
    let none: Option<&str> = None;
    w.query_one(
        "SELECT cairn_med_draft_from_save($1::text::uuid, 'notewriter', $2, NULL)",
        &[&c, &none],
    )
    .await
    .expect("an unset preset is allowed");
}

/// Controller ruling R15: a change of the line's source or why is not a change of the DRUG, so it
/// clears a hand tick (the tick vouched for the old provenance) but never the clinician's untick:
/// an unticked line stays unticked when a why is added. Only the drug words clear a veto.
#[tokio::test]
async fn a_why_edit_keeps_an_untick() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_o, w, chart) = fixture(&base).await;
    let l = Uuid::now_v7();
    save_v(&w, l, chart, "a", false, true, [None; 4]).await;
    assert!(row_of(&w, chart, l).await.1, "the untick is stored");
    for edit in [
        [None, None, Some("packet 25 mg"), None],
        [
            Some("external-record"),
            Some("GP letter"),
            Some("packet 25 mg"),
            None,
        ],
    ] {
        assert_eq!(save_v(&w, l, chart, "a", false, true, edit).await, "saved");
        assert!(row_of(&w, chart, l).await.1, "{edit:?} keeps the untick");
    }
    // The drug words still clear it (their own content: the veto covered the old drug).
    save_v(&w, l, chart, "b", false, true, [None; 4]).await;
    assert!(!row_of(&w, chart, l).await.1, "new words clear the untick");
}

/// Delete one line through the door, spending the saved_at it currently has (as Sign does).
async fn delete_line(c: &Client, chart: Uuid, line: Uuid) -> String {
    c.query_one("SELECT cairn_med_draft_line_delete($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', (SELECT s.saved_at FROM cairn_med_draft_lines($2::text::uuid, 'notewriter') s WHERE s.line_id = $1::text::uuid))", &[&line.to_string(), &chart.to_string()])
        .await
        .unwrap()
        .get(0)
}

/// How many "From:" rows (0 or 1) the writer has for the chart.
async fn from_rows(c: &Client, chart: Uuid, who: &str) -> usize {
    c.query(
        "SELECT 1 FROM cairn_med_draft_from($1::text::uuid, $2)",
        &[&chart.to_string(), &who],
    )
    .await
    .unwrap()
    .len()
}

/// Ruling R22: "From: Packets seen" given at Monday's admission must not silently stamp Friday's new
/// lines. Every Sign spends its lines through the delete door, so when the writer's LAST line on the
/// chart leaves, the door drops their "From:" row too (another writer's row is theirs, untouched).
#[tokio::test]
async fn the_last_line_leaving_resets_from() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_o, w, chart) = fixture(&base).await;
    let l = Uuid::now_v7();
    save_v(&w, l, chart, "a", false, false, [None; 4]).await;
    for who in ["notewriter", "nurse-b"] {
        w.query_one(
            "SELECT cairn_med_draft_from_save($1::text::uuid, $2, 'Packets / Webster pack seen', NULL)",
            &[&chart.to_string(), &who],
        )
        .await
        .unwrap();
    }
    assert_eq!(delete_line(&w, chart, l).await, "deleted");
    assert_eq!(
        from_rows(&w, chart, "notewriter").await,
        0,
        "From: is reset"
    );
    assert_eq!(
        from_rows(&w, chart, "nurse-b").await,
        1,
        "another writer's row stays"
    );
}

/// ...and while a line of the writer's remains on the chart, their "From:" row stays.
#[tokio::test]
async fn from_survives_while_a_line_remains() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_o, w, chart) = fixture(&base).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    save_v(&w, a, chart, "a", false, false, [None; 4]).await;
    save_v(&w, b, chart, "b", false, false, [None; 4]).await;
    w.query_one(
        "SELECT cairn_med_draft_from_save($1::text::uuid, 'notewriter', 'GP letter', 'Dr X')",
        &[&chart.to_string()],
    )
    .await
    .unwrap();
    assert_eq!(delete_line(&w, chart, a).await, "deleted");
    assert_eq!(
        from_rows(&w, chart, "notewriter").await,
        1,
        "a line remains: From: stays"
    );
}
