//! Batch charting slice 1, Task 1 — the per-writer medication draft list store (db/078).
//!
//! The list is mutable, unsigned scratch space beside the record. These tests drive its SQL doors
//! under the window's OWN login (`cairn_window`) and pin the rules a hostile or merely racing
//! client must not get around: a hand tick is cleared by an edit IN THE DOOR, a second terminal
//! is told `moved` until it takes the list, another writer's line is `not-yours`, an unnamed
//! writer has its own bucket, and the window holds EXECUTE but no table privilege.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
mod common;

use cairn_node::db;
use common::{note_setup, owner_and_window};
use serde_json::{json, Value};
use tokio_postgres::Client;
use uuid::Uuid;

/// Save a line with the usual defaults; only what a test varies is a parameter.
/// Returns the door's one-word answer.
async fn save(
    c: &Client,
    line: Uuid,
    chart: Uuid,
    terminal: &str,
    writer: Option<&str>,
    text: &str,
    ticked: bool,
) -> String {
    // tokio-postgres here has no uuid/json codecs: ids and jsonb cross as text, cast in SQL.
    let parts = json!([{"term": text, "coding": null}]).to_string();
    let against =
        json!([{"source": "allergy", "ref": "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01"}]).to_string();
    let (line, chart) = (line.to_string(), chart.to_string());
    c.query_one(
        "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, $3, $4, 'started-here', $5, $6::text::jsonb, NULL, $7, $8::text::jsonb, false, '', '')",
        &[&line, &chart, &terminal, &writer, &text, &parts, &ticked, &against],
    )
    .await
    .expect("the save door answers")
    .get(0)
}

/// A second clinical member beside `note_setup`'s `notewriter` (placeholder sealed blob, derived key).
async fn nurse_b(owner: &Client) {
    owner
        .execute(
            "INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, \
                                       sealed, enrolled_by, contact_email) \
             VALUES ('nurse-b', 'Nurse B', 'nurse', '\\x00'::bytea, $1, '\\x00'::bytea, 'console', 'x@example.org')",
            &[&hex::encode([0xb2u8; 32])],
        )
        .await
        .unwrap();
}

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

#[tokio::test]
async fn a_saved_line_reads_back_byte_for_byte() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_owner, w, chart) = fixture(&base).await;
    let (l1, l2) = (Uuid::now_v7(), Uuid::now_v7());
    let raw = "  Augmentin 875/125 \u{00e9}  bd <b>x</b>\n";
    assert_eq!(
        save(&w, l1, chart, "pc-1", Some("notewriter"), raw, false).await,
        "saved"
    );
    assert_eq!(
        save(&w, l2, chart, "pc-1", Some(" Notewriter "), "second", false).await,
        "saved"
    );
    let rows = w
        .query("SELECT line_id::text, line_text, seq, parts::text FROM cairn_med_draft_lines($1::text::uuid, 'notewriter')", &[&chart.to_string()])
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<_, String>(0), l1.to_string());
    assert_eq!(rows[0].get::<_, String>(1), raw);
    assert_eq!((rows[0].get::<_, i32>(2), rows[1].get::<_, i32>(2)), (1, 2));
    assert_eq!(
        serde_json::from_str::<Value>(&rows[0].get::<_, String>(3)).unwrap(),
        json!([{"term": raw, "coding": null}])
    );
    // A chart with no events has no list to start.
    assert_eq!(
        save(
            &w,
            Uuid::now_v7(),
            Uuid::now_v7(),
            "pc-1",
            Some("notewriter"),
            "x",
            false
        )
        .await,
        "no-chart"
    );
    // And a stranger is not a writer.
    assert_eq!(
        save(
            &w,
            Uuid::now_v7(),
            chart,
            "pc-1",
            Some("nobody"),
            "x",
            false
        )
        .await,
        "no-such-writer"
    );
}

#[tokio::test]
async fn an_edit_after_a_hand_tick_clears_it_in_the_door() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_owner, w, chart) = fixture(&base).await;
    let line = Uuid::now_v7();
    save(
        &w,
        line,
        chart,
        "pc-1",
        Some("notewriter"),
        "warfarin 5 mg",
        true,
    )
    .await;
    let read = || async {
        let r = w
            .query_one("SELECT hand_ticked, ticked_against::text FROM cairn_med_draft_lines($1::text::uuid, 'notewriter')", &[&chart.to_string()])
            .await
            .unwrap();
        (
            r.get::<_, bool>(0),
            serde_json::from_str::<Value>(&r.get::<_, String>(1)).unwrap(),
        )
    };
    // Same content, tick kept.
    assert_eq!(
        save(
            &w,
            line,
            chart,
            "pc-1",
            Some("notewriter"),
            "warfarin 5 mg",
            true
        )
        .await,
        "saved"
    );
    assert!(read().await.0);
    // New text WITH p_hand_ticked = true: the door itself voids the tick.
    assert_eq!(
        save(
            &w,
            line,
            chart,
            "pc-1",
            Some("notewriter"),
            "warfarin 7.5 mg",
            true
        )
        .await,
        "saved"
    );
    assert_eq!(read().await, (false, json!([])));
}

#[tokio::test]
async fn a_second_terminal_answers_moved_until_it_takes() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_owner, w, chart) = fixture(&base).await;
    let line = Uuid::now_v7();
    save(&w, line, chart, "pc-1", Some("notewriter"), "a", false).await;
    assert_eq!(
        save(&w, line, chart, "pc-2", Some("notewriter"), "b", false).await,
        "moved"
    );
    let del: String = w
        .query_one(
            "SELECT cairn_med_draft_line_delete($1::text::uuid, $2::text::uuid, 'pc-2', 'notewriter', (SELECT s.saved_at FROM cairn_med_draft_lines($2::text::uuid, 'notewriter') s WHERE s.line_id = $1::text::uuid))",
            &[&line.to_string(), &chart.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(del, "moved");
    let taken: i32 = w
        .query_one(
            "SELECT cairn_med_draft_take($1::text::uuid, 'notewriter', 'pc-2')",
            &[&chart.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(taken, 1);
    assert_eq!(
        save(&w, line, chart, "pc-2", Some("notewriter"), "b", false).await,
        "saved"
    );
    // The old terminal is now the one told to stop; another chart answers elsewhere.
    assert_eq!(
        save(&w, line, chart, "pc-1", Some("notewriter"), "c", false).await,
        "moved"
    );
    assert_eq!(
        save(
            &w,
            line,
            Uuid::now_v7(),
            "pc-2",
            Some("notewriter"),
            "c",
            false
        )
        .await,
        "elsewhere"
    );
    let del = |t: &'static str| {
        let (w, chart) = (&w, chart);
        async move {
            w.query_one(
                "SELECT cairn_med_draft_line_delete($1::text::uuid, $2::text::uuid, $3, 'notewriter', (SELECT s.saved_at FROM cairn_med_draft_lines($2::text::uuid, 'notewriter') s WHERE s.line_id = $1::text::uuid))",
                &[&line.to_string(), &chart.to_string(), &t],
            )
            .await
            .unwrap()
            .get::<_, String>(0)
        }
    };
    assert_eq!(del("pc-2").await, "deleted");
    assert_eq!(del("pc-2").await, "gone");
}

#[tokio::test]
async fn another_writers_line_answers_not_yours() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, w, chart) = fixture(&base).await;
    nurse_b(&owner).await;
    let line = Uuid::now_v7();
    save(&w, line, chart, "pc-1", Some("notewriter"), "mine", false).await;
    assert_eq!(
        save(&w, line, chart, "pc-1", Some("nurse-b"), "theirs", false).await,
        "not-yours"
    );
    assert_eq!(
        save(&w, line, chart, "pc-1", None, "theirs", false).await,
        "not-yours"
    );
    let t: String = w
        .query_one(
            "SELECT line_text FROM cairn_med_draft_lines($1::text::uuid, 'notewriter')",
            &[&chart.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(t, "mine");
}

#[tokio::test]
async fn a_null_writer_list_is_its_own_bucket() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_owner, w, chart) = fixture(&base).await;
    save(&w, Uuid::now_v7(), chart, "pc-1", None, "unnamed 1", false).await;
    save(&w, Uuid::now_v7(), chart, "pc-1", None, "unnamed 2", false).await;
    save(
        &w,
        Uuid::now_v7(),
        chart,
        "pc-1",
        Some("notewriter"),
        "named",
        false,
    )
    .await;
    let unnamed = w
        .query(
            "SELECT seq FROM cairn_med_draft_lines($1::text::uuid, NULL)",
            &[&chart.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(
        unnamed
            .iter()
            .map(|r| r.get::<_, i32>(0))
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    let named = w
        .query(
            "SELECT seq FROM cairn_med_draft_lines($1::text::uuid, 'notewriter')",
            &[&chart.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(named.len(), 1);
    assert_eq!(named[0].get::<_, i32>(0), 1);
}

#[tokio::test]
async fn the_window_role_holds_execute_and_no_table_privilege() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, w, chart) = fixture(&base).await;
    save(
        &w,
        Uuid::now_v7(),
        chart,
        "pc-1",
        Some("notewriter"),
        "x",
        false,
    )
    .await;
    // The window's own login reads through a door but not the table.
    let err = w
        .query("SELECT * FROM medication_draft_line", &[])
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );
    // And the role itself, from the owner's session, holds EXECUTE on all five doors.
    let n: i64 = owner
        .query_one(
            "SELECT count(*) FROM pg_proc p WHERE p.proname IN ('cairn_med_draft_line_save', \
             'cairn_med_draft_lines', 'cairn_med_draft_line_delete', 'cairn_med_draft_take', \
             'cairn_med_drafts_by_writer') AND has_function_privilege('cairn_window', p.oid, 'EXECUTE') \
             AND NOT has_function_privilege('public', p.oid, 'EXECUTE')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 5);
    assert!(!owner
        .query_one(
            "SELECT has_table_privilege('cairn_window', 'medication_draft_line', 'SELECT')",
            &[]
        )
        .await
        .unwrap()
        .get::<_, bool>(0));
}

#[tokio::test]
async fn drafts_by_writer_counts_named_writers_only() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_owner, w, chart) = fixture(&base).await;
    for t in ["a", "b"] {
        save(
            &w,
            Uuid::now_v7(),
            chart,
            "pc-1",
            Some("notewriter"),
            t,
            false,
        )
        .await;
    }
    save(&w, Uuid::now_v7(), chart, "pc-1", None, "unnamed", false).await;
    let rows = w
        .query(
            "SELECT writer, patient_id::text, lines, newest FROM cairn_med_drafts_by_writer()",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "the unnamed list is not offered to anyone");
    assert_eq!(rows[0].get::<_, String>(0), "notewriter");
    assert_eq!(rows[0].get::<_, String>(1), chart.to_string());
    assert_eq!(rows[0].get::<_, i64>(2), 2);
    let _: chrono::DateTime<chrono::Utc> = rows[0].get(3);
}

#[tokio::test]
async fn a_save_after_delete_never_brings_the_line_back() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_owner, w, chart) = fixture(&base).await;
    let line = Uuid::now_v7();
    save(&w, line, chart, "pc-1", Some("notewriter"), "a", true).await;
    let (l, c) = (line.to_string(), chart.to_string());
    let del: String = w
        .query_one("SELECT cairn_med_draft_line_delete($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', (SELECT s.saved_at FROM cairn_med_draft_lines($2::text::uuid, 'notewriter') s WHERE s.line_id = $1::text::uuid))", &[&l, &c])
        .await
        .unwrap()
        .get(0);
    assert_eq!(del, "deleted");
    // The stale autosave, ticked, arrives after the delete.
    assert_eq!(
        save(&w, line, chart, "pc-1", Some("notewriter"), "a", true).await,
        "deleted"
    );
    let n = w
        .query(
            "SELECT 1 FROM cairn_med_draft_lines($1::text::uuid, 'notewriter')",
            &[&c],
        )
        .await
        .unwrap()
        .len();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn another_writers_delete_answers_not_yours() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, w, chart) = fixture(&base).await;
    nurse_b(&owner).await;
    let line = Uuid::now_v7();
    save(&w, line, chart, "pc-1", Some("notewriter"), "mine", false).await;
    let (l, c) = (line.to_string(), chart.to_string());
    for who in [Some("nurse-b"), None] {
        let r: String = w
            .query_one(
                "SELECT cairn_med_draft_line_delete($1::text::uuid, $2::text::uuid, 'pc-1', $3, (SELECT s.saved_at FROM cairn_med_draft_lines($2::text::uuid, 'notewriter') s WHERE s.line_id = $1::text::uuid))",
                &[&l, &c, &who],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(r, "not-yours");
    }
    assert_eq!(
        save(&w, line, chart, "pc-1", Some("notewriter"), "mine", false).await,
        "saved"
    );
}

#[tokio::test]
async fn only_a_content_change_clears_the_tick_and_the_parts_must_be_an_array() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_owner, w, chart) = fixture(&base).await;
    let (line, c) = (Uuid::now_v7(), chart.to_string());
    let l = line.to_string();
    let parts = json!([{"term": "x", "coding": null}]).to_string();
    let other = json!([{"term": "y", "coding": null}]).to_string();
    let against =
        json!([{"source": "allergy", "ref": "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01"}]).to_string();
    // (charted_as, parts, reason, finished) sent; whether the tick must survive.
    let cases: [(&str, &str, Option<&str>, bool, bool); 4] = [
        ("started-here", &parts, Some("why"), false, true), // reason only
        ("started-here", &parts, Some("why"), true, true),  // finished only
        ("started-here", &other, Some("why"), true, false), // parts change
        ("pre-existing", &other, Some("why"), true, false), // charted_as change
    ];
    for (charted, p, reason, finished, kept) in cases {
        // Re-establish a ticked baseline: same content as the previous step, tick sent.
        let base_row = w
            .query_opt("SELECT charted_as, parts::text FROM cairn_med_draft_lines($1::text::uuid, 'notewriter')", &[&c])
            .await
            .unwrap();
        let (bc, bp) = base_row
            .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
            .unwrap_or(("started-here".into(), parts.clone()));
        w.query_one(
            "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', $3, 'x', $4::text::jsonb, NULL, true, $5::text::jsonb, false, '', '')",
            &[&l, &c, &bc, &bp, &against],
        ).await.unwrap();
        let first = w
            .query_one(
                "SELECT hand_ticked FROM cairn_med_draft_lines($1::text::uuid, 'notewriter')",
                &[&c],
            )
            .await
            .unwrap()
            .get::<_, bool>(0);
        assert!(first, "baseline ticked");
        w.query_one(
            "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', $3, 'x', $4::text::jsonb, $5, true, $6::text::jsonb, $7, '', '')",
            &[&l, &c, &charted, &p, &reason, &against, &finished],
        ).await.unwrap();
        let now = w
            .query_one(
                "SELECT hand_ticked FROM cairn_med_draft_lines($1::text::uuid, 'notewriter')",
                &[&c],
            )
            .await
            .unwrap()
            .get::<_, bool>(0);
        assert_eq!(now, kept, "{charted} {p} {reason:?} {finished}");
    }
    // A non-array parts is refused at the door (the table's CHECK).
    let e = w
        .query_one(
            "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', 'started-here', 'x', '{}'::jsonb, NULL, false, '[]'::jsonb, false, '', '')",
            &[&Uuid::now_v7().to_string(), &c],
        )
        .await
        .unwrap_err();
    assert_eq!(
        e.code(),
        Some(&tokio_postgres::error::SqlState::CHECK_VIOLATION)
    );
}

/// A tick's `ref` must be uuid-shaped and its `source` a non-empty string: the floor refuses a
/// malformed element at the door (the node's reader tolerates old bad rows, but none is new).
#[tokio::test]
async fn a_malformed_tick_element_is_refused_by_the_floor() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_owner, w, chart) = fixture(&base).await;
    let parts = json!([{"term": "x", "coding": null}]).to_string();
    for bad in [
        json!([{"source": "allergy", "ref": "r1"}]),
        json!([{"source": "", "ref": "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01"}]),
        json!([{"ref": "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01"}]),
        json!(["x"]),
    ] {
        let r = w
            .query_one(
                "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', 'started-here', 'x', $3::text::jsonb, NULL, true, $4::text::jsonb, false, '', '')",
                &[&Uuid::now_v7().to_string(), &chart.to_string(), &parts, &bad.to_string()],
            )
            .await;
        assert!(r.is_err(), "{bad} must be refused");
    }
}

/// The person menu's feed: named writers only, the line count, and the newest save.
#[tokio::test]
async fn the_node_reader_decodes_drafts_by_writer() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_owner, w, chart) = fixture(&base).await;
    for _ in 0..2 {
        save(
            &w,
            Uuid::now_v7(),
            chart,
            "pc-1",
            Some("notewriter"),
            "a",
            false,
        )
        .await;
    }
    save(&w, Uuid::now_v7(), chart, "pc-1", None, "unnamed", false).await;
    let got = cairn_node::medication::draft::writer_drafts(&w)
        .await
        .unwrap();
    assert_eq!(got.len(), 1, "the unnamed bucket is not listed");
    assert_eq!(
        (got[0].0.as_str(), got[0].1, got[0].2),
        ("notewriter", chart, 2)
    );
    let newest: String = w
        .query_one(
            "SELECT max(saved_at)::text FROM cairn_med_draft_lines($1::text::uuid, 'notewriter')",
            &[&chart.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    let direct: chrono::DateTime<chrono::Utc> = w
        .query_one(
            "SELECT max(saved_at) FROM cairn_med_draft_lines($1::text::uuid, 'notewriter')",
            &[&chart.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(got[0].3, direct, "newest is the last save ({newest})");
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
        // A veto stands too, on a fresh line, until a provenance edit.
        let h = Uuid::now_v7();
        save_v(&w, h, chart, "a", false, true, base_x).await;
        assert!(row_of(&w, chart, h).await.1);
        // Now the edit arrives with the tick re-asserted by a stale client.
        save_v(&w, l, chart, "a", true, false, edit).await;
        save_v(&w, h, chart, "a", false, true, edit).await;
        let (ticked, ..) = row_of(&w, chart, l).await;
        assert!(!ticked, "{edit:?} must clear the tick in the door");
        assert!(
            !row_of(&w, chart, h).await.1,
            "{edit:?} must clear the veto"
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
        (Some("notewriter"), "patient", Some("  said so\n")),
        (Some("nurse-b"), "gp", None),
        (None, "other", Some("x")),
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
        "SELECT cairn_med_draft_from_save($1::text::uuid, ' Notewriter ', 'patient', 'again')",
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
        vec![(Some("patient".into()), Some("again".into()))]
    );
    assert_eq!(read(Some("nurse-b")).await, vec![(Some("gp".into()), None)]);
    assert_eq!(
        read(None).await,
        vec![(Some("other".into()), Some("x".into()))]
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
