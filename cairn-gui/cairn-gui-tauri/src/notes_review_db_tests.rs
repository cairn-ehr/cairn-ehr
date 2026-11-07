//! DB-gated tests for `notes_review.rs` (on `CAIRN_TEST_PG`, run locally — #905: CI never runs
//! these): "Co-sign as reviewed" under the window's own login, with real vault members whose
//! keys the strip releases by staff ID and passphrase.
//!
//! The fixture: Dr Sue supervises Dr Bea (the owner's console door), Dr Zed supervises no one,
//! and Bea has signed a one-part note that owes no co-sign. Who's here names Sue, so the pane
//! OFFERS the review; what it SIGNS is decided by the key the strip releases (ADR-0092 decision
//! 16, Review Focus 5).
use super::*;
use crate::notes::note_list_impl;
use crate::whos_here::Claim;
use cairn_node::vault::{bootstrap_superuser, enrol_member, release_for_act, NewMember, Purpose};
use zeroize::Zeroizing;

const ORIGIN: &str = "notes-review-test-node";

/// A test passphrase built at runtime (house rule 6: no credential literal in a test).
fn pass(word: &str) -> String {
    [word, "review", "battery"].join("-")
}

/// A vault member to enrol.
fn member<'a>(
    staff_id: &'a str,
    name: &'a str,
    role: &'a str,
    passphrase: &'a str,
) -> NewMember<'a> {
    NewMember {
        staff_id,
        display_name: name,
        role,
        phone: None,
        email: Some("r@example.org"),
        passphrase,
        hint: None,
    }
}

/// The strip's staff ID and passphrase for `staff_id`.
pub(crate) fn credential(staff_id: &str) -> Option<Credential> {
    Some(Credential {
        staff_id: staff_id.into(),
        passphrase: Zeroizing::new(pass(staff_id)),
    })
}

/// The fixture above: the serial guard (bind it), the owner's client, the window's state with
/// who's here naming Sue, the chart, and Bea's part. `None` without `CAIRN_TEST_PG`.
pub(crate) async fn supervised_note() -> Option<(
    tokio_postgres::Client,
    tokio_postgres::Client,
    AppState,
    Uuid,
    Uuid,
)> {
    let cs = std::env::var("CAIRN_TEST_PG").ok()?;
    let guard = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    let mut owner = cairn_node::db::connect_and_load_schema(&cs).await.unwrap();
    // The HLC is reset too: a part is timed by it, a supervision row by the DB clock, and a
    // clock a previous suite left ahead would time parts after acts recorded later.
    owner
        .batch_execute(
            "TRUNCATE clinical_note, clinical_note_attestation, note_draft, note_draft_spent, \
             note_policy, vault_attempt, vault_member, vault_terminal, vault_policy, \
             patient_link, person_member, link_veto_flag, event_log, actor_event, patient_chart, \
             node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE; \
             UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0",
        )
        .await
        .unwrap();
    let (node_sk, node_kid) = cairn_event::generate_key().unwrap();
    owner
        .execute(
            "SELECT enroll_actor('device', \
             '{\"role\":\"registration-desk\",\"node_key\":\"notes-review-test\"}', $1)",
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

    // The vault: a superuser, who enrols the three clinicians with their own passphrases.
    bootstrap_superuser(
        &owner,
        &member("boss", "The Boss", "superuser", &pass("boss")),
    )
    .await
    .unwrap();
    let window = crate::enrol::tests::window_login(&owner, &cs).await;
    let window_db = cairn_node::db::connect(&window).await.unwrap();
    let release = |staff: &'static str, purpose| {
        let db = &window_db;
        async move {
            match release_for_act(db, staff, &pass(staff), purpose)
                .await
                .unwrap()
            {
                cairn_node::vault::Release::Released { key, kid } => (key, kid),
                other => panic!("{staff}: {other:?}"),
            }
        }
    };
    let (su, _) = release("boss", Purpose::Enrol).await;
    for (staff, name) in [("bea", "Dr Bea"), ("sue", "Dr Sue"), ("zed", "Dr Zed")] {
        enrol_member(
            &owner,
            &member(staff, name, "clinician", &pass(staff)),
            "boss",
            &su,
        )
        .await
        .unwrap();
    }
    owner
        .execute(
            "SELECT cairn_supervision_console_act('link-supervisor', 'bea', 'sue')",
            &[],
        )
        .await
        .unwrap();

    // Bea's note, signed with her own key.
    let (chart, _, _) = cairn_node::john_doe::register_john_doe(
        &mut owner,
        &node_sk,
        &node_kid,
        ORIGIN,
        "ED",
        "test-site",
        "2026-11-07",
        "arrived unconscious",
    )
    .await
    .unwrap();
    let (bea_sk, bea_kid) = release("bea", Purpose::Clinical).await;
    let input = cairn_node::note::sign::SignNoteInput {
        draft: Uuid::now_v7(),
        patient: chart,
        terminal: "fixture-terminal",
        writer: Some("bea"),
        narrative: "obs stable",
        stated: None,
        node_origin: ORIGIN,
        addendum_to: None,
        also_vouches: &[],
    };
    let author = cairn_node::medication::AuthorParams {
        human_sk: &bea_sk,
        human_kid: &bea_kid,
    };
    let part = cairn_node::note::sign::sign_note(&mut owner, &node_sk, &input, &author)
        .await
        .unwrap()
        .expect("Bea's note signs")
        .event_id;

    let mut state = AppState::mock(Some(chart));
    state.db = Some(std::sync::Arc::new(cairn_gui_live::NodeConnection::fixed(
        window_db,
    )));
    state.node_sk = Some(node_sk);
    state.node_origin = ORIGIN.into();
    state.key_hold = std::time::Duration::from_secs(600);
    state.whos_here.lock().await.pick(Claim::Named {
        staff_id: "sue".into(),
        display_name: "Dr Sue".into(),
    });
    Some((guard, owner, state, chart, part))
}

/// How many vouches the projection holds (read as the owner: the window may not).
async fn vouch_count(owner: &tokio_postgres::Client) -> i64 {
    owner
        .query_one("SELECT count(*) FROM clinical_note_attestation", &[])
        .await
        .unwrap()
        .get(0)
}

/// Sue, named in who's here and signing with her own key, co-signs Bea's note as reviewed: the
/// pane offers it (and not Sign off: Bea signed every part and owes nothing), the press writes
/// one co-sign, the note reads it, and it is not offered or written again.
#[tokio::test]
async fn a_supervisor_reviews_from_the_window() {
    let Some((_guard, owner, state, chart, part)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (patient, root) = (chart.to_string(), part.to_string());
    let pane = note_list_impl(&state, &patient).await.unwrap();
    let note = &pane.notes[0];
    assert!(note.can_review, "Sue supervises Bea");
    assert!(!note.can_sign_off, "Bea's note owes no sign-off");

    let report = note_review_cosign_impl(
        &state,
        &patient,
        &root,
        std::slice::from_ref(&root),
        credential("sue"),
    )
    .await
    .expect("Sue's review is written");
    assert_eq!(report.text, REVIEWED);
    assert!(report.behind_the_record.is_none());
    assert_eq!(vouch_count(&owner).await, 1);

    let pane = note_list_impl(&state, &patient).await.unwrap();
    let note = &pane.notes[0];
    assert!(!note.can_review, "reviewed once is enough");
    assert!(
        note.byline.contains("co-signed by Dr Sue"),
        "{}",
        note.byline
    );
    // Sue's key is held now: a second press needs no passphrase, and signs nothing.
    let again = note_review_cosign_impl(&state, &patient, &root, std::slice::from_ref(&root), None)
        .await
        .unwrap_err();
    assert_eq!(
        again.text,
        "There is nothing here for you to co-sign as reviewed. Nothing was signed."
    );
    assert_eq!(vouch_count(&owner).await, 1);
}

/// Review Focus 5, the window's half: who's here names Sue, so the pane offers the review, but
/// the strip releases Zed's key (his staff ID and passphrase). The KEY decides: Zed supervises
/// no one, so nothing is signed, and the refusal says so.
#[tokio::test]
async fn the_key_not_the_claim_decides_in_the_window() {
    let Some((_guard, owner, state, chart, part)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (patient, root) = (chart.to_string(), part.to_string());
    let pane = note_list_impl(&state, &patient).await.unwrap();
    assert!(pane.notes[0].can_review, "the claim names Bea's supervisor");

    let err = note_review_cosign_impl(
        &state,
        &patient,
        &root,
        std::slice::from_ref(&root),
        credential("zed"),
    )
    .await
    .unwrap_err();
    assert_eq!(
        err.text,
        "There is nothing here for you to co-sign as reviewed. Nothing was signed."
    );
    assert_eq!(vouch_count(&owner).await, 0, "Zed's key wrote no vouch");
}
