//! Unit tests for `state.rs` (the held key, the idle clock, the race rules), moved out of the
//! module so the module stays readable (#1138 session).
use super::*;

/// A state holding nothing but a session — enough to exercise the two accessors, which
/// is where the lock's real behaviour lives.
fn state_holding(session: Option<SessionKey>) -> AppState {
    let mut state = AppState::mock(None);
    // A personal workstation's longest hold: the timing tests below are about it.
    state.key_hold = IDLE_TIMEOUT;
    state.session.try_lock().expect("uncontended").key = session;
    state
}

/// A held key is what makes the whole-list gesture cost ONE act, so how long it is
/// held is a clinical decision, not a timer detail. Pinned so changing it is visible
/// in a diff.
#[test]
fn the_idle_timeout_is_fifteen_minutes() {
    assert_eq!(IDLE_TIMEOUT, Duration::from_secs(15 * 60));
}

#[test]
fn a_key_is_live_until_the_timeout_elapses() {
    let start = Now::read();
    let key = SessionKey::for_test(start);
    assert!(!key.is_expired(start, IDLE_TIMEOUT));
    assert!(!key.is_expired(start.plus(Duration::from_secs(14 * 60 + 59)), IDLE_TIMEOUT));
}

#[test]
fn a_key_expires_at_the_timeout() {
    let start = Now::read();
    let key = SessionKey::for_test(start);
    assert!(key.is_expired(start.plus(IDLE_TIMEOUT), IDLE_TIMEOUT));
    assert!(key.is_expired(start.plus(Duration::from_secs(60 * 60)), IDLE_TIMEOUT));
}

/// Idle means idle: activity resets the clock, so a clinician working continuously is
/// never asked for their passphrase mid-list.
#[test]
fn activity_pushes_the_expiry_out() {
    let start = Now::read();
    let mut key = SessionKey::for_test(start);
    key.touch(start.plus(Duration::from_secs(10 * 60)));
    assert!(!key.is_expired(start.plus(Duration::from_secs(20 * 60)), IDLE_TIMEOUT));
    assert!(key.is_expired(start.plus(Duration::from_secs(25 * 60)), IDLE_TIMEOUT));
}

/// A closed laptop is the commonest unattended workstation there is, and the monotonic
/// clock does not count the time it spent asleep. The wall clock must catch it.
#[test]
fn a_key_expires_across_a_machine_sleep_the_monotonic_clock_did_not_count() {
    let start = Now::read();
    let key = SessionKey::for_test(start);
    assert!(
        key.is_expired(start.slept(Duration::from_secs(8 * 60 * 60)), IDLE_TIMEOUT),
        "a machine asleep for eight hours must not reopen with a live signing key"
    );
}

/// An unknown idle time is not evidence of recent activity (principle 4 applied to a
/// security control), so a backwards wall-clock step re-locks rather than extends.
#[test]
fn a_backwards_wall_clock_step_re_locks_rather_than_extending() {
    let start = Now::read();
    let key = SessionKey::for_test(start);
    assert!(key.is_expired(
        start.wall_stepped_back(Duration::from_secs(60 * 60)),
        IDLE_TIMEOUT
    ));
}

/// The kid must be the key's real public identity, not a label: it is what the node
/// checks against the enrolled-human registry, and what the window shows the clinician
/// as "who is signed in".
#[test]
fn the_kid_is_the_verifying_key_in_hex() {
    let key = SessionKey::for_test(Now::read());
    let expected = hex::encode(key.signing_key().verifying_key().to_bytes());
    assert_eq!(key.kid, expected);
}

/// The reconstructed key must be the SAME key every time, or a session's signatures
/// would not verify against the kid the window reported.
#[test]
fn the_reconstructed_key_is_stable() {
    let key = SessionKey::for_test(Now::read());
    assert_eq!(
        key.signing_key().to_bytes(),
        key.signing_key().to_bytes(),
        "the held seed must rebuild one key, not a fresh one per call"
    );
}

/// THE REGRESSION TEST THIS SPLIT EXISTS FOR.
///
/// `main.js` polls `lock_state` every 10 seconds so the lock is ambient rather than
/// discovered when a signature is refused. When that poll went through the touching
/// accessor it reset the idle clock every 10 seconds with nobody at the keyboard, so
/// `is_expired` could never become true and the key stayed unlocked for the life of the
/// window — an unattended workstation from which any passer-by could sign off a whole
/// medication chart under the clinician's key. Every unit test above still passed,
/// because they all exercise `SessionKey` in isolation and never the polled path.
#[tokio::test]
async fn polling_the_lock_state_never_keeps_the_key_alive() {
    let start = Now::read();
    let state = state_holding(Some(SessionKey::for_test(start)));

    // The window's own poll interval, run right across the timeout.
    let poll = Duration::from_secs(10);
    let mut now = start;
    for _ in 0..(IDLE_TIMEOUT.as_secs() / poll.as_secs()) {
        now = now.plus(poll);
        state.key_status(now).await;
    }

    assert!(
        state.key_status(now).await.is_none(),
        "a key polled every {}s must still re-lock after {}s of no CLINICAL activity",
        poll.as_secs(),
        IDLE_TIMEOUT.as_secs()
    );
}

/// The other half of the same rule: a clinician who IS working must never be asked for
/// their passphrase mid-list, so a real act does push the expiry out.
#[tokio::test]
async fn signing_keeps_the_key_alive() {
    let start = Now::read();
    let state = state_holding(Some(SessionKey::for_test(start)));

    // A sign-off every 10 minutes for an hour — a plausible ward round.
    let mut now = start;
    for _ in 0..6 {
        now = now.plus(Duration::from_secs(10 * 60));
        assert!(
            state.signer_for_act(None, now).await.is_ok(),
            "a key in continuous clinical use must not re-lock"
        );
    }
}

/// And the boundary between them: the poll must still SHOW the re-lock, because the
/// clinician learning about it from a refused signature is the failure the ambient
/// lock display exists to prevent.
#[tokio::test]
async fn the_poll_reports_the_re_lock_it_did_not_cause() {
    let start = Now::read();
    let state = state_holding(Some(SessionKey::for_test(start)));

    assert!(state.key_status(start).await.is_some());
    assert!(state.key_status(start.plus(IDLE_TIMEOUT)).await.is_none());
    assert!(
        state.session.lock().await.key.is_none(),
        "the expired key must be WIPED by the check, not merely reported as expired"
    );
}

/// Fix round 1 (the wrong-author race): a release still in flight when Lock is pressed must
/// NOT store its key. Otherwise the next act with no credential signs as her.
#[test]
fn a_lock_between_the_wipe_and_the_store_drops_the_released_key() {
    let now = Now::read();
    let mut held = Held::default();
    let seen = held.wipe();
    held.wipe(); // Lock (`forget_held_key`) while the vault is still checking.
    assert!(!held.store_if_current(seen, SessionKey::for_test(now)));
    assert!(held.key.is_none());
}

/// #1138 item 1: the re-check wipes only the stale key it checked. A newer key a press stored
/// while the check was out (a different kid) stays.
#[test]
fn only_the_stale_key_is_forgotten() {
    let mut held = Held::default();
    let key = SessionKey::for_test(Now::read());
    let stale = key.kid.clone();
    held.key = Some(key);
    held.forget_if_holding(&format!("{stale}-not"));
    assert!(held.key.is_some(), "another kid: the held key stays");
    held.forget_if_holding(&stale);
    assert!(held.key.is_none());
}

/// Two credentialed presses overlapping: the LAST TYPED wins, never the last to finish.
#[test]
fn a_newer_release_outranks_an_older_one_that_finishes_later() {
    let now = Now::read();
    let mut held = Held::default();
    let first = held.wipe();
    let second = held.wipe();
    let mut newer = SessionKey::for_test(now);
    newer.staff_id = "apark".into();
    assert!(held.store_if_current(second, newer));
    assert!(!held.store_if_current(first, SessionKey::for_test(now)));
    assert_eq!(
        held.key.as_ref().map(|k| k.staff_id.as_str()),
        Some("apark")
    );
}

/// The step `signer_for_act` takes after the vault answers (PR #897 review: the race rule was
/// pinned only on its building block). A shared terminal (hold 0) never holds, even with
/// nothing interleaved; a Lock while the vault was checking drops the key; otherwise it is held.
#[test]
fn a_released_key_is_kept_only_on_a_hold_terminal_and_only_if_nothing_wiped() {
    let now = Now::read();
    let key = || SessionKey::for_test(now).signing_key();
    let mut held = Held::default();
    let seen = held.wipe();
    assert!(!held.keep_for_next_act(seen, Duration::ZERO, key(), "jsmith", now));
    assert!(held.key.is_none(), "hold 0 holds nothing");

    let seen = held.wipe();
    held.wipe(); // Lock while the vault is checking.
    assert!(!held.keep_for_next_act(seen, IDLE_TIMEOUT, key(), "jsmith", now));
    assert!(held.key.is_none(), "a Lock in flight wins");

    let seen = held.wipe();
    assert!(held.keep_for_next_act(seen, IDLE_TIMEOUT, key(), " jsmith ", now));
    assert_eq!(
        held.key.as_ref().map(|k| k.staff_id.as_str()),
        Some("jsmith")
    );
}

/// ...and `signer_for_act` takes it with the generation its OWN wipe returned, not one read
/// later (which would let a Lock in flight be undone).
#[test]
fn signer_for_act_keeps_the_key_with_the_generation_of_its_own_wipe() {
    let flat: String = crate::source_guard::rust_fn(include_str!("state.rs"), "signer_for_act")
        .split_whitespace()
        .collect();
    // `rust_fn` runs to the end of the impl for a method; cut at the next method.
    let body = flat
        .split("pubasyncfnforget_held_key")
        .next()
        .unwrap()
        .to_string();
    let wipe = body.find("letseen=held.wipe();").expect(&body);
    let keep = body.find(".keep_for_next_act(seen,").expect(&body);
    assert!(wipe < keep, "{body}");
    // #1128: the claim is asked before any key is kept, and the key is kept under the name
    // it answers, never the typed one (a DB-gated test pins the behaviour; CI skips it, #905).
    let gate = body.find("self.hold_name_for_claim(&kid,").expect(&body);
    assert!(gate < keep, "{body}");
    assert!(
        !body[keep..].contains("&credential.staff_id"),
        "the key must be held under the claim's name: {body}"
    );
    assert_eq!(body.matches("store_if_current(").count(), 0, "{body}");
}

/// With nothing interleaved, the released key is held.
#[test]
fn an_uninterrupted_release_is_held() {
    let mut held = Held::default();
    let seen = held.wipe();
    assert!(held.store_if_current(seen, SessionKey::for_test(Now::read())));
    assert!(held.key.is_some());
}

/// `held_by` shows the staff ID without the spaces a keyboard left around it. Spaces only:
/// case and Unicode normalisation live in SQL alone (`cairn_vault_staff_id`, R7).
#[test]
fn the_held_staff_id_is_trimmed_of_spaces_only() {
    let key = SessionKey::for_test(Now::read());
    let held = SessionKey::new(key.signing_key(), " JSmith ".into(), Now::read());
    assert_eq!(held.staff_id, "JSmith");
}

/// An all-empty credential is no credential: a webview sending empty fields never wipes a
/// hold, and the held key still serves.
#[tokio::test]
async fn an_all_empty_credential_keeps_the_hold() {
    let now = Now::read();
    let state = state_holding(Some(SessionKey::for_test(now)));
    let blank = Credential {
        staff_id: "  ".into(),
        passphrase: Zeroizing::new(" ".into()),
    };
    assert!(state.signer_for_act(Some(blank), now).await.is_ok());
    assert!(state.session.lock().await.key.is_some());
}

/// Review focus 2, at the seam: a typed credential wipes the held key BEFORE the vault
/// answers, so a mistyped passphrase (here, fixture mode's `Fault`) never leaves the last
/// person's key to sign the next act that carries none.
#[tokio::test]
async fn a_typed_credential_wipes_the_held_key_whatever_the_answer() {
    let now = Now::read();
    let state = state_holding(Some(SessionKey::for_test(now)));
    let credential = Credential {
        staff_id: "apark".into(),
        passphrase: Zeroizing::new(["not", "hers"].join("-")),
    };
    assert_eq!(
        state.signer_for_act(Some(credential), now).await.err(),
        Some(KeyRefusal::Fault)
    );
    assert!(state.session.lock().await.key.is_none());
    assert_eq!(
        state.signer_for_act(None, now).await.err(),
        Some(KeyRefusal::NeedCredential)
    );
}

/// Final review item 1, at the seam: the holder's own staff ID with an empty passphrase signs
/// with the held key; anyone else's wipes it and asks for a passphrase. Neither asks the vault
/// (in fixture mode a vault call would be `Fault`), so an empty passphrase is never a counted
/// wrong guess.
#[tokio::test]
async fn a_staff_id_alone_never_asks_the_vault() {
    let id_only = |id: &str| Credential {
        staff_id: id.into(),
        passphrase: Zeroizing::new(String::new()),
    };
    let now = Now::read();
    let state = state_holding(Some(SessionKey::for_test(now)));
    assert!(state
        .signer_for_act(Some(id_only(" test-signer ")), now)
        .await
        .is_ok());
    assert_eq!(
        state
            .signer_for_act(Some(id_only("apark")), now)
            .await
            .err(),
        Some(KeyRefusal::NeedCredential)
    );
    assert!(
        state.session.lock().await.key.is_none(),
        "another person's ID must forget the held key"
    );
    assert_eq!(
        state
            .signer_for_act(Some(id_only("apark")), now)
            .await
            .err(),
        Some(KeyRefusal::NeedCredential),
        "with nothing held, an ID alone is not a credential either"
    );
}

/// The explicit Lock wipes the held key.
#[tokio::test]
async fn forgetting_the_held_key_wipes_it() {
    let state = state_holding(Some(SessionKey::for_test(Now::read())));
    state.forget_held_key().await;
    assert!(state.key_status(Now::read()).await.is_none());
}

/// A locked window has no key to take, and asking must not create one.
#[tokio::test]
async fn an_empty_session_yields_nothing_from_either_accessor() {
    let state = state_holding(None);
    let now = Now::read();
    assert!(state.key_status(now).await.is_none());
    assert_eq!(
        state.signer_for_act(None, now).await.err(),
        Some(KeyRefusal::NeedCredential)
    );
}
