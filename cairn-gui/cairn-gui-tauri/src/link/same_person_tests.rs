//! Tests for `link/same_person.rs` (R2b-1's Compare and Link): each pins one of the chart-command
//! rules, in the order the commands apply them. Moved out of `link/mod.rs` with the code (#781).
use super::*;
use crate::link::view;
use cairn_patient_search::{Candidate, TrustState};

fn fixture() -> Uuid {
    cairn_gui_data::mock::fixtures::FIXTURE_UUID
        .parse()
        .unwrap()
}

/// A window open on the fixture chart, with `others` shown by a list on screen.
async fn window_showing(others: &[Uuid]) -> AppState {
    let state = AppState::mock(Some(fixture()));
    let mut shown = state.shown.lock().await;
    for id in others {
        shown.insert(
            *id,
            Candidate {
                patient_id: *id,
                display_name: "Other Person".into(),
                age: None,
                trust: TrustState::Confirmed,
                last_activity: None,
                locale: None,
                photo_ref: None,
            },
        );
    }
    drop(shown);
    state
}

fn on_screen() -> (String, Vec<String>) {
    (fixture().to_string(), vec![fixture().to_string()])
}

#[tokio::test]
async fn compare_is_bound_to_the_chart_on_screen() {
    let other = Uuid::from_u128(2);
    let state = window_showing(&[other]).await;
    let err = compare_impl(
        &state,
        &Uuid::from_u128(9).to_string(),
        vec![],
        &other.to_string(),
    )
    .await
    .unwrap_err();
    assert!(
        err.text.contains("not the chart"),
        "compare should refuse a patient that is not the chart on screen"
    );
}

#[tokio::test]
async fn compare_refuses_a_changed_set() {
    let other = Uuid::from_u128(2);
    let state = window_showing(&[other]).await;
    let (p, _) = on_screen();
    let err = compare_impl(
        &state,
        &p,
        vec![p.clone(), Uuid::from_u128(7).to_string()],
        &other.to_string(),
    )
    .await
    .unwrap_err();
    assert!(
        err.text.contains("linked charts changed"),
        "compare should refuse a changed set"
    );
}

#[tokio::test]
async fn compare_refuses_a_chart_no_list_showed() {
    let state = window_showing(&[]).await;
    let (p, charts) = on_screen();
    let err = compare_impl(&state, &p, charts, &Uuid::from_u128(2).to_string())
        .await
        .unwrap_err();
    assert_eq!(err.text, view::NOT_ON_SCREEN);
}

/// Review Focus 1: the in-chart search returns the opened chart itself.
#[tokio::test]
async fn compare_refuses_a_chart_already_in_the_record() {
    let state = window_showing(&[fixture()]).await;
    let (p, charts) = on_screen();
    let err = compare_impl(&state, &p, charts, &p).await.unwrap_err();
    assert_eq!(err.text, view::ALREADY_IN_RECORD);
}

#[tokio::test]
async fn a_fixture_comparison_has_a_column_per_chart_and_can_be_walked() {
    let other = Uuid::from_u128(2);
    let state = window_showing(&[other]).await;
    let (p, charts) = on_screen();
    let v = compare_impl(&state, &p, charts, &other.to_string())
        .await
        .unwrap();
    assert_eq!(v.columns.len(), 2);
    assert_eq!(v.left_count, 1);
    assert_eq!(v.other_charts, vec![other.to_string()]);
    assert_eq!(
        v.left_charts,
        vec![p.clone()],
        "the compared left set, sent back with the link"
    );
    assert!(
        v.can_link,
        "fixture mode reads everything it has; it refuses only the WRITE"
    );
}

/// Review Focus 2: the other record changed between Compare and Link.
#[tokio::test]
async fn link_refuses_when_the_other_record_changed() {
    let other = Uuid::from_u128(2);
    let state = window_showing(&[other]).await;
    let (p, charts) = on_screen();
    let err = link_impl(
        &state,
        &p,
        charts,
        &other.to_string(),
        vec![other.to_string(), Uuid::from_u128(4).to_string()],
    )
    .await
    .unwrap_err();
    assert_eq!(err.text, view::OTHER_CHANGED);
    // Its remedy ("compare again") re-reads the OTHER record, which the window does not do.
    assert_eq!(err.retry, crate::funnel::view::Retry::Never);
}

/// Every check runs BEFORE fixture mode's own refusal, so the fixture refusal is proof
/// that they all passed.
#[tokio::test]
async fn link_in_fixture_mode_passes_every_check_then_refuses_to_write() {
    let other = Uuid::from_u128(2);
    let state = window_showing(&[other]).await;
    let (p, charts) = on_screen();
    let err = link_impl(
        &state,
        &p,
        charts,
        &other.to_string(),
        vec![other.to_string()],
    )
    .await
    .unwrap_err();
    assert!(
        err.text.contains("fixture mode"),
        "link should refuse to write in fixture mode"
    );
}

/// Final review I1: the Link sends the LEFT set the clinician compared, not whatever the
/// window shows by the time of the click. If this record grew in between (a peer's link
/// arriving, re-read by a sign-off's refresh), the judgement would be signed over a set
/// nobody compared — so it is refused, worded like its right-hand twin.
#[tokio::test]
async fn link_refuses_when_this_record_changed() {
    let other = Uuid::from_u128(2);
    let state = window_showing(&[other]).await;
    let (p, _) = on_screen();
    let err = link_impl(
        &state,
        &p,
        vec![p.clone(), Uuid::from_u128(7).to_string()],
        &other.to_string(),
        vec![other.to_string()],
    )
    .await
    .unwrap_err();
    assert_eq!(err.text, view::THIS_CHANGED);
    assert_eq!(err.retry, crate::funnel::view::Retry::AfterReread);
}

/// Link refuses an unreadable displayed left set with the "could not tell which charts are
/// on screen — reopen" wording, never the "this record changed" wording.
#[tokio::test]
async fn an_unreadable_left_set_is_not_worded_as_a_change() {
    let other = Uuid::from_u128(2);
    let state = window_showing(&[other]).await;
    let (p, _) = on_screen();
    let err = link_impl(
        &state,
        &p,
        vec!["not-a-uuid".into()],
        &other.to_string(),
        vec![other.to_string()],
    )
    .await
    .unwrap_err();
    assert_ne!(err.text, view::THIS_CHANGED);
    assert!(
        err.text.contains("could not tell"),
        "unreadable set should produce the read-failure message"
    );
}

/// PR #707 review: the right-hand twin of the test above. An unreadable `other_charts` is
/// a window fault, not a change to the other record, so it keeps the "could not tell"
/// wording rather than "the other record changed".
#[tokio::test]
async fn an_unreadable_other_set_is_not_worded_as_a_change() {
    let other = Uuid::from_u128(2);
    let state = window_showing(&[other]).await;
    let (p, charts) = on_screen();
    let err = link_impl(
        &state,
        &p,
        charts,
        &other.to_string(),
        vec!["not-a-uuid".into()],
    )
    .await
    .unwrap_err();
    assert_ne!(err.text, view::OTHER_CHANGED);
    assert!(
        err.text.contains("could not tell"),
        "unreadable set should produce the read-failure message"
    );
}

#[tokio::test]
async fn link_is_bound_to_the_chart_on_screen() {
    let other = Uuid::from_u128(2);
    let state = window_showing(&[other]).await;
    let err = link_impl(
        &state,
        &Uuid::from_u128(9).to_string(),
        vec![],
        &other.to_string(),
        vec![other.to_string()],
    )
    .await
    .unwrap_err();
    assert!(
        err.text.contains("not the chart"),
        "link should refuse a patient that is not the chart on screen"
    );
}

#[tokio::test]
async fn link_refuses_a_chart_already_in_the_record() {
    let state = window_showing(&[fixture()]).await;
    let (p, charts) = on_screen();
    let err = link_impl(&state, &p, charts.clone(), &p, charts)
        .await
        .unwrap_err();
    assert_eq!(err.text, view::ALREADY_IN_RECORD);
}

#[tokio::test]
async fn link_refuses_a_chart_no_list_showed() {
    let state = window_showing(&[]).await;
    let (p, charts) = on_screen();
    let other = Uuid::from_u128(2).to_string();
    let err = link_impl(&state, &p, charts, &other, vec![other.clone()])
        .await
        .unwrap_err();
    assert_eq!(err.text, view::NOT_ON_SCREEN);
}
