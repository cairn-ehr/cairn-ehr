//! Tests for `rekey.rs`: an enroller (or superuser) gives a colleague who forgot their passphrase
//! a new key, from the window. The pure checks need no database; the DB-gated test self-skips
//! without `CAIRN_TEST_PG`.
use super::*;
use crate::signer::Credential;
use crate::state::AppState;
use zeroize::Zeroizing;

/// A test passphrase built at runtime (house rule 6: no credential literal in a test).
fn pass(word: &str) -> String {
    [word, "horse", "battery"].join("-")
}

fn form(first: &str, again: &str) -> RekeyForm {
    RekeyForm {
        staff_id: "nurse1".into(),
        role: "clinician".into(),
        display_name: String::new(),
        phone: String::new(),
        email: String::new(),
        passphrase: Zeroizing::new(first.to_string()),
        passphrase_again: Zeroizing::new(again.to_string()),
        hint: String::new(),
    }
}

fn credential(staff_id: &str, passphrase: &str) -> Credential {
    Credential {
        staff_id: staff_id.into(),
        passphrase: Zeroizing::new(passphrase.to_string()),
    }
}

#[test]
fn matching_passphrases_pass_the_check() {
    assert_eq!(check_rekey_form(&form(&pass("a"), &pass("a"))), Ok(()));
}

#[test]
fn differing_or_empty_passphrases_are_refused() {
    assert_eq!(
        check_rekey_form(&form(&pass("a"), &pass("b"))),
        Err(RekeyRefusal::PassphrasesDiffer)
    );
    assert_eq!(
        check_rekey_form(&form("", "")),
        Err(RekeyRefusal::PassphraseEmpty)
    );
    assert_eq!(
        check_rekey_form(&form("   ", "   ")),
        Err(RekeyRefusal::PassphraseEmpty)
    );
}

#[test]
fn a_rekey_needs_an_id_and_a_role() {
    let mut f = form(&pass("a"), &pass("a"));
    f.role = "  ".into();
    assert_eq!(check_rekey_form(&f), Err(RekeyRefusal::MissingDetail));
}

/// A mismatch is refused before any database call: a fixture state (no database) would otherwise
/// answer with the fixture sentence.
#[test]
fn a_mismatch_never_reaches_the_vault() {
    let state = AppState::mock(None);
    let said = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(rekey_member_impl(
            &state,
            credential("boss", &pass("s")),
            form(&pass("a"), &pass("b")),
        ))
        .expect_err("refused");
    assert_eq!(said, RekeyRefusal::PassphrasesDiffer.sentence());
}

#[test]
fn debug_never_prints_a_passphrase() {
    let (a, b) = (pass("first"), pass("second"));
    let shown = format!("{:?}", form(&a, &b));
    assert!(!shown.contains(&a) && !shown.contains(&b), "{shown}");
    assert!(shown.contains("redacted"), "{shown}");
}

#[test]
fn every_sentence_is_plain_and_names_no_value() {
    for r in [
        RekeyRefusal::PassphrasesDiffer,
        RekeyRefusal::PassphraseEmpty,
        RekeyRefusal::MissingDetail,
        RekeyRefusal::NeedNameAndContact,
        RekeyRefusal::NotRecorded,
        RekeyRefusal::OutcomeUnknown,
        RekeyRefusal::Fixture,
    ] {
        let s = r.sentence();
        assert!(!s.is_empty() && !s.contains('\u{2014}'), "{s}");
    }
}

/// After a restore the vault has no row for the person, and the door then needs a display name
/// and a phone or an email to re-create it (design §5.8). The window asks whether the staff ID
/// has a row BEFORE any key is released, so a missing detail is told plainly and costs the
/// approver no release (never by parsing the floor's refusal). With a row, blanks keep the
/// stored values and nothing more is needed.
#[test]
fn a_rekey_with_no_member_row_needs_a_name_and_a_contact() {
    let mut f = form(&pass("a"), &pass("a"));
    assert_eq!(
        missing_member_details(true, &f),
        None,
        "a row keeps its values"
    );
    assert_eq!(
        missing_member_details(false, &f),
        Some(RekeyRefusal::NeedNameAndContact)
    );
    f.display_name = "Nurse One".into();
    assert_eq!(
        missing_member_details(false, &f),
        Some(RekeyRefusal::NeedNameAndContact),
        "a name alone is not enough: the row needs a contact"
    );
    f.phone = " 0400 000 000 ".into();
    assert_eq!(missing_member_details(false, &f), None);
    f.phone = String::new();
    f.email = "n1@example.org".into();
    assert_eq!(missing_member_details(false, &f), None);
    f.display_name = "   ".into();
    assert_eq!(
        missing_member_details(false, &f),
        Some(RekeyRefusal::NeedNameAndContact)
    );
}

/// The success line says who must still act: an enroller's re-key is unconfirmed until a
/// superuser confirms it; a superuser's is confirmed at once (db/063). Who approved is read from
/// the grant panel (`enrollers()` lists non-superusers only); a failed read words it neutrally.
#[test]
fn the_success_line_says_whether_a_superuser_must_still_confirm() {
    use crate::enrol::Standing;
    let enroller = rekey_success("nurse1", Some(Standing::Granted));
    assert_eq!(
        enroller,
        "nurse1 has a new key. Their old passphrase no longer works. A superuser must confirm the \
         new key."
    );
    let superuser = rekey_success("nurse1", Some(Standing::NotListed));
    assert_eq!(
        superuser,
        "nurse1 has a new key. Their old passphrase no longer works."
    );
    let unknown = rekey_success("nurse1", None);
    assert!(
        unknown.contains("If an enroller approved it, a superuser must confirm the new key."),
        "{unknown}"
    );
}

/// Same order as the enrol: held key wiped, then the shared enrol-purpose release, then the re-key.
#[test]
fn the_rekey_runs_its_steps_in_order() {
    let body: String = crate::source_guard::rust_fn(include_str!("rekey.rs"), "rekey_member_impl")
        .split_whitespace()
        .collect();
    let order = [
        "check_rekey_form(&member)",
        "state.forget_held_key().await;",
        "cairn_node::vault::staff_id_taken(",
        "missing_member_details(",
        "release_enrol_key(",
        "cairn_node::vault_admin::rekey_member(",
        "rekey_success(",
    ];
    let at: Vec<usize> = order
        .iter()
        .map(|shape| {
            body.find(shape)
                .unwrap_or_else(|| panic!("{shape}\n{body}"))
        })
        .collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{order:?}\n{body}");
    assert!(
        !body.contains("signer_for_act") && !body.contains("session"),
        "{body}"
    );
}

/// The window's own login, as enrol_tests (#898): a LOGIN in the cairn_window group.
async fn window_login(owner: &tokio_postgres::Client, cs: &str) -> tokio_postgres::Client {
    owner
        .batch_execute(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_window_probe') \
             THEN CREATE ROLE cairn_window_probe LOGIN; END IF; \
             EXCEPTION WHEN duplicate_object THEN NULL; END $$; \
             GRANT cairn_window TO cairn_window_probe;",
        )
        .await
        .unwrap();
    cairn_node::db::connect(&format!("{cs} user=cairn_window_probe"))
        .await
        .unwrap()
}

/// DB-gated setup shared by the two tests below: an enroller re-keys `nurse1` from the window.
/// `None` (self-skip) without `CAIRN_TEST_PG`. Returns the serial guard, the window state, the
/// nurse's OLD passphrase, and the re-key's success sentence.
async fn rekeyed_world() -> Option<(impl Sized, AppState, String, String)> {
    use cairn_node::vault::{bootstrap_superuser, NewMember, Purpose, Release};
    let Ok(cs) = std::env::var("CAIRN_TEST_PG") else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    let owner = cairn_node::db::connect_and_load_schema(&cs).await.unwrap();
    owner
        .batch_execute(
            "TRUNCATE vault_attempt, vault_member, vault_terminal, vault_policy, event_log, actor_event, \
             patient_chart, node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
        )
        .await
        .unwrap();
    let (su_p, en_p, old_p, new_p) = (pass("su"), pass("en"), pass("old"), pass("new"));
    let member = |staff_id, role, passphrase| NewMember {
        staff_id,
        display_name: "Test Person",
        role,
        phone: None,
        email: Some("t@example.org"),
        passphrase,
        hint: None,
    };
    bootstrap_superuser(&owner, &member("boss", "superuser", &su_p))
        .await
        .unwrap();
    let window = window_login(&owner, &cs).await;
    let su_key = match cairn_node::vault::release_for_act(&window, "boss", &su_p, Purpose::Enrol)
        .await
        .unwrap()
    {
        Release::Released { key, .. } => key,
        other => panic!("{other:?}"),
    };
    for (id, role, p) in [
        ("enrol1", "clinician", &en_p),
        ("nurse1", "clinician", &old_p),
    ] {
        cairn_node::vault::enrol_member(&owner, &member(id, role, p), "boss", &su_key)
            .await
            .unwrap();
    }
    // Grant enroller to enrol1 and confirm their key: the superuser's console-side calls.
    // (Confirmation of an enroller's own key is a superuser act: use the library doors.)
    let su_signer = &su_key;
    cairn_node::vault_admin::set_enroller(&owner, "enrol1", true, "boss", su_signer)
        .await
        .unwrap();
    let pending = cairn_node::vault_admin::unconfirmed(&owner).await.unwrap();
    for u in pending.iter().filter(|u| u.staff_id == "enrol1") {
        cairn_node::vault_admin::judge_key(&owner, &u.kid, true, "boss", su_signer)
            .await
            .unwrap();
    }
    let mut state = AppState::mock(None);
    state.db = Some(std::sync::Arc::new(cairn_gui_live::NodeConnection::fixed(
        window,
    )));
    let said = rekey_member_impl(&state, credential("enrol1", &en_p), form(&new_p, &new_p))
        .await
        .expect("an enroller re-keys a clinician");
    assert!(!said.contains(&new_p) && !said.contains(&en_p));
    Some((guard, state, old_p, said))
}

/// DB-gated (run locally, #905): the OLD passphrase is told `Replaced` by the vault.
#[tokio::test]
async fn an_enroller_rekeys_from_the_window() {
    let Some((_guard, state, old_p, said)) = rekeyed_world().await else {
        return;
    };
    assert!(said.contains("nurse1"), "{said}");
    assert!(
        said.ends_with("A superuser must confirm the new key."),
        "an enroller approved: {said}"
    );
    let db = state.db.as_ref().unwrap().lock().await.unwrap();
    let old = cairn_node::vault::release_for_act(
        &db,
        "nurse1",
        &old_p,
        cairn_node::vault::Purpose::Clinical,
    )
    .await
    .unwrap();
    assert!(
        matches!(old, cairn_node::vault::Release::Replaced { .. }),
        "{old:?}"
    );
}

/// DB-gated: signing with the OLD passphrase after a re-key says the passphrase was replaced and
/// names the approver (the enroller's display name). Self-skips without `CAIRN_TEST_PG`.
#[tokio::test]
async fn a_replaced_sign_names_the_approver() {
    let Some((_guard, state, old_p, _said)) = rekeyed_world().await else {
        return;
    };
    let refusal = state
        .signer_for_act(
            Some(credential("nurse1", &old_p)),
            crate::state::Now::read(),
        )
        .await
        .unwrap_err();
    let said = refusal.sentence();
    assert!(
        said.starts_with("This passphrase was replaced on "),
        "{said}"
    );
    assert!(
        said.contains(", approved by Test Person. If you didn't ask for that, tell a superuser."),
        "{said}"
    );
}

/// DB-gated (run locally, #905): the restore path, design §5.8. The vault is emptied and the
/// registry kept, the console re-keys the superuser, and the superuser then re-keys a clinician
/// from the window. Without a name and a contact the window refuses before any key is released;
/// with them the member row is re-created on the SAME actor, the line is the superuser-approved
/// wording (confirmed at once), and the new passphrase releases the key. Self-skips without
/// `CAIRN_TEST_PG`.
#[tokio::test]
async fn a_superuser_rekeys_a_restored_member_from_the_window() {
    use cairn_node::vault::{bootstrap_superuser, NewMember, Purpose, Release};
    let Ok(cs) = std::env::var("CAIRN_TEST_PG") else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    let owner = cairn_node::db::connect_and_load_schema(&cs).await.unwrap();
    owner
        .batch_execute(
            "TRUNCATE vault_attempt, vault_member, vault_terminal, vault_policy, event_log, actor_event, \
             patient_chart, node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
        )
        .await
        .unwrap();
    let (su_p, su_p2, old_p, new_p) = (pass("su"), pass("su2"), pass("old"), pass("new"));
    let member = |staff_id, role, passphrase| NewMember {
        staff_id,
        display_name: "Test Person",
        role,
        phone: None,
        email: Some("t@example.org"),
        passphrase,
        hint: None,
    };
    bootstrap_superuser(&owner, &member("boss", "superuser", &su_p))
        .await
        .unwrap();
    let window = window_login(&owner, &cs).await;
    let su_key = match cairn_node::vault::release_for_act(&window, "boss", &su_p, Purpose::Enrol)
        .await
        .unwrap()
    {
        Release::Released { key, .. } => key,
        other => panic!("{other:?}"),
    };
    cairn_node::vault::enrol_member(
        &owner,
        &member("nurse1", "clinician", &old_p),
        "boss",
        &su_key,
    )
    .await
    .unwrap();
    // A restore: the registry comes back, the vault (outside every backup) does not.
    owner
        .batch_execute("TRUNCATE vault_attempt, vault_member CASCADE")
        .await
        .unwrap();
    cairn_node::vault_admin::rekey_superuser_from_console(
        &owner,
        &cairn_node::vault_admin::Rekey {
            staff_id: "boss",
            role: "superuser",
            display_name: Some("Site Admin"),
            phone: None,
            email: Some("it@example.org"),
            passphrase: &su_p2,
            hint: None,
        },
    )
    .await
    .unwrap();
    let mut state = AppState::mock(None);
    state.db = Some(std::sync::Arc::new(cairn_gui_live::NodeConnection::fixed(
        window,
    )));

    let blank = rekey_member_impl(&state, credential("boss", &su_p2), form(&new_p, &new_p))
        .await
        .expect_err("no row: a name and a contact are needed first");
    assert_eq!(blank, RekeyRefusal::NeedNameAndContact.sentence());

    let mut filled = form(&new_p, &new_p);
    filled.display_name = "Nurse One".into();
    filled.email = "n1@example.org".into();
    let said = rekey_member_impl(&state, credential("boss", &su_p2), filled)
        .await
        .expect("a superuser re-keys a restored member");
    assert_eq!(
        said,
        "nurse1 has a new key. Their old passphrase no longer works."
    );
    let db = state.db.as_ref().unwrap().lock().await.unwrap();
    let released = cairn_node::vault::release_for_act(&db, "nurse1", &new_p, Purpose::Clinical)
        .await
        .unwrap();
    assert!(matches!(released, Release::Released { .. }), "{released:?}");
}
