//! "Same person as…" — compare two records side by side and link them (repair path R2b-1,
//! ADR-0076 decisions 3–5; design "R2b — the window's gesture").
//!
//! Paper counterpart: fetch the other folder, lay the front sheets side by side, clip. Three
//! acts — find (the in-chart search, which reuses `browse`: it adds its results to
//! `AppState::shown` and never switches the open chart), Compare, Link. The Link click IS the
//! signature under the signing clinician's key (ADR-0053); there is no confirmation dialog (principle 3).
//!
//! Every command here applies the chart-command rules IN THIS ORDER, and each test pins one:
//! the chart on screen (`displayed_patient`), the displayed set (`check_displayed_set`), the
//! other chart is not already in the record, it was shown (a list, or R5a's banner), and — for the
//! link — the OTHER record is still the set the clinician compared (decision 3 widened to the
//! right-hand side). Only then fixture mode, then the key.
//!
//! Split out of `link/mod.rs` (#781), which keeps the module map and the two helpers the
//! gestures share (`chart_set_of`, the live chart-set read; `as_text`, a read's error as text).
use super::judgement_view::{
    key_locked, link_error_view, link_report, refused, stale, LinkReportView,
};
use super::view::{
    comparison_view, fixture_facts, ComparisonParts, ComparisonView, ALREADY_IN_RECORD,
    NOT_ON_SCREEN, OTHER_CHANGED, THIS_CHANGED,
};
use super::{as_text, chart_set_of};
use crate::chart_set::{check_displayed_set, ChartSetError};
use crate::commands::read_chart_of;
use crate::funnel::view::ErrorView;
use crate::signer::Credential;
use crate::state::{AppState, Now};
use cairn_medication_view::ChartSet;
use uuid::Uuid;

/// Which command is asking [`resolve_pair`] — it decides only how a CHANGED left-hand set is
/// worded (see [`resolve_pair`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    Compare,
    Link,
}

/// The four screen checks both commands share. Returns the opened chart, its displayed set,
/// and the other chart (with the name the list showed — `None` when an open proposal admitted
/// it instead — for fixture mode).
///
/// For Compare, `charts` is the medication list's displayed set, and a changed set keeps the
/// list's own wording (`CHANGED`). For Link, `charts` is the set the COMPARISON was
/// built over (`ComparisonView::left_charts`, sent back by the webview), so a changed set means
/// the comparison is stale and is worded [`THIS_CHANGED`] — final review I1. Both are
/// `Retry::AfterReread`.
/// An unreadable set is a window fault, not a change, and keeps its own wording either way.
async fn resolve_pair(
    state: &AppState,
    act: Act,
    patient_id: &str,
    charts: &[String],
    other_id: &str,
) -> Result<(Uuid, ChartSet, Uuid, Option<String>), ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    let left = check_displayed_set(&chart_set_of(state, patient).await?, charts).map_err(|e| {
        if act == Act::Link && e == ChartSetError::Changed {
            stale(THIS_CHANGED)
        } else {
            e.into_view()
        }
    })?;
    let other: Uuid = other_id.parse().map_err(|_| refused(NOT_ON_SCREEN))?;
    if left.contains(&other) {
        return Err(refused(ALREADY_IN_RECORD)); // before admission: no proposal read for it
    }
    // A list on screen, or (R5a) an open proposal the banner showed.
    let shown_name = crate::duplicates::admit_other(state, &left, other).await?;
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
        .map(|list| cairn_gui_tab_medications::view::build_view(&list))
        .map_err(|e| format!("{e:#}"));
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
                // Fixture mode admits only by a list (it has no proposals), so a name is there.
                right: Ok(vec![fixture_facts(
                    other,
                    shown_name.as_deref().unwrap_or_default(),
                    "confirmed",
                )]),
                findings: Ok(vec![]),
                other_medications: meds,
            }
        }
        Some(db) => {
            let db = db.lock().await?;
            ComparisonParts {
                left: as_text(
                    &db,
                    cairn_node::patient::compare::chart_facts(&*db, &left).await,
                ),
                right: as_text(
                    &db,
                    cairn_node::patient::compare::chart_facts(&*db, &right).await,
                ),
                findings: as_text(
                    &db,
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
    credential: Option<Credential>,
) -> Result<LinkReportView, ErrorView> {
    let (patient, left, other, _) =
        resolve_pair(state, Act::Link, patient_id, &charts, other_id).await?;
    // Only a CHANGED set is "the other record changed"; an unreadable one is a window fault
    // and keeps its own wording — the right-hand twin of `resolve_pair`'s rule (PR #707 review).
    let right =
        check_displayed_set(&chart_set_of(state, other).await?, &other_charts).map_err(|e| {
            if e == ChartSetError::Changed {
                refused(OTHER_CHANGED)
            } else {
                e.into_view()
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
    // Linking is a clinical act, so a held key it uses counts as activity (`signer_for_act`).
    // A missing or wrong passphrase is not a verdict: `key_locked` keeps the Link button.
    let (human_sk, human_kid) = state
        .signer_for_act(credential, Now::read())
        .await
        .map_err(|r| key_locked(&r))?;
    let mut db = state
        .db
        .as_ref()
        .ok_or_else(|| refused("no database connection"))?
        .lock()
        .await?;
    let reviewer = cairn_node::chart_link::Reviewer {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    // No gesture-timing row: db/044's `gesture_kind` CHECK admits only signoff/cease; the
    // runbook's stopwatch measures this gesture (design "R2b").
    // `link_charts_as_shown` re-reads both records under the link lock (CARNLK) before signing
    // (#708): the checks above close the human-scale Compare→Link window, and this closes the
    // last milliseconds — a peer's link applied between them and the submit is refused as a
    // changed record ("compare again"), never signed over.
    let shown = cairn_node::chart_link::RecordsShown {
        this: &left,
        other: &right,
        pair: None,
    };
    let outcome = cairn_node::chart_link::link_charts_as_shown(
        &mut db,
        patient,
        other,
        shown,
        &reviewer,
        &state.node_origin,
    )
    .await
    .map_err(|e| link_error_view(&db.explain(e)))?;
    // `left`: the set this link left the record on screen as, so the re-read never announces
    // the clinician's own link back to them (#797). Attached only if the link changed the
    // record (`LinkReportView::left`).
    Ok(link_report(outcome.effect, &outcome.charts, &compared).left(&outcome.charts, patient))
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
    credential: Option<Credential>,
) -> Result<LinkReportView, ErrorView> {
    link_impl(
        &state,
        &patient_id,
        charts,
        &other_id,
        other_charts,
        credential,
    )
    .await
}

#[cfg(test)]
#[path = "same_person_tests.rs"]
mod tests;
