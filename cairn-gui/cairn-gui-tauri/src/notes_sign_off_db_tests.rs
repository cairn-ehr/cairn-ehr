//! DB-gated tests for `notes_sign_off.rs` (on `CAIRN_TEST_PG`, run locally — #905: CI never runs
//! these): a sign-off under the window's own login, the pane's "also covers" line read through
//! `cairn_note_staff_actor` under `cairn_window`.
use super::*;
use crate::notes::note_list_impl;
use crate::whos_here::Claim;

/// The window's own login signs a note off: the pane offers it and names the claim's unsigned
/// part (`cairn_note_staff_actor` under `cairn_window`), a malformed id asks for no key, a stale
/// list is refused `Changed`, the sign-off lands, and a second press has nothing to sign.
/// Self-skips without `CAIRN_TEST_PG` (then the harness prints `skipped:`).
#[tokio::test]
async fn note_sign_off_impl_signs_off_under_the_window_login() {
    let Ok(cs) = std::env::var("CAIRN_TEST_PG") else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    let mut owner = cairn_node::db::connect_and_load_schema(&cs).await.unwrap();
    owner
        .batch_execute(
            "TRUNCATE clinical_note, clinical_note_attestation, note_draft, note_draft_spent, \
             note_policy, vault_attempt, vault_member, patient_link, person_member, \
             link_veto_flag, event_log, actor_event, patient_chart, node_unwrap_key, event_dek, \
             event_clear, erasure_shred_log CASCADE",
        )
        .await
        .unwrap();
    const ORIGIN: &str = "notes-sign-off-test-node";
    let (node_sk, node_kid) = cairn_event::generate_key().unwrap();
    owner
        .execute(
            "SELECT enroll_actor('device', \
             '{\"role\":\"registration-desk\",\"node_key\":\"notes-sign-off-test\"}', $1)",
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
        ORIGIN,
        "ED",
        "test-site",
        "2026-11-04",
        "arrived unconscious",
    )
    .await
    .unwrap();
    // Nurse A's part, never signed: the quarantine timer put it in (backdated, never slept).
    let draft = Uuid::now_v7();
    cairn_node::note::draft::save_part(
        &owner,
        draft,
        chart,
        "fixture-terminal",
        Some("notewriter"),
        "obs stable",
        None,
        None,
    )
    .await
    .unwrap();
    owner
        .execute(
            "WITH h AS (UPDATE note_draft SET last_keystroke = last_keystroke - interval '181 min', \
                               recording_started = recording_started - interval '181 min' \
                         WHERE draft_id = $1::text::uuid) \
             UPDATE note_draft_part SET last_keystroke = last_keystroke - interval '181 min', \
                    recording_started = recording_started - interval '181 min' \
              WHERE draft_id = $1::text::uuid",
            &[&draft.to_string()],
        )
        .await
        .unwrap();
    let committed = cairn_node::note::commit::commit_overdue(&mut owner, &node_sk, draft, ORIGIN)
        .await
        .unwrap();
    let cairn_node::note::commit::CommitOutcome::Committed(done) = committed else {
        panic!("the timer commits the part: {committed:?}");
    };
    let p = done.event_id;

    let window = crate::enrol::tests::window_login(&owner, &cs).await;
    let mut state = AppState::mock(Some(chart));
    state.db = Some(std::sync::Arc::new(cairn_gui_live::NodeConnection::fixed(
        cairn_node::db::connect(&window).await.unwrap(),
    )));
    state.node_sk = Some(node_sk);
    state.node_origin = ORIGIN.into();
    state.key_hold = std::time::Duration::from_secs(600);
    state.whos_here.lock().await.pick(Claim::Named {
        staff_id: "notewriter".into(),
        display_name: "Nurse A".into(),
    });
    let patient = chart.to_string();
    let root = p.to_string();

    let pane = note_list_impl(&state, &patient).await.unwrap();
    let note = &pane.notes[0];
    assert!(note.can_sign_off);
    assert_eq!(note.shown_parts, std::slice::from_ref(&root));
    assert_eq!(note.covers, std::slice::from_ref(&root));
    let line = note.covers_line.as_deref().unwrap();
    assert!(
        line.starts_with("Your signature also covers your earlier unsigned part of this note: "),
        "{line}"
    );

    // No key is held: a malformed id is refused for what it is, never "enter your passphrase".
    let err = note_sign_off_impl(&state, &patient, &root, &["junk".into()], None)
        .await
        .unwrap_err();
    assert_eq!(err.text, "not a note id");
    state.session.lock().await.key = Some(crate::state::SessionKey::new(
        writer_sk,
        "notewriter".into(),
        Now::read(),
    ));
    // A part the window did not show: refused, nothing written.
    let stale_list = [root.clone(), Uuid::now_v7().to_string()];
    let err = note_sign_off_impl(&state, &patient, &root, &stale_list, None)
        .await
        .unwrap_err();
    assert!(err.text.starts_with("This note changed"), "{}", err.text);

    let report = note_sign_off_impl(&state, &patient, &root, std::slice::from_ref(&root), None)
        .await
        .expect("the window's login may sign a note off");
    assert_eq!(report.text, SIGNED_OFF);
    assert!(report.behind_the_record.is_none());

    let again = note_sign_off_impl(&state, &patient, &root, std::slice::from_ref(&root), None)
        .await
        .unwrap_err();
    assert!(
        again.text.starts_with("You have already signed"),
        "{}",
        again.text
    );
    let pane = note_list_impl(&state, &patient).await.unwrap();
    let note = &pane.notes[0];
    assert!(!note.can_sign_off, "Nurse A signed every part");
    assert_eq!(note.covers_line, None, "a vouched part is signed");
    assert!(
        note.byline.contains("signed off by Nurse A"),
        "{}",
        note.byline
    );
}
