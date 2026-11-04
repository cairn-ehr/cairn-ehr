//! Tests for `notes_discard.rs`, the window's discard command: the wrong-chart and fixture-mode
//! refusals (against `AppState::mock`, no Tauri runtime), the pure words and the time check, and
//! — DB-gated on `CAIRN_TEST_PG`, run locally (#905: CI never runs these) — one discard under the
//! window's own login.
use super::*;
use crate::funnel::view::Retry;
use cairn_event::SigningKey;
use cairn_node::note::draft::{SaveOutcome, TakeOutcome};

fn on_screen() -> (AppState, String) {
    let chart = Uuid::from_u128(7);
    (AppState::mock(Some(chart)), chart.to_string())
}

fn a_draft() -> String {
    Uuid::from_u128(99).to_string()
}

/// Checked FIRST, as a sign's is: a discard naming a chart that is not on screen is refused for
/// THAT reason, whatever else is wrong.
#[tokio::test]
async fn discard_refuses_a_chart_not_on_screen() {
    let (state, _) = on_screen();
    let other = Uuid::from_u128(424_242).to_string();
    let err = note_discard_impl(&state, &other, &a_draft(), "seen", None, None, None)
        .await
        .unwrap_err();
    assert!(err.text.contains("not the chart"), "{}", err.text);
}

#[tokio::test]
async fn discard_in_fixture_mode_refuses() {
    let (state, chart) = on_screen();
    let err = note_discard_impl(&state, &chart, &a_draft(), "seen", None, None, None)
        .await
        .unwrap_err();
    assert_eq!(err.retry, Retry::Never);
    assert!(err.text.contains("fixture mode"), "{}", err.text);
}

/// A time that cannot be read is a verdict on the gesture, said before the key is asked for.
/// An empty editor is NOT refused: discarding an empty draft is a real, quiet gesture.
#[test]
fn a_bad_time_is_refused_before_the_key() {
    assert!(discard_checks_before_the_key(&a_draft(), Some("not a time")).is_err());
    assert!(discard_checks_before_the_key(&a_draft(), Some("0000-06-01T12:00")).is_err());
    assert!(discard_checks_before_the_key("not a draft id", None).is_err());
    assert!(discard_checks_before_the_key(&a_draft(), None).is_ok());
    assert!(discard_checks_before_the_key(&a_draft(), Some("9999-12-31T09:00")).is_ok());
}

/// Every refusal, its exact sentence, and whether the pane is behind the record (stale) or the
/// verdict stands (refused).
#[test]
fn discard_words_each_refusal() {
    use DiscardRefusal::*;
    let cases = [
        (Spent, true, "Not discarded: this note has already been signed"),
        (
            Committed,
            true,
            "Not discarded: this note had already gone into the record unsigned — nothing was signed",
        ),
        (Discarded, true, "This draft was discarded; it is no longer open."),
        (
            Elsewhere,
            false,
            "Not discarded: this draft belongs to another chart — nothing was signed; your text is still here",
        ),
        (
            Moved,
            false,
            "Not discarded: This note is now being continued at another terminal — nothing was \
             signed; your text is still here.",
        ),
        (NoChart, false, "Not discarded: this patient has no chart on this node"),
        (
            NoSuchWriter,
            false,
            "Not discarded: a writer of this note is not enrolled on this node — nothing was \
             signed; your text is still here",
        ),
        (
            NotYours,
            false,
            "Not discarded: someone else wrote part of this draft, and only its writer may throw \
             their part away. The draft's own line says when it goes into the record by itself.",
        ),
        (
            NoSuchSigner,
            false,
            "Not discarded: this staff ID may not discard a clinical note.",
        ),
        (Gone, false, "Not discarded: this draft is no longer open"),
    ];
    for (refusal, stale, sentence) in cases {
        let view = discard_refusal_view(refusal);
        assert_eq!(view.text, sentence, "{refusal:?}");
        assert_eq!(
            matches!(view.retry, Retry::AfterReread(_)),
            stale,
            "{refusal:?}"
        );
    }
}

#[test]
fn discard_words_each_outcome() {
    assert_eq!(
        RECORDED,
        "Discarded. The draft is gone from the chart; the record keeps what it said."
    );
    assert_eq!(
        EMPTY_DISCARDED,
        "Discarded. The draft was empty, so nothing was kept."
    );
}

/// A discarded save is not worded as signed (ruling 1), and a take and a sign say it is closed.
#[test]
fn a_discarded_save_reads_like_spent_and_says_discarded() {
    let v = crate::notes::save_view(SaveOutcome::Discarded);
    assert!(v.discarded && !v.saved && !v.spent);
    assert_eq!(
        v.sentence.as_deref(),
        Some("This draft was discarded; it is no longer open.")
    );
}

#[test]
fn take_words_discarded() {
    let v = crate::notes::take_view(TakeOutcome::Discarded);
    assert!(!v.taken);
    assert_eq!(
        v.sentence.as_deref(),
        Some("This draft was discarded; it is no longer open.")
    );
    let e = crate::notes::sign_refusal_view(cairn_node::note::sign::SignRefusal::Discarded);
    assert_eq!(e.text, "This draft was discarded; it is no longer open.");
}

/// The window's own login discards a draft: the report says so, the chart lists a marker, and no
/// note was made. Self-skips without `CAIRN_TEST_PG` (then the harness prints `skipped:`).
#[tokio::test]
async fn note_discard_impl_discards_under_the_window_login() {
    let Ok(cs) = std::env::var("CAIRN_TEST_PG") else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    let mut owner = cairn_node::db::connect_and_load_schema(&cs).await.unwrap();
    owner
        .batch_execute(
            "TRUNCATE clinical_note, note_draft, note_draft_spent, note_policy, vault_attempt, \
             vault_member, patient_link, person_member, link_veto_flag, event_log, actor_event, \
             patient_chart, node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
        )
        .await
        .unwrap();
    // The node's device key and unwrap key, as `notes_commit_tests` mints them (runtime, rule 6).
    let (node_sk, node_kid): (SigningKey, String) = cairn_event::generate_key().unwrap();
    owner
        .execute(
            "SELECT enroll_actor('device', \
             '{\"role\":\"registration-desk\",\"node_key\":\"notes-discard-test\"}', $1)",
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
    // The discarder: a clinician whose key the window holds.
    let (writer_sk, writer_kid) = cairn_event::generate_key().unwrap();
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
        "notes-discard-test-node",
        "ED",
        "test-site",
        "2026-11-04",
        "arrived unconscious",
    )
    .await
    .unwrap();
    let draft = Uuid::now_v7();
    let saved = cairn_node::note::draft::save_part(
        &owner,
        draft,
        chart,
        "fixture-terminal",
        Some("notewriter"),
        "obs stable",
        None,
    )
    .await
    .unwrap();
    assert_eq!(saved, SaveOutcome::Saved);

    let window = crate::enrol::tests::window_login(&owner, &cs).await;
    let mut state = AppState::mock(Some(chart));
    state.db = Some(std::sync::Arc::new(cairn_gui_live::NodeConnection::fixed(
        cairn_node::db::connect(&window).await.unwrap(),
    )));
    state.node_sk = Some(node_sk);
    state.node_origin = "notes-discard-test-node".into();
    state.key_hold = std::time::Duration::from_secs(600);
    state.session.lock().await.key = Some(crate::state::SessionKey::new(
        writer_sk,
        "notewriter".into(),
        Now::read(),
    ));

    let report = note_discard_impl(
        &state,
        &chart.to_string(),
        &draft.to_string(),
        "obs stable",
        None,
        Some("notewriter"),
        None,
    )
    .await
    .expect("the window's login may discard");
    assert_eq!(report.text, RECORDED);
    assert!(report.behind_the_record.is_none());

    let list = cairn_node::note::read::list_patient_notes(&owner, chart)
        .await
        .unwrap();
    assert!(list.notes.is_empty(), "a discard is never a note");
    assert!(list.drafts.is_empty());
    assert_eq!(list.discards.len(), 1);
    assert_eq!(list.discards[0].parts[0].text, "obs stable");
}
