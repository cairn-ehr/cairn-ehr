//! "Same person as…" — compare two records side by side and link them (repair path R2b-1,
//! ADR-0076 decisions 3–5; design "R2b — the window's gesture").
//!
//! Paper counterpart: fetch the other folder, lay the front sheets side by side, clip. Three
//! acts — find (the in-chart search, which reuses `browse`: it adds its results to
//! `AppState::shown` and never switches the open chart), Compare, Link. The Link click IS the
//! signature under the unlocked key (ADR-0053); there is no confirmation dialog (principle 3).
//!
//! Every command here applies the chart-command rules IN THIS ORDER, and each test pins one:
//! the chart on screen (`displayed_patient`), the displayed set (`check_displayed_set`), the
//! other chart was shown by a list (`shown`), it is not already in the record, and — for the
//! link — the OTHER record is still the set the clinician compared (decision 3 widened to the
//! right-hand side). Only then fixture mode, then the key.
//!
//! Its sibling gesture, "Not the same person…" (R2b-2 — undo ONE link of a combined record),
//! lives in `unlink.rs` with its own rule order and its own panel; `record_links.rs` reads the
//! "How these charts are linked" list both gestures sit beside, and `unlink_view.rs` words it.
pub mod record_links;
pub mod search;
pub mod unlink;
pub mod unlink_view;
pub mod view;

use crate::chart_set::{check_displayed_set, CHANGED};
use crate::commands::read_chart_of;
use crate::funnel::view::ErrorView;
use crate::state::{AppState, Now};
use cairn_medication_view::ChartSet;
use uuid::Uuid;
use view::{
    comparison_view, fixture_facts, key_locked, link_error_view, link_report, refused,
    ComparisonParts, ComparisonView, LinkReportView, ALREADY_IN_RECORD, NOT_ON_SCREEN,
    OTHER_CHANGED, THIS_CHANGED,
};

/// A read's error as the text a comparison part carries (the operator chain, legible).
/// A plain generic fn, not a closure: it is used at two different `T`s.
pub(crate) fn as_text<T>(r: anyhow::Result<T>) -> Result<T, String> {
    r.map_err(|e| cairn_node::db_diagnosis::operator_chain(&e))
}

/// The record `patient` belongs to: its link component live, itself alone in fixture mode
/// (fixture charts are never linked). A failure here is a READ that failed — nothing was
/// judged — so it is worded as one, retryable, never as a link outcome.
pub(crate) async fn chart_set_of(state: &AppState, patient: Uuid) -> Result<ChartSet, ErrorView> {
    let Some(db) = state.db.as_ref() else {
        return Ok(ChartSet::single(patient));
    };
    let db = db.lock().await;
    cairn_node::patient::person::person_charts(&*db, patient)
        .await
        .map_err(|e| ErrorView {
            text: format!(
                "Could not read which charts this record combines — nothing was done: {}",
                cairn_node::db_diagnosis::operator_chain(&e)
            ),
            retry: crate::funnel::view::Retry::Now,
        })
}

/// Which command is asking [`resolve_pair`] — it decides only how a CHANGED left-hand set is
/// worded (see [`resolve_pair`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    Compare,
    Link,
}

/// The four screen checks both commands share. Returns the opened chart, its displayed set,
/// and the other chart (with the name the list showed, for fixture mode).
///
/// For Compare, `charts` is the medication list's displayed set, and a changed set keeps the
/// list's own wording ("reload the chart"). For Link, `charts` is the set the COMPARISON was
/// built over (`ComparisonView::left_charts`, sent back by the webview), so a changed set means
/// the comparison is stale and is worded [`THIS_CHANGED`] ("reload the chart and compare
/// again") — final review I1.
/// An unreadable set is a window fault, not a change, and keeps its own wording either way.
async fn resolve_pair(
    state: &AppState,
    act: Act,
    patient_id: &str,
    charts: &[String],
    other_id: &str,
) -> Result<(Uuid, ChartSet, Uuid, String), ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    let left = check_displayed_set(&chart_set_of(state, patient).await?, charts).map_err(|e| {
        if act == Act::Link && e == CHANGED {
            refused(THIS_CHANGED)
        } else {
            refused(e)
        }
    })?;
    let other: Uuid = other_id.parse().map_err(|_| refused(NOT_ON_SCREEN))?;
    let shown_name = state
        .shown
        .lock()
        .await
        .get(&other)
        .map(|c| c.display_name.clone())
        .ok_or_else(|| refused(NOT_ON_SCREEN))?;
    if left.contains(&other) {
        return Err(refused(ALREADY_IN_RECORD));
    }
    Ok((patient, left, other, shown_name))
}

/// "Compare" — read both records and build the panel. The view carries BOTH sets it was read
/// over (`left_charts`, `other_charts`); the webview sends exactly those back with Link.
pub async fn compare_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    other_id: &str,
) -> Result<ComparisonView, ErrorView> {
    let (patient, left, other, shown_name) =
        resolve_pair(state, Act::Compare, patient_id, &charts, other_id).await?;
    let right = chart_set_of(state, other).await?;
    // The other record's list: the SAME custody-applied read opening it would give (§5.9).
    let meds = read_chart_of(state, other)
        .await
        .map(|list| cairn_gui_tab_medications::view::build_view(&list));
    let parts = match state.db.as_ref() {
        None => {
            let header = state
                .chart
                .lock()
                .await
                .as_ref()
                .map(|c| c.header.name.clone());
            ComparisonParts {
                left: Ok(vec![fixture_facts(
                    patient,
                    &header.unwrap_or_default(),
                    "confirmed",
                )]),
                right: Ok(vec![fixture_facts(other, &shown_name, "confirmed")]),
                findings: Ok(vec![]),
                other_medications: meds,
            }
        }
        Some(db) => {
            let db = db.lock().await;
            ComparisonParts {
                left: as_text(cairn_node::patient::compare::chart_facts(&*db, &left).await),
                right: as_text(cairn_node::patient::compare::chart_facts(&*db, &right).await),
                findings: as_text(
                    cairn_node::patient::compare::cross_vetoes(&*db, &left, &right).await,
                ),
                other_medications: meds,
            }
        }
    };
    Ok(comparison_view(parts, &left, &right))
}

/// "Link — same person" — the attested judgement, then what it did.
///
/// `charts` and `other_charts` are the two sets the comparison showed (not the medication
/// list's): each must still be the record's current set, or the judgement is refused with
/// [`THIS_CHANGED`] / [`OTHER_CHANGED`] — a link is never signed over a set nobody compared.
/// Their union is handed to [`link_report`] so the outcome names any chart the record now
/// combines that the comparison never showed.
pub async fn link_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    other_id: &str,
    other_charts: Vec<String>,
) -> Result<LinkReportView, ErrorView> {
    let (patient, left, other, _) =
        resolve_pair(state, Act::Link, patient_id, &charts, other_id).await?;
    // Only a CHANGED set is "the other record changed"; an unreadable one is a window fault
    // and keeps its own wording — the right-hand twin of `resolve_pair`'s rule (PR #707 review).
    let right =
        check_displayed_set(&chart_set_of(state, other).await?, &other_charts).map_err(|e| {
            if e == CHANGED {
                refused(OTHER_CHANGED)
            } else {
                refused(e)
            }
        })?;
    // Both sets non-empty, so the union always builds; `single` is an unreachable fallback.
    let compared = ChartSet::new(left.members().iter().chain(right.members()).copied())
        .unwrap_or_else(|| ChartSet::single(patient));
    if state.is_mock() {
        return Err(refused(
            "fixture mode: this window is showing mock data and cannot write",
        ));
    }
    // Linking is a clinical act, so taking the key counts as activity (`live_key`). A locked
    // key is not a verdict: `key_locked` keeps the Link button for after the unlock.
    let (human_sk, human_kid) = state.live_key(Now::read()).await.ok_or_else(key_locked)?;
    let mut db = state
        .db
        .as_ref()
        .ok_or_else(|| refused("no database connection"))?
        .lock()
        .await;
    let reviewer = cairn_node::chart_link::Reviewer {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    // No gesture-timing row: db/044's `gesture_kind` CHECK admits only signoff/cease; the
    // runbook's stopwatch measures this gesture (design "R2b").
    let outcome =
        cairn_node::chart_link::link_charts(&mut db, patient, other, &reviewer, &state.node_origin)
            .await
            .map_err(|e| link_error_view(&e))?;
    Ok(link_report(outcome.effect, &outcome.charts, &compared))
}

// ---- Tauri forwarders (camelCase JS keys → snake_case parameters). ----

#[tauri::command]
pub async fn compare_records(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    other_id: String,
) -> Result<ComparisonView, ErrorView> {
    compare_impl(&state, &patient_id, charts, &other_id).await
}

#[tauri::command]
pub async fn link_records(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    other_id: String,
    other_charts: Vec<String>,
) -> Result<LinkReportView, ErrorView> {
    link_impl(&state, &patient_id, charts, &other_id, other_charts).await
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
