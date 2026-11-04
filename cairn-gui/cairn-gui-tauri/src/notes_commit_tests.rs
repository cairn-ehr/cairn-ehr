//! Tests for `notes_commit.rs`, the window as the quarantine commit's runner: the due words
//! (pure), fixture mode committing nothing, the tick spawned once (a source guard on `main.rs`),
//! and — DB-gated on `CAIRN_TEST_PG`, run locally (#905: CI never runs these) — a chart read that
//! commits its overdue draft first, and one that still reads when the commit cannot run or fails.
use super::*;
use crate::notes::note_list_impl;
use crate::notes::tests::{bne, utc};
use crate::state::AppState;
use cairn_event::SigningKey;

const ORIGIN: &str = "notes-commit-test-node";

/// Each sentence is true in every state it is shown in (review I1). The read runs AFTER the
/// pre-read commit, so a draft still listed overdue is stuck (it will never go in by itself), one
/// whose commit failed (said so, never "tries again"), or one a save or Sign held the lock of (it
/// is tried again).
#[test]
fn due_words_before_and_after() {
    let today = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
    // 20:40 UTC is 06:40 the next morning in Brisbane.
    let due = utc("2026-11-02T20:40:00Z");
    assert_eq!(
        due_words(due, false, false, false, &bne(), today),
        "goes into the record unsigned at 06:40 if nobody types in it"
    );
    assert_eq!(
        due_words(due, true, false, false, &bne(), today),
        "overdue — not in the record yet; the window tries again at every check to put it in unsigned"
    );
    assert_eq!(
        due_words(due, true, true, true, &bne(), today),
        "overdue, but cannot go into the record by itself — a writer of it is no longer \
         enrolled on this node; it goes in once they are enrolled again"
    );
    // PR #981 review F1: a draft whose last commit failed says so, and asks for a Sign — never
    // "tries again", which a failure that repeats every pass would make a broken promise.
    assert_eq!(
        due_words(due, true, false, true, &bne(), today),
        "overdue — could not go into the record by itself (the window's last try failed; the \
         cause is in its log); sign it, or tell the site's operator"
    );
    // Final review I1: stuck wins even before it falls due — a stuck draft never goes in at
    // its due time, so it must never say it will.
    assert_eq!(
        due_words(due, false, true, true, &bne(), today),
        "cannot go into the record by itself — a writer of it is no longer enrolled on this \
         node; it can go in only once they are enrolled again"
    );
}

/// Review M2: the tick says each stuck draft once per window run, not every minute.
#[test]
fn a_stuck_draft_is_said_once() {
    let (a, b) = (uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(2));
    let mut said = std::collections::HashSet::new();
    assert_eq!(newly_stuck(&mut said, vec![a]), vec![a]);
    assert_eq!(newly_stuck(&mut said, vec![a, b]), vec![b]);
    assert!(newly_stuck(&mut said, vec![a, b]).is_empty());
}

/// PR #981 review F2: a tick that stops running says so once, and once when it runs again.
#[test]
fn a_paused_tick_is_said_once_each_way() {
    let mut paused = false;
    assert_eq!(pause_change(&mut paused, true), None, "running stays quiet");
    assert!(pause_change(&mut paused, false).is_some_and(|l| l.contains("paused")));
    assert_eq!(
        pause_change(&mut paused, false),
        None,
        "said once, not a minute"
    );
    assert!(pause_change(&mut paused, true).is_some_and(|l| l.contains("running again")));
    assert_eq!(pause_change(&mut paused, true), None);
}

/// Review I2 and M1, pinned in the source (no behavioural test sees a hold's extent): the pass
/// lists under one short hold and takes ONE HOLD PER DRAFT, inside the loop — never one hold
/// for the whole pass, which would keep a clinician's press waiting behind every draft — and the
/// tick never reopens a lost connection (the next press does).
#[test]
fn each_draft_is_its_own_hold_and_the_tick_never_reopens() {
    let src = include_str!("notes_commit.rs");
    let pass = crate::source_guard::rust_fn(src, "commit_overdue_impl");
    assert!(!pass.contains("commit_overdue_drafts("), "{pass}");
    let each = crate::source_guard::position(pass, "for draft in drafts {");
    let held = &pass[each..];
    assert!(
        crate::source_guard::position(held, "hold(db, may_reopen)")
            < crate::source_guard::position(held, "commit_overdue(&mut"),
        "{pass}"
    );
    let hold = crate::source_guard::rust_fn(src, "hold");
    assert!(hold.contains("lock_if_open()"), "{hold}");
    let tick = crate::source_guard::rust_fn(src, "run_ticks");
    assert!(
        tick.contains("commit_overdue_impl(&app.state::<AppState>(), None, false)"),
        "{tick}"
    );
}

#[tokio::test]
async fn fixture_mode_commits_nothing() {
    let id = uuid::Uuid::now_v7();
    for may_reopen in [false, true] {
        let mock = AppState::mock(Some(id));
        assert!(commit_overdue_impl(&mock, None, may_reopen).await.is_none());
        assert!(commit_overdue_impl(&mock, Some(id), may_reopen)
            .await
            .is_none());
    }
}

/// The tick is a Rust task, spawned ONCE in setup: never a webview timer (WebKit throttles a
/// hidden window's timers), and never twice (two passes would only race each other's door).
#[test]
fn the_tick_is_spawned_once_in_setup() {
    let main = include_str!("main.rs");
    let spawn = "tauri::async_runtime::spawn(notes::commit::run_ticks(app.handle().clone()));";
    assert_eq!(main.matches("run_ticks(").count(), 1, "one spawn");
    let setup = &main[crate::source_guard::position(main, ".setup(|app| {")..];
    let end = crate::source_guard::position(setup, "Ok(())");
    assert!(setup[..end].contains(spawn), "spawned in setup: {setup}");
}

/// A chart with ONE overdue draft (quarantine 30 min, the draft typed 31 min ago) by the
/// enrolled writer `notewriter`, and the window's state over the window's own login, open on that
/// chart. `node_key`: whether the state holds the node key (without it the runner does nothing).
/// Returns the serial guard (bind it, or another suite interleaves its TRUNCATE), the owner's
/// client, the state and the chart. `None` (self-skip) without `CAIRN_TEST_PG`.
async fn an_overdue_draft(
    node_key: bool,
) -> Option<(impl Sized, tokio_postgres::Client, AppState, uuid::Uuid)> {
    let Ok(cs) = std::env::var("CAIRN_TEST_PG") else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    let mut owner = cairn_node::db::connect_and_load_schema(&cs).await.unwrap();
    owner
        .batch_execute(
            "TRUNCATE clinical_note, note_draft, note_draft_spent, note_policy, vault_attempt, \
             vault_member, patient_link, person_member, link_veto_flag, event_log, actor_event, \
             patient_chart, node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
        )
        .await
        .unwrap();
    // The node: a device actor whose key seals, with a fresh CSPRNG unwrap key registered (it
    // stands in for provisioning, which a DB-only test cannot run; never derived from the
    // signing seed, ADR-0066). Keys are minted at runtime (house rule 6).
    // Enrolled through SQL, as the other window fixtures do: the provisioning ceremony is the
    // CLI's alone (#654), so no window source may run it.
    let (node_sk, node_kid): (SigningKey, String) = cairn_event::generate_key().unwrap();
    owner
        .execute(
            "SELECT enroll_actor('device', \
             '{\"role\":\"registration-desk\",\"node_key\":\"notes-commit-test\"}', $1)",
            &[&node_kid],
        )
        .await
        .unwrap();
    let unwrap = cairn_event::seal::generate_unwrap_secret().unwrap();
    owner
        .execute(
            "SELECT cairn_register_unwrap_key($1)",
            &[&cairn_event::seal::unwrap_public(&unwrap)
                .as_bytes()
                .as_slice()],
        )
        .await
        .unwrap();
    // The writer: a human actor and a vault member holding its key, so the commit names them.
    let (_writer_sk, writer_kid) = cairn_event::generate_key().unwrap();
    owner
        .execute(
            "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
            &[&writer_kid],
        )
        .await
        .unwrap();
    owner
        .execute(
            "INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, \
                                       sealed, enrolled_by, contact_email) \
             SELECT 'notewriter', 'Nurse A', 'clinician', actor_id, $1, '\\x00'::bytea, \
                    'console', 'notes@example.org' \
               FROM actor_event WHERE signing_key_id = $1 AND op = 'enroll' LIMIT 1",
            &[&writer_kid],
        )
        .await
        .unwrap();
    let (chart, _, _) = cairn_node::john_doe::register_john_doe(
        &mut owner,
        &node_sk,
        &node_kid,
        ORIGIN,
        "ED",
        "test-site",
        "2026-11-04",
        "arrived unconscious",
    )
    .await
    .unwrap();
    let draft = uuid::Uuid::now_v7();
    let saved = cairn_node::note::draft::save_part(
        &owner,
        draft,
        chart,
        "fixture-terminal",
        Some("notewriter"),
        "obs stable overnight",
        None,
    )
    .await
    .unwrap();
    assert_eq!(saved, cairn_node::note::draft::SaveOutcome::Saved);
    owner
        .execute("SELECT cairn_note_set_quarantine(30)", &[])
        .await
        .unwrap();
    // Typed 31 minutes ago: the header and its part move back together, as note_commit.rs ages.
    owner
        .execute(
            "WITH h AS (UPDATE note_draft SET last_keystroke = last_keystroke - interval '31 min', \
                               recording_started = recording_started - interval '31 min' \
                         WHERE draft_id = $1::text::uuid) \
             UPDATE note_draft_part SET last_keystroke = last_keystroke - interval '31 min', \
                    recording_started = recording_started - interval '31 min' \
              WHERE draft_id = $1::text::uuid",
            &[&draft.to_string()],
        )
        .await
        .unwrap();
    let window = crate::enrol::tests::window_login(&owner, &cs).await;
    let mut state = AppState::mock(Some(chart));
    state.db = Some(std::sync::Arc::new(cairn_gui_live::NodeConnection::fixed(
        cairn_node::db::connect(&window).await.unwrap(),
    )));
    state.node_sk = node_key.then_some(node_sk);
    state.node_origin = ORIGIN.into();
    Some((guard, owner, state, chart))
}

/// Review Focus 5: a window that cannot commit (here: no node key) never blocks the read. The
/// draft is still listed, and says it is overdue.
#[tokio::test]
async fn a_window_without_the_node_key_still_reads() {
    let Some((_guard, _owner, state, chart)) = an_overdue_draft(false).await else {
        return;
    };
    let pane = note_list_impl(&state, &chart.to_string())
        .await
        .expect("the chart reads");
    assert!(pane.notes.is_empty(), "nothing went in");
    assert_eq!(pane.drafts.len(), 1);
    assert!(
        pane.drafts[0]
            .words
            .ends_with(" · overdue — not in the record yet; the window tries again at every check to put it in unsigned"),
        "{}",
        pane.drafts[0].words
    );
}

/// Review Focus 5, with a commit that really FAILS (PR #981 review): the node key was never
/// enrolled, so the floor refuses the note mid-transaction. The read on the SAME connection still
/// works, the draft is kept, and it says its commit failed — not "tries again" (review F1).
#[tokio::test]
async fn a_failed_commit_never_blocks_the_read() {
    let Some((_guard, _owner, mut state, chart)) = an_overdue_draft(true).await else {
        return;
    };
    state.node_sk = Some(cairn_event::generate_key().unwrap().0);
    let report = commit_overdue_impl(&state, Some(chart), true)
        .await
        .expect("the commit was asked");
    assert_eq!(report.failed.len(), 1, "{report:?}");
    let pane = note_list_impl(&state, &chart.to_string())
        .await
        .expect("the chart reads");
    assert!(pane.notes.is_empty(), "nothing went in");
    assert_eq!(pane.drafts.len(), 1, "the draft is kept");
    assert!(
        pane.drafts[0].words.ends_with(" · overdue — could not go into the record by itself (the window's last try failed; the cause is in its log); sign it, or tell the site's operator"),
        "{}",
        pane.drafts[0].words
    );
}

/// Opening a chart commits its overdue drafts BEFORE the read: the pane shows the note that went
/// in unsigned, and no draft — never a draft the next tick is about to take away.
#[tokio::test]
async fn opening_a_chart_commits_its_overdue_draft_first() {
    let Some((_guard, owner, state, chart)) = an_overdue_draft(true).await else {
        return;
    };
    let pane = note_list_impl(&state, &chart.to_string())
        .await
        .expect("the chart reads");
    assert!(pane.drafts.is_empty(), "the draft went in");
    assert_eq!(pane.notes.len(), 1);
    assert_eq!(pane.notes[0].text, "obs stable overnight");
    let list = cairn_node::note::read::list_patient_notes(&owner, chart)
        .await
        .unwrap();
    assert_eq!(
        list.notes[0].grade,
        cairn_node::note::view::NoteGrade::WentInUnsigned
    );
    assert!(
        pane.notes[0].byline.contains("went in unsigned"),
        "{}",
        pane.notes[0].byline
    );
}

/// Review I1: a draft whose named writer has left the vault is STUCK — the pre-read commit rolls
/// it back every time — and the pane says so, with what makes it go in, never "it goes in".
#[tokio::test]
async fn a_stuck_draft_reads_stuck() {
    let Some((_guard, owner, state, chart)) = an_overdue_draft(true).await else {
        return;
    };
    owner
        .batch_execute(
            "DELETE FROM vault_attempt WHERE staff_id = 'notewriter'; \
             DELETE FROM vault_member WHERE staff_id = 'notewriter';",
        )
        .await
        .unwrap();
    let pane = note_list_impl(&state, &chart.to_string())
        .await
        .expect("the chart reads");
    assert!(pane.notes.is_empty(), "nothing went in");
    assert_eq!(pane.drafts.len(), 1, "the draft is kept");
    assert!(
        pane.drafts[0].words.ends_with(" · overdue, but cannot go into the record by itself — a writer of it is no longer enrolled on this node; it goes in once they are enrolled again"),
        "{}",
        pane.drafts[0].words
    );
}
