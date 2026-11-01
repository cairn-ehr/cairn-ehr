//! Tests for `enrol.rs`: a superuser enrols staff from the window.
//!
//! The pure checks need no database and are tested first, because the brief's rule is that a
//! typo in the new member's passphrase is caught BEFORE the vault is asked anything. The
//! DB-gated tests (`CAIRN_TEST_PG`) pin the three vault-facing promises: a clinician cannot
//! enrol, the superuser's key is never held, and no sentence echoes a passphrase.
use super::*;
use crate::signer::Credential;
use crate::state::{AppState, Now};
use cairn_node::vault::{bootstrap_superuser, NewMember};
use std::time::Duration;
use zeroize::Zeroizing;

/// A test passphrase built at runtime (house rule 6: no credential literal in a test).
fn pass(word: &str) -> String {
    [word, "horse", "battery"].join("-")
}

fn form(first: &str, again: &str) -> NewMemberForm {
    NewMemberForm {
        staff_id: "newbie".into(),
        display_name: "Nina Newbie".into(),
        role: "nurse".into(),
        phone: "0400 000 000".into(),
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
    assert_eq!(check_form(&form(&pass("a"), &pass("a"))), Ok(()));
}

#[test]
fn mismatched_passphrases_are_refused_before_the_vault() {
    // A fixture-mode state has NO database: if the impl reached for the vault it would say so
    // ("cannot enrol in fixture mode"); the mismatch sentence proves the check ran first.
    let state = AppState::mock(None);
    let said = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(enrol_member_impl(
            &state,
            credential("boss", &pass("s")),
            form(&pass("a"), &pass("b")),
        ));
    let said = said.expect_err("a mismatch is refused");
    assert_eq!(said, EnrolRefusal::PassphrasesDiffer.sentence());
    assert!(said.contains("do not match"), "{said}");
}

#[test]
fn an_empty_passphrase_is_refused() {
    assert_eq!(
        check_form(&form("", "")),
        Err(EnrolRefusal::PassphraseEmpty)
    );
}

/// Spaces alone are no passphrase: sign-in would never send one to the vault, so enrolling it
/// seals a key its owner can never use (PR #897 review).
#[test]
fn a_passphrase_of_spaces_is_refused() {
    assert_eq!(
        check_form(&form("   ", "   ")),
        Err(EnrolRefusal::PassphraseEmpty)
    );
}

/// A failed enrolment the server did not answer may have been recorded: the superuser is told to
/// check, never "nothing was recorded", which invites enrolling the person again under a new ID.
#[test]
fn an_unanswered_enrolment_failure_is_outcome_unknown() {
    let lost = anyhow::anyhow!("connection closed");
    assert_eq!(enrol_failure(&lost), EnrolRefusal::OutcomeUnknown);
    let said = EnrolRefusal::OutcomeUnknown.sentence();
    assert!(said.contains("may have been enrolled"), "{said}");
    assert!(!said.contains("Nothing was recorded"), "{said}");
}

/// A typed superuser passphrase means someone new is at the keyboard, as on a clinical act: the
/// held key is wiped first, so the next act with an empty strip cannot sign as its holder.
#[test]
fn enrolling_wipes_a_held_key() {
    let mut state = AppState::mock(None);
    state.key_hold = Duration::from_secs(900);
    state.session.try_lock().unwrap().key = Some(crate::state::SessionKey::for_test(Now::read()));
    let said = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(enrol_member_impl(
            &state,
            credential("boss", &pass("s")),
            form(&pass("a"), &pass("a")),
        ));
    assert_eq!(said, Err(EnrolRefusal::Fixture.sentence()));
    assert!(
        state.session.try_lock().unwrap().key.is_none(),
        "the held key was wiped"
    );
}

#[test]
fn a_member_needs_a_phone_or_an_email() {
    let mut f = form(&pass("a"), &pass("a"));
    f.phone = "  ".into();
    assert_eq!(check_form(&f), Err(EnrolRefusal::NeedContact));
    f.email = "n@example.org".into();
    assert_eq!(check_form(&f), Ok(()));
}

#[test]
fn a_member_needs_an_id_a_name_and_a_role() {
    for blank in ["staff_id", "display_name", "role"] {
        let mut f = form(&pass("a"), &pass("a"));
        match blank {
            "staff_id" => f.staff_id = " ".into(),
            "display_name" => f.display_name = "".into(),
            _ => f.role = "".into(),
        }
        assert_eq!(check_form(&f), Err(EnrolRefusal::MissingDetail), "{blank}");
    }
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
        EnrolRefusal::PassphrasesDiffer,
        EnrolRefusal::PassphraseEmpty,
        EnrolRefusal::NeedContact,
        EnrolRefusal::MissingDetail,
        EnrolRefusal::NotForEnrolment,
        EnrolRefusal::NotRecorded,
        EnrolRefusal::OutcomeUnknown,
        EnrolRefusal::Fixture,
        EnrolRefusal::AlreadyEnrolled,
        EnrolRefusal::NeedSuperuser,
        EnrolRefusal::ApproverMayNotEnrolThisRole,
        EnrolRefusal::ApproverNotConfirmed,
    ] {
        let s = r.sentence();
        assert!(!s.is_empty() && !s.contains('\u{2014}'), "{s}");
    }
}

/// The ORDER the DB-gated tests below prove, pinned where CI can see it: CI's GUI job has no
/// database, so those tests skip there (PR #897 review). Held key wiped, then the duplicate asked
/// about, then the superuser's key released, then the enrolment; and never through the held
/// session (`signer_for_act` may hold a key, and a superuser's must never be held).
#[test]
fn the_enrolment_runs_its_steps_in_order() {
    let body: String = crate::source_guard::rust_fn(include_str!("enrol.rs"), "enrol_member_impl")
        .split_whitespace()
        .collect();
    let order = [
        "check_form(&member)",
        "if!credential.has_passphrase(){",
        "state.forget_held_key().await;",
        "cairn_node::vault::staff_id_taken(",
        "release_enrol_key(",
        "cairn_node::vault::enrol_member(",
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

// ---- DB-gated ----

const SU_STAFF: &str = "boss";

/// A connection string that logs in as the reference window does (#898): a
/// `cairn_window_probe` LOGIN granted db/062's `cairn_window` group role, never the owner.
/// `release_for_act` refuses the owner's connection, so the window under test must use this.
///
/// DUPLICATED, deliberately, from `crates/cairn-node/tests/common/mod.rs` (and
/// `cairn-gui/cairn-gui-live/tests/common/mod.rs`): this is a src unit-test module with no
/// `tests/common` to import, three test trees cannot share a test module, and a library fn that
/// mints passwordless logins is not something to ship. Keep the copies identical.
async fn window_login(owner: &tokio_postgres::Client, cs: &str) -> String {
    owner
        .batch_execute(
            "DO $$ BEGIN \
               IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_window_probe') THEN \
                 CREATE ROLE cairn_window_probe LOGIN; \
               END IF; \
             EXCEPTION WHEN duplicate_object THEN NULL; \
             END $$; \
             GRANT cairn_window TO cairn_window_probe;",
        )
        .await
        .expect("the window's probe login");
    format!("{cs} user=cairn_window_probe")
}

/// The window's connection, as `AppState` holds it.
fn node(conn: tokio_postgres::Client) -> Option<std::sync::Arc<cairn_gui_live::NodeConnection>> {
    Some(std::sync::Arc::new(cairn_gui_live::NodeConnection::fixed(
        conn,
    )))
}

/// A state over the real test database with a superuser `boss` and a clinician `nurse1`
/// enrolled, on a 900 s terminal (the longest hold: the case "never held" must survive).
/// The state's connection is the WINDOW's login (#898); the second client is the owner's, for
/// setup and for reading what the window may not (the vault's tables, its attempt counts).
/// Returns the serial guard too: bind it, or another suite interleaves its TRUNCATE.
async fn live(
    su_pass: &str,
    clin_pass: &str,
) -> Option<(tokio_postgres::Client, tokio_postgres::Client, AppState)> {
    let cs = std::env::var("CAIRN_TEST_PG").ok()?;
    let guard = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&cs).await.unwrap();
    c.batch_execute(
        "TRUNCATE vault_attempt, vault_member, vault_terminal, event_log, actor_event, \
         patient_chart, node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
    )
    .await
    .unwrap();
    let member = |staff_id, role, passphrase| NewMember {
        staff_id,
        display_name: "Test Person",
        role,
        phone: None,
        email: Some("t@example.org"),
        passphrase,
        hint: None,
    };
    bootstrap_superuser(&c, &member(SU_STAFF, "superuser", su_pass))
        .await
        .unwrap();
    let window = window_login(&c, &cs).await;
    let su = match cairn_node::vault::release_for_act(
        &cairn_node::db::connect(&window).await.unwrap(),
        SU_STAFF,
        su_pass,
        cairn_node::vault::Purpose::Enrol,
    )
    .await
    .unwrap()
    {
        cairn_node::vault::Release::Released { key, .. } => key,
        other => panic!("{other:?}"),
    };
    cairn_node::vault::enrol_member(&c, &member("nurse1", "clinician", clin_pass), SU_STAFF, &su)
        .await
        .unwrap();
    let mut state = AppState::mock(None);
    state.db = node(cairn_node::db::connect(&window).await.unwrap());
    state.key_hold = Duration::from_secs(900);
    Some((guard, c, state))
}

#[tokio::test]
async fn a_clinician_credential_cannot_enrol() {
    let (su_p, cl_p) = (pass("su"), pass("cl"));
    let Some((_g, _owner, state)) = live(&su_p, &cl_p).await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let said = enrol_member_impl(
        &state,
        credential("nurse1", &cl_p),
        form(&pass("n"), &pass("n")),
    )
    .await
    .expect_err("a clinician's key is not released for an enrolment");
    assert_eq!(said, EnrolRefusal::NotForEnrolment.sentence());
    assert!(!said.contains(&cl_p));
}

#[tokio::test]
async fn the_superuser_key_is_never_held() {
    let (su_p, cl_p, new_p) = (pass("su"), pass("cl"), pass("new"));
    let Some((_g, _owner, state)) = live(&su_p, &cl_p).await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let said = enrol_member_impl(&state, credential(SU_STAFF, &su_p), form(&new_p, &new_p))
        .await
        .expect("the superuser enrols");
    assert!(
        said.contains("Nina Newbie") && said.contains("newbie"),
        "{said}"
    );
    for secret in [&su_p, &cl_p, &new_p] {
        assert!(!said.contains(secret.as_str()));
    }
    // 900 s hold, and still nothing held: an enrol act is not a clinical act.
    assert_eq!(state.key_status(Now::read()).await, None);
    assert!(state.session.lock().await.key.is_none());
    // The new member really is in the vault, with the passphrase they typed.
    let db = state.db.as_ref().unwrap().lock().await.unwrap();
    let released = cairn_node::vault::release_for_act(
        &db,
        "newbie",
        &new_p,
        cairn_node::vault::Purpose::Clinical,
    )
    .await
    .unwrap();
    assert!(matches!(
        released,
        cairn_node::vault::Release::Released { .. }
    ));
}

#[tokio::test]
async fn a_wrong_superuser_passphrase_enrols_nobody_and_echoes_nothing() {
    let (su_p, cl_p, new_p, wrong) = (pass("su"), pass("cl"), pass("new"), pass("bad"));
    let Some((_g, owner, state)) = live(&su_p, &cl_p).await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let said = enrol_member_impl(&state, credential(SU_STAFF, &wrong), form(&new_p, &new_p))
        .await
        .expect_err("refused");
    assert!(said.contains("not right"), "{said}");
    for secret in [&su_p, &new_p, &wrong] {
        assert!(!said.contains(secret.as_str()));
    }
    let n: i64 = owner
        .query_one(
            "SELECT count(*) FROM vault_member WHERE staff_id = 'newbie'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0);
}

/// R11: a duplicate staff ID is told plainly, in BOTH shapes the floor refuses differently (same
/// role: `enroll_actor` first; another role: the vault's primary key). The superuser passphrase
/// here is WRONG on purpose: the pre-check runs before the key is released, so the answer is still
/// "already in use" and no release attempt was counted against the superuser.
#[tokio::test]
async fn a_duplicate_staff_id_is_named_before_the_superuser_key_is_released() {
    let (su_p, cl_p, wrong) = (pass("su"), pass("cl"), pass("bad"));
    let Some((_g, owner, state)) = live(&su_p, &cl_p).await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    for role in ["clinician", "nurse"] {
        let mut f = form(&pass("n"), &pass("n"));
        f.staff_id = " NURSE1 ".into();
        f.role = role.into();
        let said = enrol_member_impl(&state, credential(SU_STAFF, &wrong), f)
            .await
            .expect_err("a duplicate is refused");
        assert_eq!(said, EnrolRefusal::AlreadyEnrolled.sentence(), "{role}");
    }
    let failures: i64 = owner
        .query_one(
            "SELECT coalesce(sum(failures), 0)::bigint FROM cairn_vault_attempt_state('boss')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(failures, 0, "the superuser's key was never asked for");
}

/// #898: a window on the OWNER's connection enrols nobody and says why in plain words; the
/// superuser's passphrase never reached the vault, so it was not counted either.
#[tokio::test]
async fn an_over_powered_connection_enrols_nobody_and_says_so() {
    let (su_p, cl_p, new_p) = (pass("su"), pass("cl"), pass("new"));
    let Some((_g, owner, mut state)) = live(&su_p, &cl_p).await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let cs = std::env::var("CAIRN_TEST_PG").unwrap();
    state.db = node(cairn_node::db::connect(&cs).await.unwrap());
    let before: i64 = owner
        .query_one(
            "SELECT coalesce(sum(failures), 0)::bigint FROM cairn_vault_attempt_state('boss')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    let said = enrol_member_impl(
        &state,
        credential(SU_STAFF, &pass("bad")),
        form(&new_p, &new_p),
    )
    .await
    .expect_err("refused");
    assert_eq!(said, crate::signer::KeyRefusal::OverPowered.sentence());
    let row = owner
        .query_one(
            "SELECT (SELECT count(*) FROM vault_member WHERE staff_id = 'newbie'), \
                    (SELECT coalesce(sum(failures), 0)::bigint FROM cairn_vault_attempt_state('boss'))",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, i64>(0), 0, "nobody was enrolled");
    assert_eq!(
        row.get::<_, i64>(1),
        before,
        "the wrong guess was never counted"
    );
}

#[tokio::test]
async fn a_blank_credential_asks_for_a_superuser_not_a_signature() {
    let said = enrol_member_impl(
        &AppState::mock(None),
        credential("", ""),
        form(&pass("a"), &pass("a")),
    )
    .await
    .expect_err("refused");
    assert_eq!(said, EnrolRefusal::NeedSuperuser.sentence());
    assert!(said.contains("enrol staff"), "{said}");
}

/// Re-review item 1: a staff ID with no passphrase (the strip keeps the ID on a held terminal) is
/// no superuser credential. It is refused before the node is asked, so it can never be counted as
/// a superuser's wrong guess. On the fixture state any later step would answer `Fixture`, so
/// `NeedSuperuser` here proves no vault call was reached.
#[tokio::test]
async fn a_staff_id_alone_asks_for_a_superuser_and_never_reaches_the_vault() {
    for blank in ["", "  "] {
        let said = enrol_member_impl(
            &AppState::mock(None),
            credential(SU_STAFF, blank),
            form(&pass("a"), &pass("a")),
        )
        .await
        .expect_err("refused");
        assert_eq!(said, EnrolRefusal::NeedSuperuser.sentence(), "{blank:?}");
    }
}

/// Only a superuser vouches for a superuser or an admin; an enroller may not (the floor says the
/// same, this is the courtesy that spares them a release attempt).
#[test]
fn only_a_superuser_may_enrol_the_powerful_roles() {
    for (role, restricted) in [
        ("superuser", true),
        (" admin ", true),
        ("clinician", false),
        ("nurse", false),
    ] {
        assert_eq!(role_needs_superuser(role), restricted, "{role}");
    }
}

#[test]
fn the_new_refusals_name_no_field_and_no_passphrase() {
    for r in [
        EnrolRefusal::ApproverMayNotEnrolThisRole,
        EnrolRefusal::ApproverNotConfirmed,
    ] {
        let s = r.sentence();
        assert!(!s.is_empty() && !s.contains('\u{2014}'), "{s}");
        assert!(!s.to_lowercase().contains("passphrase"), "{s}");
    }
}

fn row(id: &str, enroller: bool) -> cairn_node::vault_admin::EnrollerRow {
    cairn_node::vault_admin::EnrollerRow {
        staff_id: id.into(),
        display_name: "Test Person".into(),
        enroller,
    }
}

/// The grant panel lists non-superusers only: absent means superuser.
#[test]
fn standing_reads_the_grant_panel_rows() {
    let rows = [row("en1", true), row("cl1", false)];
    assert_eq!(standing(&rows, "en1"), Standing::Granted);
    assert_eq!(standing(&rows, " EN1 "), Standing::Granted);
    assert_eq!(standing(&rows, "cl1"), Standing::NotGranted);
    assert_eq!(standing(&rows, "boss"), Standing::NotListed);
}

/// M2: the pre-check decisions, with no database. After a key was released, only an enroller is
/// ever refused, and only for superuser/admin; a superuser (even with a stray grant, who is
/// still absent from the non-superuser list) is never refused.
#[test]
fn the_precheck_refuses_only_an_enroller_aiming_high() {
    use EnrolRefusal::ApproverMayNotEnrolThisRole as No;
    assert_eq!(precheck_decision(Standing::Granted, "admin"), Some(No));
    assert_eq!(precheck_decision(Standing::Granted, "superuser"), Some(No));
    assert_eq!(precheck_decision(Standing::Granted, "nurse"), None);
    assert_eq!(precheck_decision(Standing::NotListed, "admin"), None);
    assert_eq!(precheck_decision(Standing::NotListed, "superuser"), None);
}

/// A refused enrol purpose: a granted-but-unconfirmed enroller is told to wait; a clinician or an
/// unknown ID gets the plain refusal.
#[test]
fn a_refused_purpose_tells_an_unconfirmed_enroller_from_a_clinician() {
    assert_eq!(
        refused_purpose_decision(Standing::Granted),
        EnrolRefusal::ApproverNotConfirmed
    );
    for s in [Standing::NotGranted, Standing::NotListed] {
        assert_eq!(refused_purpose_decision(s), EnrolRefusal::NotForEnrolment);
    }
}

/// The shared release helper releases with `Purpose::Enrol` and never touches the held session.
#[test]
fn the_shared_release_is_enrol_purpose_and_never_held() {
    let body: String = crate::source_guard::rust_fn(include_str!("enrol.rs"), "release_enrol_key")
        .split_whitespace()
        .collect();
    assert!(body.contains("Purpose::Enrol"), "{body}");
    assert!(
        !body.contains("signer_for_act") && !body.contains("session"),
        "{body}"
    );
}
