//! #909 slice A, Task 6 — the terminal service runs the quarantine commit
//! (`terminal_service::commit::commit_tick`) and records the drafts that would not go in.
//!
//! WHY THESE TESTS EXIST. The window no longer holds the node key, so the 60 s tick that puts an
//! abandoned note draft into the record (node-signed, graded `Asserted`) moves to the service.
//! The tick must (1) commit what is overdue, signed by the NODE, (2) leave alone what is not due,
//! and (3) leave the clinician a trace when a draft would not go in, because the window can no
//! longer know: the service writes db/083's failure fact (design D7), and clears it on a later
//! success. Each test runs the tick as a login granted `cairn_terminal_service`, the role the
//! real process holds. DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
mod common;

use cairn_event::generate_key;
use cairn_node::db;
use cairn_node::note::draft::save_part;
use cairn_node::note::read::list_patient_notes;
use cairn_node::note::view::NoteGrade;
use cairn_node::terminal_service::commit::{commit_tick, failure_line};
use common::{cs, owner_and_window, service_login};
use std::collections::HashSet;
use tokio_postgres::Client;
use uuid::Uuid;

const ORIGIN: &str = "terminal-service-commit-test";

/// Save one part of `draft` on `chart` as `writer`, typed `minutes` ago.
async fn save_aged(c: &Client, draft: Uuid, chart: Uuid, writer: Option<&str>, minutes: i32) {
    save_part(
        c,
        draft,
        chart,
        "ward-3-pc",
        writer,
        "obs stable",
        None,
        None,
    )
    .await
    .unwrap();
    c.execute(
        "WITH h AS (UPDATE note_draft SET last_keystroke = last_keystroke - make_interval(mins => $2), \
                           recording_started = recording_started - make_interval(mins => $2) \
                     WHERE draft_id = $1::text::uuid) \
         UPDATE note_draft_part SET last_keystroke = last_keystroke - make_interval(mins => $2), \
                recording_started = recording_started - make_interval(mins => $2) \
          WHERE draft_id = $1::text::uuid",
        &[&draft.to_string(), &minutes],
    )
    .await
    .unwrap();
}

/// What the window's notes read lists as failed on `chart`, as the window.
async fn failed_on(window: &Client, chart: Uuid) -> Vec<Uuid> {
    window
        .query(
            "SELECT d::text FROM cairn_note_drafts_commit_failed($1::text::uuid) d",
            &[&chart.to_string()],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get::<_, String>(0).parse().unwrap())
        .collect()
}

async fn notes_on(c: &Client, chart: Uuid) -> Vec<(String, String)> {
    c.query(
        "SELECT e.signer_key_id, n.effective_basis FROM clinical_note n \
           JOIN event_log e ON e.event_id = n.event_id WHERE n.patient_id = $1::text::uuid",
        &[&chart.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1)))
    .collect()
}

struct Rig {
    _guard: Client,
    owner: Client,
    window: Client,
    service: Client,
    sk: cairn_event::SigningKey,
    kid: String,
    chart: Uuid,
    base: String,
}

async fn rig() -> Option<Rig> {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = db::test_serial_guard(&base).await.unwrap();
    let (owner, window) = owner_and_window(&base).await;
    let (sk, kid, _, _, chart) = common::note_setup(&owner).await;
    owner
        .batch_execute("TRUNCATE note_draft_commit_failure")
        .await
        .unwrap();
    let service = db::connect(&service_login(&owner, &base).await)
        .await
        .expect("the service's login connects");
    Some(Rig {
        _guard: guard,
        owner,
        window,
        service,
        sk,
        kid,
        chart,
        base,
    })
}

#[tokio::test]
async fn an_overdue_draft_goes_in_node_signed() {
    let Some(mut r) = rig().await else { return };
    let draft = Uuid::now_v7();
    save_aged(&r.owner, draft, r.chart, Some("notewriter"), 400).await;

    let report = commit_tick(&mut r.service, &r.sk, ORIGIN, &mut HashSet::new())
        .await
        .unwrap();

    assert_eq!(report.committed, 1);
    let notes = notes_on(&r.owner, r.chart).await;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].0, r.kid, "signed by the node's key");
    let list = list_patient_notes(&r.owner, r.chart).await.unwrap();
    assert_eq!(list.notes[0].grade, NoteGrade::WentInUnsigned);
    assert!(failed_on(&r.window, r.chart).await.is_empty());
}

#[tokio::test]
async fn a_not_yet_due_draft_is_untouched() {
    let Some(mut r) = rig().await else { return };
    let draft = Uuid::now_v7();
    save_aged(&r.owner, draft, r.chart, Some("notewriter"), 0).await;

    let report = commit_tick(&mut r.service, &r.sk, ORIGIN, &mut HashSet::new())
        .await
        .unwrap();

    assert_eq!(report.committed, 0);
    assert!(notes_on(&r.owner, r.chart).await.is_empty());
    assert!(failed_on(&r.window, r.chart).await.is_empty());
    let open: i64 = r
        .owner
        .query_one("SELECT count(*) FROM note_draft", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(open, 1, "the draft is still open");
}

#[tokio::test]
async fn a_failing_commit_is_recorded_and_a_later_success_clears_it() {
    let Some(mut r) = rig().await else { return };
    // A named writer whose vault key is gone: the draft is stuck, it will not go in.
    let (_sk, a_kid) = generate_key().unwrap();
    r.owner
        .execute(
            "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"handle\":\"nurse-a\"}', $1)",
            &[&a_kid],
        )
        .await
        .unwrap();
    let enrol =
        "INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, \
                                   sealed, enrolled_by, contact_email) \
         SELECT 'nurse-a', 'Nurse A', 'nurse', actor_id, $1, '\\x00'::bytea, 'console', \
                'a@example.org' \
           FROM actor_event WHERE signing_key_id = $1 AND op = 'enroll' LIMIT 1";
    r.owner.execute(enrol, &[&a_kid]).await.unwrap();
    let draft = Uuid::now_v7();
    save_aged(&r.owner, draft, r.chart, Some("nurse-a"), 400).await;
    r.owner
        .batch_execute(
            "DELETE FROM vault_attempt WHERE staff_id = 'nurse-a'; \
             DELETE FROM vault_member WHERE staff_id = 'nurse-a';",
        )
        .await
        .unwrap();

    let report = commit_tick(&mut r.service, &r.sk, ORIGIN, &mut HashSet::new())
        .await
        .unwrap();
    assert_eq!(report.stuck, vec![draft]);
    assert_eq!(failed_on(&r.window, r.chart).await, vec![draft]);
    assert!(notes_on(&r.owner, r.chart).await.is_empty());

    // She is enrolled again: the next tick puts it in and the fact goes.
    r.owner.execute(enrol, &[&a_kid]).await.unwrap();
    let report = commit_tick(&mut r.service, &r.sk, ORIGIN, &mut HashSet::new())
        .await
        .unwrap();
    assert_eq!(report.committed, 1);
    assert!(failed_on(&r.window, r.chart).await.is_empty());
    assert_eq!(notes_on(&r.owner, r.chart).await.len(), 1);
}

/// The log line names the draft, the SQLSTATE and the PRIMARY message only; DETAIL (which can
/// carry row values) and HINT never reach stderr.
#[tokio::test]
async fn the_failure_line_drops_detail_and_hint() {
    let Some(r) = rig().await else { return };
    let e = r
        .owner
        .batch_execute(
            "DO $$ BEGIN RAISE EXCEPTION 'floor says no' USING ERRCODE = 'P0001', \
             DETAIL = 'row had Jane Citizen', HINT = 'try Jane'; END $$",
        )
        .await
        .unwrap_err();
    let draft = Uuid::from_u128(7);
    let line = failure_line(draft, &anyhow::Error::new(e).context("committing"));
    assert!(line.contains(&draft.to_string()), "{line}");
    assert!(line.contains("P0001"), "{line}");
    assert!(line.contains("floor says no"), "{line}");
    assert!(!line.contains("Jane"), "{line}");
}

/// An error that is not a database error says only its outermost sentence.
#[test]
fn a_non_database_failure_line_is_the_outer_sentence() {
    let e = anyhow::anyhow!("inner detail").context("the commit failed");
    let line = failure_line(Uuid::from_u128(1), &e);
    assert!(line.contains("the commit failed"), "{line}");
    assert!(!line.contains("inner detail"), "{line}");
}

/// A draft whose commit is skipped (its lock is held) is not failing: an old failed fact clears.
#[tokio::test]
async fn a_skipped_draft_clears_its_old_failed_fact() {
    let Some(mut r) = rig().await else { return };
    let draft = Uuid::now_v7();
    save_aged(&r.owner, draft, r.chart, Some("notewriter"), 400).await;
    r.service
        .execute(
            "SELECT cairn_note_draft_commit_failed($1::text::uuid, true)",
            &[&draft.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(failed_on(&r.window, r.chart).await, vec![draft]);
    let mut other = db::connect(&r.base).await.unwrap();
    let holder = other.transaction().await.unwrap();
    holder
        .execute(
            "SELECT pg_advisory_xact_lock(hashtextextended('note_draft:' || $1::text::uuid::text, 0))",
            &[&draft.to_string()],
        )
        .await
        .unwrap();

    let report = commit_tick(&mut r.service, &r.sk, ORIGIN, &mut HashSet::new())
        .await
        .unwrap();

    assert_eq!(report.committed, 0, "the held draft was skipped");
    assert!(failed_on(&r.window, r.chart).await.is_empty());
    holder.rollback().await.unwrap();
}
