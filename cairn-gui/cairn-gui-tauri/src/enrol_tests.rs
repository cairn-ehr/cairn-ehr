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
        EnrolRefusal::Fixture,
        EnrolRefusal::AlreadyEnrolled,
        EnrolRefusal::NeedSuperuser,
    ] {
        let s = r.sentence();
        assert!(!s.is_empty() && !s.contains('\u{2014}'), "{s}");
    }
}

// ---- DB-gated ----

const SU_STAFF: &str = "boss";

/// A state over the real test database with a superuser `boss` and a clinician `nurse1`
/// enrolled, on a 900 s terminal (the longest hold: the case "never held" must survive).
/// Returns the serial guard too: bind it, or another suite interleaves its TRUNCATE.
async fn live(su_pass: &str, clin_pass: &str) -> Option<(tokio_postgres::Client, AppState)> {
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
    let su = match cairn_node::vault::release_for_act(
        &c,
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
    let conn = cairn_node::db::connect_and_load_schema(&cs).await.unwrap();
    state.db = Some(std::sync::Arc::new(cairn_gui_live::NodeConnection::fixed(
        conn,
    )));
    state.key_hold = Duration::from_secs(900);
    Some((guard, state))
}

#[tokio::test]
async fn a_clinician_credential_cannot_enrol() {
    let (su_p, cl_p) = (pass("su"), pass("cl"));
    let Some((_g, state)) = live(&su_p, &cl_p).await else {
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
    let Some((_g, state)) = live(&su_p, &cl_p).await else {
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
    let Some((_g, state)) = live(&su_p, &cl_p).await else {
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
    let db = state.db.as_ref().unwrap().lock().await.unwrap();
    let n: i64 = db
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
    let Some((_g, state)) = live(&su_p, &cl_p).await else {
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
    let db = state.db.as_ref().unwrap().lock().await.unwrap();
    let failures: i64 = db
        .query_one(
            "SELECT coalesce(sum(failures), 0)::bigint FROM cairn_vault_attempt_state('boss')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(failures, 0, "the superuser's key was never asked for");
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
