//! Tests for `reseal.rs`. All DB-free (the vault round trip is covered by cairn-node's
//! vault_admin tests).
use super::*;
use zeroize::Zeroizing;

/// A test passphrase built at runtime (house rule 6: no credential literal in a test).
fn pass(word: &str) -> String {
    [word, "horse", "battery"].join("-")
}

fn form(current: &str, new: &str, again: &str) -> ResealForm {
    ResealForm {
        staff_id: "nurse1".into(),
        current: Zeroizing::new(current.to_string()),
        new: Zeroizing::new(new.to_string()),
        new_again: Zeroizing::new(again.to_string()),
        hint: String::new(),
    }
}

#[test]
fn a_good_form_passes() {
    assert_eq!(
        check_reseal_form(&form(&pass("a"), &pass("b"), &pass("b"))),
        Ok(())
    );
}

#[test]
fn the_new_passphrase_must_differ_match_and_be_usable() {
    assert_eq!(
        check_reseal_form(&form(&pass("a"), &pass("a"), &pass("a"))),
        Err(ResealRefusal::NewSameAsCurrent)
    );
    assert_eq!(
        check_reseal_form(&form(&pass("a"), &pass("b"), &pass("c"))),
        Err(ResealRefusal::NewsDiffer)
    );
    assert_eq!(
        check_reseal_form(&form(&pass("a"), "   ", "   ")),
        Err(ResealRefusal::NewEmpty)
    );
    assert_eq!(
        check_reseal_form(&form("", &pass("b"), &pass("b"))),
        Err(ResealRefusal::MissingDetail)
    );
}

#[test]
fn no_sentence_or_debug_carries_a_passphrase() {
    let f = form(&pass("cur"), &pass("new"), &pass("new"));
    let shown = format!("{f:?}");
    for secret in [pass("cur"), pass("new")] {
        assert!(!shown.contains(&secret));
    }
    for r in [
        ResealRefusal::MissingDetail,
        ResealRefusal::NewEmpty,
        ResealRefusal::NewSameAsCurrent,
        ResealRefusal::NewsDiffer,
        ResealRefusal::NotChanged,
        ResealRefusal::OutcomeUnknown,
        ResealRefusal::Fixture,
    ] {
        assert!(!r.sentence().contains("horse"));
    }
}

/// Fixture mode has no vault: refused with its own sentence, before anything is asked.
#[tokio::test]
async fn fixture_mode_refuses() {
    let state = AppState::mock(None);
    let said = change_passphrase_impl(&state, form(&pass("a"), &pass("b"), &pass("b")))
        .await
        .unwrap_err();
    assert_eq!(said, ResealRefusal::Fixture.sentence());
}
