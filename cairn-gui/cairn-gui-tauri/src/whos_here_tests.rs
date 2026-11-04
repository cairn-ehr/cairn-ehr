//! Tests for `whos_here.rs`: the pure claim/recent/bookmark rules, the words a claimed name reads
//! as, the launch flag's bounds, and — against `AppState::mock`, no Tauri runtime — that a pick
//! and the re-lock forget the held key and that a pick's offer is openable.
use super::*;
use crate::funnel::commands::{close_chart_impl, open_chart_impl};
use crate::funnel::view::tests::sample_candidate;
use crate::state::{Now, SessionKey, IDLE_TIMEOUT};

fn named(staff_id: &str, display_name: &str) -> Claim {
    Claim::Named {
        staff_id: staff_id.into(),
        display_name: display_name.into(),
    }
}

fn here() -> Here {
    Here::new(DEFAULT_IDLE_LOCK)
}

#[test]
fn pick_named_then_forget_keeps_bookmarks() {
    let mut h = here();
    h.pick(named("nurse-a", "Nurse A"));
    h.remember_chart(&sample_candidate());
    h.forget_claim();
    assert_eq!(h.claim(), &Claim::NotAsked);
    assert_eq!(h.offer(), None, "nobody is named, so nothing is offered");
    assert_eq!(h.recent(), [("nurse-a".into(), "Nurse A".into())]);
    h.pick(named("nurse-a", "Nurse A"));
    assert_eq!(
        h.offer(),
        Some(&sample_candidate()),
        "her bookmark survived"
    );
    // Another person's pick does not see it.
    h.pick(named("dr-b", "Dr B"));
    assert_eq!(h.offer(), None);
}

#[test]
fn remember_chart_ignored_unless_named() {
    let mut h = here();
    h.remember_chart(&sample_candidate());
    h.pick(Claim::Unknown);
    h.remember_chart(&sample_candidate());
    assert_eq!(h.offer(), None);
    h.pick(named("nurse-a", "Nurse A"));
    assert_eq!(h.offer(), None, "neither earlier chart was hers");
    assert!(h.recent().iter().all(|(id, _)| id == "nurse-a"));
}

#[test]
fn recent_is_capped_at_8_and_deduped() {
    let mut h = here();
    for n in 0..10 {
        h.pick(named(&format!("s{n}"), &format!("Staff {n}")));
    }
    h.pick(named("s5", "Staff 5"));
    let ids: Vec<&str> = h.recent().iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["s5", "s9", "s8", "s7", "s6", "s4", "s3", "s2"]);
    h.pick(Claim::Unknown);
    assert_eq!(h.recent().len(), 8, "Don't know is nobody to remember");
}

#[test]
fn claimed_name_words() {
    let a = claimed_name(Some("Nurse A"));
    assert_eq!(a.shown, "?Nurse A");
    assert_eq!(a.spoken, "unverified Nurse A");
    let none = claimed_name(None);
    assert_eq!(none.shown, "writer not known");
    assert_eq!(none.spoken, "writer not known");
}

#[test]
fn the_bar_says_who_is_writing() {
    assert_eq!(
        claim_view(&named("nurse-a", "Nurse A")).words,
        "Writing as Nurse A"
    );
    let unknown = claim_view(&Claim::Unknown);
    assert_eq!(unknown.kind, "unknown");
    assert_eq!(unknown.staff_id, None);
    assert_eq!(claim_view(&Claim::NotAsked).kind, "not-asked");
}

#[test]
fn the_directory_lists_recent_people_first() {
    let entry = |id: &str| StaffEntry {
        staff_id: id.into(),
        display_name: id.to_uppercase(),
        role: "nurse".into(),
    };
    let recent = [
        ("c".to_string(), "C".to_string()),
        ("gone".to_string(), "Gone".to_string()),
        ("a".to_string(), "A".to_string()),
    ];
    let view = directory_view(vec![entry("a"), entry("b"), entry("c")], &recent);
    let ids: Vec<(&str, bool)> = view
        .iter()
        .map(|e| (e.staff_id.as_str(), e.recent))
        .collect();
    // A recent person no longer in the directory is not offered.
    assert_eq!(ids, [("c", true), ("a", true), ("b", false)]);
}

#[test]
fn idle_lock_minutes_bounds() {
    assert!(parse_idle_lock_minutes("0").is_err());
    assert!(parse_idle_lock_minutes("241").is_err());
    assert!(parse_idle_lock_minutes("soon").is_err());
    assert_eq!(
        parse_idle_lock_minutes("15"),
        Ok(Duration::from_secs(15 * 60))
    );
    assert!(parse_idle_lock_minutes("1").is_ok() && parse_idle_lock_minutes("240").is_ok());
    assert_eq!(DEFAULT_IDLE_LOCK, Duration::from_secs(15 * 60));
}

/// A mock window holding a key, as a key-hold terminal would after an act.
fn holding_a_key() -> AppState {
    let mut state = AppState::mock(None);
    state.key_hold = IDLE_TIMEOUT;
    state.session.try_lock().expect("uncontended").key = Some(SessionKey::for_test(Now::read()));
    state
}

/// Read the slot directly: `key_status` would expire the key itself on a zero hold.
async fn key_is_held(state: &AppState) -> bool {
    state.session.lock().await.key.is_some()
}

/// Design §3.1 review finding: Dr B, switched in, must never sign with Dr A's held key.
#[tokio::test]
async fn pick_forgets_the_held_key() {
    let state = holding_a_key();
    let picked = whos_here_pick_impl(&state, Some("dr-b")).await.unwrap();
    assert_eq!(picked.claim.staff_id.as_deref(), Some("dr-b"));
    assert!(!key_is_held(&state).await);
    let state = holding_a_key();
    whos_here_pick_impl(&state, None).await.unwrap();
    assert!(!key_is_held(&state).await, "Don't know forgets it too");
}

/// A pick of someone not enrolled is refused — and the key and the old claim are forgotten anyway:
/// someone new is at the keyboard.
#[tokio::test]
async fn a_pick_of_someone_not_enrolled_names_nobody() {
    let state = holding_a_key();
    whos_here_pick_impl(&state, Some("nurse-a")).await.unwrap();
    let err = whos_here_pick_impl(&state, Some("mallory"))
        .await
        .unwrap_err();
    assert!(err.text.contains("not an enrolled member"), "{}", err.text);
    assert_eq!(state.whos_here.lock().await.claim(), &Claim::NotAsked);
    assert!(!key_is_held(&state).await);
}

#[tokio::test]
async fn relock_forgets_claim_and_key() {
    let state = holding_a_key();
    whos_here_pick_impl(&state, Some("nurse-a")).await.unwrap();
    // She signs, and her key is held again.
    state.session.lock().await.key = Some(SessionKey::for_test(Now::read()));
    let lock = relock_impl(&state).await;
    assert_eq!(lock.whos_here.kind, "not-asked");
    assert_eq!(lock.idle_lock_secs, 15 * 60);
    assert!(!key_is_held(&state).await);
    assert_eq!(
        state.whos_here.lock().await.recent().len(),
        1,
        "recent kept"
    );
}

/// The offer IS a list on screen, so `open_chart`'s rule (only a chart some list showed) holds.
#[tokio::test]
async fn an_offered_chart_becomes_openable() {
    let state = AppState::mock(None);
    let c = sample_candidate();
    let id = c.patient_id.to_string();
    whos_here_pick_impl(&state, Some("nurse-a")).await.unwrap();
    state.shown.lock().await.insert(c.patient_id, c.clone());
    open_chart_impl(&state, &id).await.unwrap();
    close_chart_impl(&state).await;
    assert!(
        open_chart_impl(&state, &id).await.is_err(),
        "no list shows it"
    );

    let nobody = whos_here_pick_impl(&state, None).await.unwrap();
    assert!(nobody.offer.is_none());
    let back = whos_here_pick_impl(&state, Some("nurse-a")).await.unwrap();
    assert_eq!(back.offer.map(|h| h.patient_id), Some(id.clone()));
    assert_eq!(open_chart_impl(&state, &id).await.unwrap().patient_id, id);
}

/// Fixture mode has a directory, so the operator accessibility pass can walk the picker.
#[tokio::test]
async fn fixture_mode_lists_its_staff() {
    let state = AppState::mock(None);
    whos_here_pick_impl(&state, Some("dr-b")).await.unwrap();
    let dir = whos_here_directory_impl(&state).await.unwrap();
    assert_eq!(dir[0].staff_id, "dr-b");
    assert!(dir[0].recent);
    assert!(dir.iter().any(|e| e.staff_id == "nurse-a" && !e.recent));
}
