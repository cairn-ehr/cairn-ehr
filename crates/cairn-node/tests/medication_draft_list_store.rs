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
    let against = json!([{"source": "allergy", "ref": "r1"}]).to_string();
    let (line, chart) = (line.to_string(), chart.to_string());
    c.query_one(
        "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, $3, $4, 'started-here', $5, $6::text::jsonb, NULL, $7, $8::text::jsonb, false)",
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
        .batch_execute("TRUNCATE medication_draft_line")
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
    assert_eq!(read().await.0, true);
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
            "SELECT cairn_med_draft_line_delete($1::text::uuid, $2::text::uuid, 'pc-2', 'notewriter')",
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
                "SELECT cairn_med_draft_line_delete($1::text::uuid, $2::text::uuid, $3, 'notewriter')",
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
        .query_one("SELECT cairn_med_draft_line_delete($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter')", &[&l, &c])
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
                "SELECT cairn_med_draft_line_delete($1::text::uuid, $2::text::uuid, 'pc-1', $3)",
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
    let against = json!([{"source": "allergy", "ref": "r1"}]).to_string();
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
            "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', $3, 'x', $4::text::jsonb, NULL, true, $5::text::jsonb, false)",
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
            "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', $3, 'x', $4::text::jsonb, $5, true, $6::text::jsonb, $7)",
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
            "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', 'started-here', 'x', '{}'::jsonb, NULL, false, '[]'::jsonb, false)",
            &[&Uuid::now_v7().to_string(), &c],
        )
        .await
        .unwrap_err();
    assert_eq!(
        e.code(),
        Some(&tokio_postgres::error::SqlState::CHECK_VIOLATION)
    );
}
