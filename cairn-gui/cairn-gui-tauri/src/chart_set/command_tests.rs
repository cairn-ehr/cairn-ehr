//! Each chart command calls the set check (ADR-0076 decision 3); see the declaration
//! in `chart_set/mod.rs` for why these sit beside the check rather than in `commands.rs`.
use crate::chart_set::CHANGED;
use crate::commands::{cease_impl, med_list_impl, sign_off_impl};
use crate::funnel::view::{Recorded, Retry};
use crate::state::AppState;
use uuid::Uuid;

fn open_on_the_fixture_chart() -> (AppState, String) {
    let fixture: Uuid = cairn_gui_data::mock::fixtures::FIXTURE_UUID
        .parse()
        .unwrap();
    (AppState::mock(Some(fixture)), fixture.to_string())
}

/// The set check runs BEFORE fixture mode's own refusal, so a set that changed is reported
/// as exactly that even here — which is also what proves the order.
#[tokio::test]
async fn a_changed_set_refuses_sign_off_as_stale() {
    let (state, on_screen) = open_on_the_fixture_chart();
    let err = sign_off_impl(
        &state,
        &on_screen,
        vec![on_screen.clone(), Uuid::from_u128(99).to_string()],
    )
    .await
    .unwrap_err();
    assert_eq!(err.retry, Retry::AfterReread(Recorded::No));
    assert_eq!(err.text, CHANGED);
}

/// Cease is a write on the same list, so it carries the same rule — ahead of the fixture
/// refusal and ahead of the reason check.
#[tokio::test]
async fn a_changed_set_refuses_cease_as_stale() {
    let (state, on_screen) = open_on_the_fixture_chart();
    let err = cease_impl(
        &state,
        &on_screen,
        vec![on_screen.clone(), Uuid::from_u128(99).to_string()],
        &Uuid::from_u128(10).to_string(),
        "",
    )
    .await
    .unwrap_err();
    assert_eq!(err.retry, Retry::AfterReread(Recorded::No));
    assert_eq!(err.text, CHANGED);
}

/// The read hands the webview the set it must send back, and — the fixture chart being
/// linked to nothing (fixture charts never are) — no member lines.
#[tokio::test]
async fn the_medication_list_names_its_chart_set() {
    let (state, on_screen) = open_on_the_fixture_chart();
    let pane = med_list_impl(&state, &on_screen, None).await.unwrap();
    assert_eq!(pane.list.charts, vec![on_screen]);
    assert!(pane.members.is_empty());
}
