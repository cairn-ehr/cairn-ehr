//! Batch charting slice 2b, Task 4 — the draft list's RESTORE line (db/078).
//!
//! A restore line is a list line pre-filled from a set-aside version; at Sign it will replace the
//! line that version lost to. Here only the storage door is tested: `cairn_med_draft_restore_add`,
//! what a plain save may never do to the `restores` column, and that a delete leaves no trace of
//! it. Same rules as the other list suites: the window's own login, DB-gated on `$CAIRN_TEST_PG`,
//! serialized via `db::test_serial_guard` taken BEFORE connecting.
mod common;

use cairn_node::db;
use common::{note_setup, owner_and_window};
use tokio_postgres::Client;
use uuid::Uuid;

/// (owner, window, chart) with a clean list and no set-aside versions.
async fn fixture(base: &str) -> (Client, Client, Uuid) {
    let (owner, window) = owner_and_window(base).await;
    let (_, _, _, _, chart) = note_setup(&owner).await;
    owner
        .batch_execute(
            "TRUNCATE medication_draft_line, medication_draft_from, medication_set_aside",
        )
        .await
        .unwrap();
    (owner, window, chart)
}

/// A projected set-aside version `version` of `alternative_to`, on `chart` (owner-only insert:
/// the real route is a signed event, which Task 5's tests drive).
async fn seed_version(owner: &Client, chart: Uuid, version: Uuid) {
    owner
        .execute(
            "INSERT INTO medication_set_aside (content_address, event_id, patient_id, version_id, \
                 alternative_to, term, info_source, t_recorded) \
             VALUES (decode(md5($1), 'hex'), $2::text::uuid, $3::text::uuid, $1::text::uuid, \
                 $4::text::uuid, 'oxycodone', 'patient-reported', now())",
            &[
                &version.to_string(),
                &Uuid::now_v7().to_string(),
                &chart.to_string(),
                &Uuid::now_v7().to_string(),
            ],
        )
        .await
        .unwrap();
}

/// Add a restore line as `notewriter` on `pc-1`; the door's one word.
async fn restore_add(c: &Client, line: Uuid, chart: Uuid, version: Uuid) -> String {
    c.query_one(
        "SELECT cairn_med_draft_restore_add($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', \
                $3::text::uuid, 'pre-existing', 'oxycodone 5 mg qid', \
                '[{\"term\":\"oxycodone\",\"coding\":null}]'::text::jsonb, 'oxycodone', '5 mg qid')",
        &[&line.to_string(), &chart.to_string(), &version.to_string()],
    )
    .await
    .expect("the restore door answers")
    .get(0)
}

/// Save over `line` (an existing row) with new words, optionally naming a set-aside mark.
async fn save_over(c: &Client, line: Uuid, chart: Uuid, text: &str, mark: Option<&str>) -> String {
    c.query_one(
        "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', \
                'pre-existing', $3, '[]'::text::jsonb, NULL, false, '[]'::text::jsonb, true, '', '', \
                false, NULL, NULL, NULL, $4)",
        &[&line.to_string(), &chart.to_string(), &text, &mark],
    )
    .await
    .expect("the save door answers")
    .get(0)
}

/// `(restores, set_aside_of, held, hand_ticked, finished, line_text)` of one list line, by the read door.
async fn stored(
    c: &Client,
    chart: Uuid,
    line: Uuid,
) -> Option<(Option<String>, Option<String>, bool, bool, bool, String)> {
    c.query_opt(
        "SELECT restores::text, set_aside_of, held, hand_ticked, finished, line_text \
           FROM cairn_med_draft_lines($2::text::uuid, 'notewriter') WHERE line_id = $1::text::uuid",
        &[&line.to_string(), &chart.to_string()],
    )
    .await
    .unwrap()
    .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5)))
}

#[tokio::test]
async fn a_restore_line_is_added_with_its_version() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, w, chart) = fixture(&base).await;
    let (version, line) = (Uuid::now_v7(), Uuid::now_v7());
    seed_version(&owner, chart, version).await;
    assert_eq!(restore_add(&w, line, chart, version).await, "saved");
    let (restores, mark, held, hand, finished, text) = stored(&w, chart, line).await.unwrap();
    assert_eq!(restores, Some(version.to_string()));
    assert_eq!(mark, None);
    assert_eq!((held, hand, finished), (false, false, true));
    assert_eq!(text, "oxycodone 5 mg qid");
    // Same answers as the save door's new-row branch: a second terminal, a deleted id, no chart.
    let other = Uuid::now_v7();
    assert_eq!(
        restore_add(&w, other, Uuid::now_v7(), version).await,
        "no-chart"
    );
    assert_eq!(
        restore_add(&w, line, chart, version).await,
        "saved",
        "a retry is idempotent"
    );
    assert_eq!(
        w.query_one(
            "SELECT count(*) FROM cairn_med_draft_lines($1::text::uuid, 'notewriter')",
            &[&chart.to_string()]
        )
        .await
        .unwrap()
        .get::<_, i64>(0),
        1
    );
}

#[tokio::test]
async fn restore_add_refuses_another_charts_version() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, w, chart) = fixture(&base).await;
    let (elsewhere, unknown) = (Uuid::now_v7(), Uuid::now_v7());
    seed_version(&owner, Uuid::now_v7(), elsewhere).await;
    for v in [elsewhere, unknown] {
        let line = Uuid::now_v7();
        assert_eq!(restore_add(&w, line, chart, v).await, "no-such-version");
        assert!(
            stored(&w, chart, line).await.is_none(),
            "nothing was written"
        );
    }
}

#[tokio::test]
async fn a_save_never_clears_or_sets_restores() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, w, chart) = fixture(&base).await;
    let (version, line, plain) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    seed_version(&owner, chart, version).await;
    restore_add(&w, line, chart, version).await;
    assert_eq!(
        save_over(&w, line, chart, "oxycodone 10 mg qid", None).await,
        "saved"
    );
    let (restores, _, _, _, _, text) = stored(&w, chart, line).await.unwrap();
    assert_eq!(
        restores,
        Some(version.to_string()),
        "an edit keeps the version it restores"
    );
    assert_eq!(text, "oxycodone 10 mg qid");
    // A save has no `restores` parameter, so a plain line can never become a restore line.
    save_over(&w, plain, chart, "paracetamol", None).await;
    assert_eq!(stored(&w, chart, plain).await.unwrap().0, None);
}

#[tokio::test]
async fn a_restore_line_cannot_be_set_aside() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, w, chart) = fixture(&base).await;
    let (version, line, other) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    seed_version(&owner, chart, version).await;
    restore_add(&w, line, chart, version).await;
    let mark = format!("line:{other}");
    assert_eq!(
        save_over(&w, line, chart, "oxycodone", Some(&mark)).await,
        "saved"
    );
    let (restores, set_aside, ..) = stored(&w, chart, line).await.unwrap();
    assert_eq!(restores, Some(version.to_string()));
    assert_eq!(
        set_aside, None,
        "a restore line is never a set-aside version"
    );
}

#[tokio::test]
async fn deleting_a_restore_line_leaves_no_record() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, w, chart) = fixture(&base).await;
    let (version, line) = (Uuid::now_v7(), Uuid::now_v7());
    seed_version(&owner, chart, version).await;
    restore_add(&w, line, chart, version).await;
    let answer: String = w
        .query_one(
            "SELECT cairn_med_draft_line_delete($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', \
                (SELECT s.saved_at FROM cairn_med_draft_lines($2::text::uuid, 'notewriter') s \
                  WHERE s.line_id = $1::text::uuid))",
            &[&line.to_string(), &chart.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(answer, "deleted");
    assert!(stored(&w, chart, line).await.is_none());
    // The tombstone is an id and a time (nothing of the version), and the version itself stands.
    let cols: i64 = owner
        .query_one(
            "SELECT count(*) FROM information_schema.columns \
              WHERE table_name = 'medication_draft_line_deleted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(cols, 2);
    let n: i64 = owner
        .query_one(
            "SELECT count(*) FROM medication_set_aside WHERE version_id = $1::text::uuid",
            &[&version.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1);
}
