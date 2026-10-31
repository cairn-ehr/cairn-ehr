//! Tests for `signer.rs`: who signs the next act, and what the clinician is told when nobody can.
//!
//! `decide` is pure, so the hold rules are tested here with no database: the shared-terminal
//! default (hold 0), a personal workstation's hold, a new signer at a held terminal, and an
//! expired hold. The sentences are tested for the one property no reviewer can see by reading
//! them one at a time: none of them can ever carry the passphrase.
use super::*;
use crate::state::{Now, SessionKey, IDLE_TIMEOUT};
use std::time::Duration;

/// A held key whose owner is `staff_id`, last used at `now`.
fn held_by(staff_id: &str, now: Now) -> SessionKey {
    let mut held = SessionKey::for_test(now);
    held.staff_id = staff_id.to_string();
    held
}

/// A test passphrase, built at runtime (house rule 6: no credential literal in a test).
fn test_passphrase() -> String {
    ["correct", "horse", "battery"].join("-")
}

fn credential(staff_id: &str) -> Credential {
    Credential {
        staff_id: staff_id.to_string(),
        passphrase: Zeroizing::new(test_passphrase()),
    }
}

/// The shared-terminal default: a key is never held, so even a fresh one in the slot (there
/// should be none) does not sign. The next person cannot write as the last one.
#[test]
fn with_no_hold_every_act_needs_a_credential() {
    let now = Now::read();
    let held = held_by("jsmith", now);
    assert_eq!(
        decide(Some(&held), None, Duration::ZERO, now),
        HeldDecision::NeedCredential
    );
    assert_eq!(
        decide(None, None, Duration::ZERO, now),
        HeldDecision::NeedCredential
    );
}

/// A personal workstation: one passphrase, then the whole ward round (#288's one-gesture
/// sign-off) until the hold runs out.
#[test]
fn a_held_key_serves_the_next_act() {
    let now = Now::read();
    let held = held_by("jsmith", now);
    let later = now.plus(Duration::from_secs(60));
    assert_eq!(
        decide(Some(&held), None, Duration::from_secs(900), later),
        HeldDecision::UseHeld
    );
}

/// Review focus 2: the next nurse types her own credential, and it is HERS that signs. A
/// credential is never ignored in favour of a held key, whoever holds it.
#[test]
fn a_new_signer_replaces_the_held_one() {
    let now = Now::read();
    let held = held_by("jsmith", now);
    assert_eq!(
        decide(
            Some(&held),
            Some(&credential("apark")),
            Duration::from_secs(900),
            now
        ),
        HeldDecision::Release
    );
    // And with no hold at all, a credential is the only way to sign.
    assert_eq!(
        decide(None, Some(&credential("apark")), Duration::ZERO, now),
        HeldDecision::Release
    );
}

/// Review focus 2's other half: once the hold has run out, a write with no credential never
/// signs as the previous person. Crossed on the monotonic clock, and across a machine sleep the
/// monotonic clock did not count (`Now`'s module doc).
#[test]
fn an_expired_hold_needs_a_credential() {
    let now = Now::read();
    let held = held_by("jsmith", now);
    for later in [now.plus(IDLE_TIMEOUT), now.slept(IDLE_TIMEOUT)] {
        assert_eq!(
            decide(Some(&held), None, IDLE_TIMEOUT, later),
            HeldDecision::NeedCredential
        );
    }
    // A hold shorter than the idle timeout ends at the hold, not at the idle timeout.
    let hold = Duration::from_secs(60);
    assert_eq!(
        decide(Some(&held), None, hold, now.plus(hold)),
        HeldDecision::NeedCredential
    );
}

/// Review focus 3: no sentence carries the passphrase (they are built from the enum alone, so
/// this pins that nobody adds a field that could), and a wait names its seconds.
#[test]
fn refusal_sentences_never_echo_the_passphrase() {
    let secret = test_passphrase();
    let every = [
        KeyRefusal::NeedCredential,
        KeyRefusal::Wrong { retry_after: None },
        KeyRefusal::Wrong {
            retry_after: Some(Duration::from_secs(2)),
        },
        KeyRefusal::Waiting {
            retry_after: Duration::from_secs(12),
        },
        KeyRefusal::Unknown,
        KeyRefusal::NotForClinicalActs,
        KeyRefusal::Fault,
    ];
    for refusal in &every {
        let sentence = refusal.sentence();
        assert!(!sentence.is_empty(), "{refusal:?}");
        assert!(!sentence.contains(&secret), "{refusal:?}: {sentence}");
        assert!(sentence.ends_with('.'), "{refusal:?}: {sentence}");
    }
    let wait = KeyRefusal::Waiting {
        retry_after: Duration::from_secs(12),
    }
    .sentence();
    assert!(wait.contains("12 s"), "{wait}");
    // A part-second wait is rounded UP: "0 s" would invite a press that is refused again.
    let short = KeyRefusal::Waiting {
        retry_after: Duration::from_millis(1_200),
    }
    .sentence();
    assert!(short.contains("2 s"), "{short}");
    assert_eq!(
        KeyRefusal::NeedCredential.sentence(),
        "Enter your staff ID and passphrase to sign."
    );
    assert_eq!(
        KeyRefusal::Wrong { retry_after: None }.sentence(),
        "That passphrase is not right."
    );
}

/// The window's own `Debug` of a credential (a panic message, a `{:?}` in a log line) never
/// prints the passphrase.
#[test]
fn a_credential_s_debug_hides_the_passphrase() {
    let shown = format!("{:?}", credential("jsmith"));
    assert!(shown.contains("jsmith"), "{shown}");
    assert!(!shown.contains(&test_passphrase()), "{shown}");
}

/// R3: the webview sends `{staffId, passphrase}`.
#[test]
fn a_credential_reads_the_webview_s_keys() {
    let json = serde_json::json!({ "staffId": "jsmith", "passphrase": test_passphrase() });
    let c: Credential = serde_json::from_value(json).expect("the webview's shape deserializes");
    assert_eq!(c.staff_id, "jsmith");
    assert_eq!(c.passphrase.as_str(), test_passphrase());
}

/// Each vault answer becomes the refusal it means; only a release yields a key, and the key
/// id travels with it.
#[test]
fn each_vault_answer_maps_to_its_refusal() {
    use cairn_node::vault::Release;
    let wait = Duration::from_secs(4);
    assert_eq!(
        from_release(Release::Wrong {
            retry_after: Some(wait)
        })
        .unwrap_err(),
        KeyRefusal::Wrong {
            retry_after: Some(wait)
        }
    );
    assert_eq!(
        from_release(Release::Waiting { retry_after: wait }).unwrap_err(),
        KeyRefusal::Waiting { retry_after: wait }
    );
    assert_eq!(
        from_release(Release::Unknown).unwrap_err(),
        KeyRefusal::Unknown
    );
    assert_eq!(
        from_release(Release::RefusedPurpose).unwrap_err(),
        KeyRefusal::NotForClinicalActs
    );
    let key = SessionKey::for_test(Now::read()).signing_key();
    let kid = hex::encode(key.verifying_key().to_bytes());
    let (got, got_kid) = from_release(Release::Released {
        key: key.clone(),
        kid: kid.clone(),
    })
    .expect("a released key signs");
    assert_eq!(got.to_bytes(), key.to_bytes());
    assert_eq!(got_kid, kid);
}

/// The hold is capped at the idle timeout: a personal workstation keeps today's 15 minutes
/// at most, and an operator asking for more is told the cap at launch, not silently clamped.
#[test]
fn key_hold_is_capped_at_the_idle_timeout() {
    use clap::Parser;
    let parse =
        |secs: &str| crate::Cli::try_parse_from(["cairn-med-list", "--key-hold-seconds", secs]);
    let err = parse("5000")
        .err()
        .expect("a hold past the idle timeout is refused");
    assert!(err.to_string().contains("900"), "{err}");
    assert_eq!(
        parse("900")
            .expect("the cap itself is allowed")
            .key_hold_seconds,
        IDLE_TIMEOUT
    );
    let default = crate::Cli::try_parse_from(["cairn-med-list"]).expect("parses");
    assert_eq!(
        default.key_hold_seconds,
        Duration::ZERO,
        "the default is a shared terminal"
    );
}

/// Fix round 1: empty fields (after trimming spaces) are no credential at all; anything typed
/// in either field is one.
#[test]
fn an_all_empty_credential_is_no_credential() {
    let typed = |id: &str, pass: &str| Credential {
        staff_id: id.into(),
        passphrase: Zeroizing::new(pass.into()),
    };
    assert!(typed("", "").is_blank());
    assert!(typed("  ", " ").is_blank());
    // A staff ID alone is still a credential (it may name the held key's owner, below), but it
    // carries no passphrase, so it can never be sent to the vault.
    assert!(!typed("jsmith", "").is_blank());
    assert!(!typed("jsmith", " ").has_passphrase());
    assert!(!typed("", &test_passphrase()).is_blank());
    assert!(typed("", &test_passphrase()).has_passphrase());
}

fn id_only(staff_id: &str) -> Credential {
    Credential {
        staff_id: staff_id.to_string(),
        passphrase: Zeroizing::new(String::new()),
    }
}

/// Final review item 1: on a held terminal the strip keeps the holder's staff ID, so the next
/// act arrives as that ID with an empty passphrase. That is the holder carrying on, not a new
/// credential: it signs with the held key. Matched after trimming spaces, and exactly (case
/// lives in SQL alone, R7).
#[test]
fn the_holder_s_own_id_alone_uses_the_held_key() {
    let now = Now::read();
    let held = held_by("JSmith", now);
    let hold = Duration::from_secs(900);
    assert_eq!(
        decide(Some(&held), Some(&id_only(" JSmith ")), hold, now),
        HeldDecision::UseHeld
    );
    // Once the hold has run out, the ID alone signs nothing.
    assert_eq!(
        decide(Some(&held), Some(&id_only("JSmith")), hold, now.plus(hold)),
        HeldDecision::NeedCredential
    );
    // Nor on a shared terminal, where no hold is ever live.
    assert_eq!(
        decide(Some(&held), Some(&id_only("JSmith")), Duration::ZERO, now),
        HeldDecision::NeedCredential
    );
}

/// Anyone else's ID with no passphrase is a new person at the keyboard: the held key is
/// forgotten (they must not sign as the holder) and they are asked for their passphrase.
#[test]
fn another_id_alone_forgets_the_held_key() {
    let now = Now::read();
    let held = held_by("JSmith", now);
    let hold = Duration::from_secs(900);
    for other in ["apark", "jsmith"] {
        assert_eq!(
            decide(Some(&held), Some(&id_only(other)), hold, now),
            HeldDecision::Forget,
            "{other}"
        );
    }
    assert_eq!(
        decide(None, Some(&id_only("apark")), hold, now),
        HeldDecision::NeedCredential
    );
}

/// The bug this pins: an empty passphrase sent to the vault is a counted WRONG guess, so a
/// nurse carrying on at her own held terminal would walk herself into the back-off. Whatever
/// is held, for whoever, on any hold, an empty passphrase never yields a release.
#[test]
fn an_empty_passphrase_is_never_sent_to_the_vault() {
    let now = Now::read();
    let held = held_by("jsmith", now);
    for key in [None, Some(&held)] {
        for hold in [Duration::ZERO, Duration::from_secs(60), IDLE_TIMEOUT] {
            for later in [now, now.plus(IDLE_TIMEOUT)] {
                for id in ["jsmith", "apark", " jsmith "] {
                    assert_ne!(
                        decide(key, Some(&id_only(id)), hold, later),
                        HeldDecision::Release,
                        "{id:?} hold {hold:?}"
                    );
                }
                // A blank credential (both fields empty) is no credential, in `decide` itself and
                // not only by the caller's filter: never a release, and never a forget.
                for blank in ["", "  "] {
                    assert_eq!(
                        decide(key, Some(&id_only(blank)), hold, later),
                        decide(key, None, hold, later),
                        "{blank:?} hold {hold:?}"
                    );
                }
            }
        }
    }
}
