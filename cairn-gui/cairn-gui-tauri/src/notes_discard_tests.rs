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
    // Not a worded refusal: it carries the lead (Task 6 review, item 3).
    assert!(
        err.text.starts_with("Not discarded: fixture mode"),
        "{}",
        err.text
    );
}

/// Task 6 review, item 3: a failure that is not one of the node's worded refusals (an outage, a
/// node that cannot write, a check before the key) reaches the clinician led by what it means
/// for the discard — chosen by its retry class — and a text already led is never led twice.
#[test]
fn a_failure_is_led_by_what_it_means_for_the_discard() {
    let view = |text: &str, retry| ErrorView {
        text: text.into(),
        retry,
    };
    let led = |v: ErrorView| (led_failure(v.clone()).text, led_failure(v).retry);
    assert_eq!(
        led(view(
            "connection reset",
            Retry::AfterReread(Recorded::Unknown)
        )),
        (
            "Whether this draft was discarded is NOT known: connection reset".to_string(),
            Retry::AfterReread(Recorded::Unknown)
        )
    );
    for retry in [
        Retry::Now,
        Retry::AfterOperator,
        Retry::Never,
        Retry::AfterReread(Recorded::No),
    ] {
        assert_eq!(
            led(view("no database connection", retry)),
            ("Not discarded: no database connection".to_string(), retry)
        );
    }
    for already in [
        "Not discarded: this draft belongs to another chart",
        "Whether this draft was discarded is NOT known: connection reset",
    ] {
        assert_eq!(led(view(already, Retry::Now)).0, already, "never doubled");
        assert_eq!(
            led(view(already, Retry::AfterReread(Recorded::Unknown))).0,
            already,
            "never doubled"
        );
    }
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
            "Not discarded: this note had already gone into the record unsigned.",
        ),
        (Discarded, true, "This draft was discarded; it is no longer open."),
        (
            Elsewhere,
            false,
            "Not discarded: this draft belongs to another chart — nothing was thrown away; your text is still here.",
        ),
        (
            Moved,
            false,
            "Not discarded: this draft is now being continued at another terminal — nothing was \
             thrown away; your text is still here.",
        ),
        (NoChart, false, "Not discarded: this patient has no chart on this node"),
        (
            NoSuchWriter,
            false,
            "Not discarded: the name at this terminal is not an enrolled member — say who's \
             here again; your text is still here.",
        ),
        (
            NotYours,
            false,
            "Not discarded: someone else wrote part of this draft, and only its writer may throw \
             their part away — nothing was thrown away; your text is still here.",
        ),
        (
            NoSuchSigner,
            false,
            "Not discarded: this staff ID may not discard a clinical draft.",
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

/// PR #984 review: a key refusal under Discard is never worded as Sign's ("…to sign", "Nothing was
/// signed"): a clinician throwing a draft away must not read that the press will sign it. Every
/// variant, so a new one must be worded for a discard too; the retry is the key's own.
#[test]
fn a_key_refusal_under_discard_never_says_sign() {
    use crate::signer::KeyRefusal;
    use std::time::Duration;
    let every = [
        KeyRefusal::NeedCredential,
        KeyRefusal::Wrong { retry_after: None },
        KeyRefusal::Wrong {
            retry_after: Some(Duration::from_secs(2)),
        },
        KeyRefusal::Replaced {
            at: "3 Nov 03:40".into(),
            approver: "Nurse A".into(),
            retry_after: None,
        },
        KeyRefusal::Waiting {
            retry_after: Duration::from_secs(12),
        },
        KeyRefusal::Unknown,
        KeyRefusal::NotForClinicalActs,
        KeyRefusal::NotCurrent,
        KeyRefusal::Expired,
        KeyRefusal::OverPowered,
        KeyRefusal::Fault,
    ];
    for refusal in &every {
        let view = led_failure(discard_key_refused(refusal));
        assert!(
            view.text.starts_with("Not discarded: "),
            "{refusal:?}: {}",
            view.text
        );
        assert!(
            !view.text.to_lowercase().contains("sign"),
            "{refusal:?}: {}",
            view.text
        );
        assert_eq!(view.retry, key_locked(refusal).retry, "{refusal:?}");
    }
    assert_eq!(
        led_failure(discard_key_refused(&KeyRefusal::NeedCredential)).text,
        "Not discarded: Enter your staff ID and passphrase to discard this draft."
    );
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
