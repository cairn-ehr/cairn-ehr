//! Tests for `notes_commit.rs`: the due words (pure) and — DB-gated on `CAIRN_TEST_PG`, run locally
//! (#905: CI never runs these) — that a chart read says what the node's terminal service did: an
//! overdue draft waits for it, a draft it failed to commit says so, and a failed-set read that
//! cannot run never blocks the notes. The window itself commits nothing (#909).
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
        due_words(due, false, false, false, due, &bne(), today),
        "goes into the record unsigned at 06:40 if nobody types in it"
    );
    assert_eq!(
        due_words(due, true, false, false, due, &bne(), today),
        "overdue — not in the record yet; the node's terminal service puts it in unsigned at its next check"
    );
    assert_eq!(
        due_words(due, true, true, true, due, &bne(), today),
        "overdue, but cannot go into the record by itself — a writer of it is no longer \
         enrolled on this node; it goes in once they are enrolled again"
    );
    // PR #981 review F1: a draft whose last commit failed says so, and asks for a Sign — never
    // "tries again", which a failure that repeats every pass would make a broken promise.
    assert_eq!(
        due_words(due, true, false, true, due, &bne(), today),
        "overdue — could not go into the record by itself (the node's last try failed; the cause is \
         in the terminal service's log); sign it, or tell the site's operator"
    );
    // Final review I1: stuck wins even before it falls due — a stuck draft never goes in at
    // its due time, so it must never say it will.
    assert_eq!(
        due_words(due, false, true, true, due, &bne(), today),
        "cannot go into the record by itself — a writer of it is no longer enrolled on this \
         node; it can go in only once they are enrolled again"
    );
}

/// Final review F2 (ruling R16). An overdue draft still on the chart past three of the service's
/// commit ticks has not been put in by a service that would have run three times: it is not
/// running, or not reaching it. The window cannot see the service, so it stops promising "its
/// next check" and asks for a Sign instead. Exactly three ticks is still the ordinary wait.
#[test]
fn an_overdue_draft_past_three_ticks_is_not_put_in() {
    let today = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
    let due = utc("2026-11-02T20:40:00Z");
    let ticks = chrono::TimeDelta::from_std(COMMIT_TICK * 3).unwrap();
    let words = |now, stuck, failed| due_words(due, true, stuck, failed, now, &bne(), today);
    let at_three = due + ticks;
    let past_three = at_three + chrono::TimeDelta::seconds(1);
    assert_eq!(words(at_three, false, false), OVERDUE);
    assert_eq!(
        words(past_three, false, false),
        "overdue — the node's terminal service has not put it in the record; sign it, or tell \
         the site's operator"
    );
    // Precedence: stuck > failed > not put in > overdue.
    assert_eq!(words(past_three, false, true), FAILED);
    assert_eq!(words(past_three, true, true), STUCK);
    assert_eq!(words(past_three, true, false), STUCK);
    // Not overdue on the node's clock (the window's clock runs ahead): never "not put in".
    assert_eq!(
        due_words(due, false, false, false, past_three, &bne(), today),
        "goes into the record unsigned at 06:40 if nobody types in it"
    );
}

/// A chart with ONE overdue draft (quarantine 30 min, the draft typed 31 min ago) by the
/// enrolled writer `notewriter`, and the window's state over the window's own login, open on that
/// chart. The window holds no node key (#909): nothing here commits the draft.
/// Returns the serial guard (bind it, or another suite interleaves its TRUNCATE), the owner's
/// client, the state and the chart. `None` (self-skip) without `CAIRN_TEST_PG`.
async fn an_overdue_draft() -> Option<(impl Sized, tokio_postgres::Client, AppState, uuid::Uuid)> {
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
    state.node_kid = Some(node_kid);
    state.node_origin = ORIGIN.into();
    Some((guard, owner, state, chart))
}

/// An overdue draft the service has not reached reads as waiting for it, and is still listed.
#[tokio::test]
async fn an_overdue_draft_waits_for_the_service() {
    let Some((_guard, _owner, state, chart)) = an_overdue_draft().await else {
        return;
    };
    let pane = note_list_impl(&state, &chart.to_string())
        .await
        .expect("the chart reads");
    assert!(pane.notes.is_empty(), "the window committed nothing");
    assert_eq!(pane.drafts.len(), 1);
    assert!(
        pane.drafts[0]
            .words
            .ends_with(" · overdue — not in the record yet; the node's terminal service puts it in unsigned at its next check"),
        "{}",
        pane.drafts[0].words
    );
}

/// A draft the service's last try failed on (db/083's fact) says so, and asks for a Sign.
#[tokio::test]
async fn a_draft_the_service_failed_on_says_so() {
    let Some((_guard, owner, state, chart)) = an_overdue_draft().await else {
        return;
    };
    owner
        .execute(
            "SELECT cairn_note_draft_commit_failed(draft_id, true) FROM note_draft",
            &[],
        )
        .await
        .unwrap();
    let pane = note_list_impl(&state, &chart.to_string())
        .await
        .expect("the chart reads");
    assert_eq!(pane.drafts.len(), 1, "the draft is kept");
    assert!(
        pane.drafts[0].words.ends_with(" · overdue — could not go into the record by itself (the node's last try failed; the cause is in the terminal service's log); sign it, or tell the site's operator"),
        "{}",
        pane.drafts[0].words
    );
}

/// Stuck is computed from the writer, never from the failure fact: a stuck draft reads stuck
/// whether or not the service marked it.
#[tokio::test]
async fn a_stuck_draft_reads_stuck() {
    let Some((_guard, owner, state, chart)) = an_overdue_draft().await else {
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

/// A failed-set read that cannot run (the window lacks the door) never blocks the notes: the
/// draft is still listed, with no FAILED. The grant is restored for the next test.
#[tokio::test]
async fn a_failed_set_read_that_fails_still_reads_the_notes() {
    let Some((_guard, owner, state, chart)) = an_overdue_draft().await else {
        return;
    };
    owner
        .batch_execute(
            "REVOKE EXECUTE ON FUNCTION cairn_note_drafts_commit_failed(uuid) FROM cairn_window",
        )
        .await
        .unwrap();
    let pane = note_list_impl(&state, &chart.to_string()).await;
    owner
        .batch_execute(
            "GRANT EXECUTE ON FUNCTION cairn_note_drafts_commit_failed(uuid) TO cairn_window",
        )
        .await
        .unwrap();
    let pane = pane.expect("the chart still reads");
    assert_eq!(pane.drafts.len(), 1);
    assert!(!pane.drafts[0].words.contains("could not go into"));
}
