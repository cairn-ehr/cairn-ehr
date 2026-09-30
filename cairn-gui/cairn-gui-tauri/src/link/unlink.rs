//! "Not the same person" — undo ONE link of a combined record from the window (repair path
//! R2b-2, ADR-0076 decision 4; #699 (a)).
//!
//! Paper counterpart: unclip the two folders and annotate "not the same person". Two acts:
//! "Not the same person…" on the link's line (reads the two charts side by side), then
//! "Unlink — not the same person". The Unlink click IS the signature under the unlocked key
//! (ADR-0053); there is no confirmation dialog (principle 3).
//!
//! Both commands apply the chart-command rules IN THIS ORDER, each pinned by a test: the chart
//! on screen (`displayed_patient`), the displayed set (`check_displayed_set`), the link is still
//! one of that set's standing links (`record_edges`). Only then fixture mode, then the key.
//! The link is named by its two charts; the node is told which chart it is judged FROM, so an
//! unlink of a link whose charts are not held here is filed under the open chart (#699 (a)).

use super::record_links::read_record_edges;
use super::unlink_view::{
    unlink_comparison_view, unlink_error_view, unlink_report, UnlinkComparisonView, UnlinkParts,
    LINK_GONE,
};
use super::view::{key_locked_for, refused, LinkReportView, THIS_CHANGED};
use super::{as_text, chart_set_of};
use crate::chart_set::{check_displayed_set, CHANGED};
use crate::funnel::view::{ErrorView, Retry};
use crate::state::{AppState, Now};
use cairn_medication_view::ChartSet;
use cairn_node::patient::edges::RecordEdge;
use uuid::Uuid;

/// Which command is asking [`resolve_edge`] — it decides only how a CHANGED set is worded.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    Compare,
    Unlink,
}

/// The screen checks both commands share, in the order the module doc gives. Returns the
/// opened chart, its displayed set, and the link's canonical `(low, high)`.
///
/// For Compare a changed set keeps the list's own wording ("reload the chart"); for Unlink
/// `charts` is the set the COMPARISON was made from, so a change means the comparison is stale
/// and reads [`THIS_CHANGED`] (the R2b-1 rule). An unparseable id and a link the record no
/// longer has both read [`LINK_GONE`]: nothing is guessed, nothing is signed.
async fn resolve_edge(
    state: &AppState,
    act: Act,
    patient_id: &str,
    charts: &[String],
    low: &str,
    high: &str,
) -> Result<(Uuid, ChartSet, Uuid, Uuid), ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    let set = check_displayed_set(&chart_set_of(state, patient).await?, charts).map_err(|e| {
        if act == Act::Unlink && e == CHANGED {
            refused(THIS_CHANGED)
        } else {
            refused(e)
        }
    })?;
    let (Ok(low), Ok(high)) = (low.parse::<Uuid>(), high.parse::<Uuid>()) else {
        return Err(refused(LINK_GONE));
    };
    let (low, high) = cairn_node::chart_link::canonical_pair(low, high);
    let edges = read_record_edges(state, &set).await;
    standing_edge(edges, low, high, act)?;
    Ok((patient, set, low, high))
}

/// The membership stage as a pure function: `low`/`high` must already be canonical. A failed
/// read is a window fault, not a verdict on the link — retryable, worded per act (a Compare
/// changed nothing, so it does not say "nothing was done"); a link the record does not have
/// reads [`LINK_GONE`].
fn standing_edge(
    edges: Result<Vec<RecordEdge>, String>,
    low: Uuid,
    high: Uuid,
    act: Act,
) -> Result<(), ErrorView> {
    let edges = edges.map_err(|e| ErrorView {
        text: match act {
            Act::Compare => {
                format!("Could not read the links joining this record's charts: {e}")
            }
            Act::Unlink => format!(
                "Could not read the links joining this record's charts — nothing was done: {e}"
            ),
        },
        retry: Retry::Now,
    })?;
    if edges.iter().any(|e| (e.low, e.high) == (low, high)) {
        Ok(())
    } else {
        Err(refused(LINK_GONE))
    }
}

/// "Not the same person…" — read the link's two charts side by side. Read-only.
pub async fn compare_linked_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    low: &str,
    high: &str,
) -> Result<UnlinkComparisonView, ErrorView> {
    let (_, set, low, high) =
        resolve_edge(state, Act::Compare, patient_id, &charts, low, high).await?;
    // Fixture charts are never linked, so `resolve_edge` cannot pass without a database;
    // refuse defensively rather than invent a comparison.
    let Some(db) = state.db.as_ref() else {
        return Err(refused(LINK_GONE));
    };
    let db = db.lock().await;
    let (l, h) = (ChartSet::single(low), ChartSet::single(high));
    let parts = UnlinkParts {
        low: as_text(cairn_node::patient::compare::chart_facts(&*db, &l).await),
        high: as_text(cairn_node::patient::compare::chart_facts(&*db, &h).await),
        findings: as_text(cairn_node::patient::compare::cross_vetoes(&*db, &l, &h).await),
    };
    Ok(unlink_comparison_view(parts, &set, low, high))
}

/// "Unlink — not the same person" — the attested judgement on ONE link, then what it did.
pub async fn unlink_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    low: &str,
    high: &str,
) -> Result<LinkReportView, ErrorView> {
    let (patient, set, low, high) =
        resolve_edge(state, Act::Unlink, patient_id, &charts, low, high).await?;
    if state.is_mock() {
        return Err(refused(
            "fixture mode: this window is showing mock data and cannot write",
        ));
    }
    // An unlink is a clinical act, so taking the key counts as activity (`live_key`).
    let (human_sk, human_kid) = state
        .live_key(Now::read())
        .await
        .ok_or_else(|| key_locked_for("Unlink — not the same person"))?;
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
    // No gesture-timing row: db/044's CHECK admits only signoff/cease; the runbook's
    // stopwatch measures this gesture.
    let outcome = cairn_node::chart_link::unlink_charts(
        &mut db,
        low,
        high,
        Some(patient),
        &reviewer,
        &state.node_origin,
    )
    .await
    .map_err(|e| unlink_error_view(&e))?;
    Ok(unlink_report(
        outcome.effect,
        low,
        high,
        &set,
        &outcome.charts,
    ))
}

// ---- Tauri forwarders (camelCase JS keys → snake_case parameters). ----

#[tauri::command]
pub async fn compare_linked(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    low: String,
    high: String,
) -> Result<UnlinkComparisonView, ErrorView> {
    compare_linked_impl(&state, &patient_id, charts, &low, &high).await
}

#[tauri::command]
pub async fn unlink_records(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    low: String,
    high: String,
) -> Result<LinkReportView, ErrorView> {
    unlink_impl(&state, &patient_id, charts, &low, &high).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use uuid::Uuid;

    fn fixture() -> Uuid {
        cairn_gui_data::mock::fixtures::FIXTURE_UUID
            .parse()
            .unwrap()
    }
    fn on_screen() -> (String, Vec<String>) {
        (fixture().to_string(), vec![fixture().to_string()])
    }

    #[tokio::test]
    async fn compare_linked_is_bound_to_the_chart_on_screen() {
        let state = AppState::mock(Some(fixture()));
        let err = compare_linked_impl(&state, &Uuid::from_u128(9).to_string(), vec![], "1", "2")
            .await
            .unwrap_err();
        assert!(
            err.text.contains("not the chart"),
            "must refuse a chart that is not on screen"
        );
    }

    #[tokio::test]
    async fn unlink_is_bound_to_the_chart_on_screen() {
        let state = AppState::mock(Some(fixture()));
        let err = unlink_impl(&state, &Uuid::from_u128(9).to_string(), vec![], "1", "2")
            .await
            .unwrap_err();
        assert!(
            err.text.contains("not the chart"),
            "must refuse a chart that is not on screen"
        );
    }

    #[tokio::test]
    async fn unlink_refuses_a_changed_record() {
        let state = AppState::mock(Some(fixture()));
        let (p, _) = on_screen();
        let err = unlink_impl(
            &state,
            &p,
            vec![p.clone(), Uuid::from_u128(7).to_string()],
            &Uuid::from_u128(1).to_string(),
            &Uuid::from_u128(2).to_string(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.text, crate::link::view::THIS_CHANGED);
    }

    #[tokio::test]
    async fn compare_linked_refuses_a_changed_record_in_the_lists_wording() {
        let state = AppState::mock(Some(fixture()));
        let (p, _) = on_screen();
        let err = compare_linked_impl(
            &state,
            &p,
            vec![p.clone(), Uuid::from_u128(7).to_string()],
            &Uuid::from_u128(1).to_string(),
            &Uuid::from_u128(2).to_string(),
        )
        .await
        .unwrap_err();
        assert_ne!(err.text, crate::link::view::THIS_CHANGED);
        assert!(err.text.contains("linked charts changed"));
    }

    /// Review Focus 3.
    #[tokio::test]
    async fn unlink_refuses_a_link_the_record_no_longer_has() {
        let state = AppState::mock(Some(fixture()));
        let (p, charts) = on_screen();
        let err = unlink_impl(&state, &p, charts, &p, &Uuid::from_u128(2).to_string())
            .await
            .unwrap_err();
        assert_eq!(err.text, crate::link::unlink_view::LINK_GONE);
    }

    /// Cannot tell the membership stage from compare's `db None` -> LINK_GONE; the pure
    /// `standing_edge` tests pin the membership stage.
    #[tokio::test]
    async fn compare_linked_refuses_a_link_the_record_no_longer_has() {
        let state = AppState::mock(Some(fixture()));
        let (p, charts) = on_screen();
        let err = compare_linked_impl(&state, &p, charts, &p, &Uuid::from_u128(2).to_string())
            .await
            .unwrap_err();
        assert_eq!(err.text, crate::link::unlink_view::LINK_GONE);
    }

    #[tokio::test]
    async fn an_unparseable_link_is_refused_not_guessed() {
        let state = AppState::mock(Some(fixture()));
        let (p, charts) = on_screen();
        let err = unlink_impl(&state, &p, charts, "not-a-uuid", "2")
            .await
            .unwrap_err();
        assert_eq!(err.text, crate::link::unlink_view::LINK_GONE);
    }

    fn edge(a: u128, b: u128) -> RecordEdge {
        RecordEdge {
            low: Uuid::from_u128(a),
            high: Uuid::from_u128(b),
            attested: true,
            recorded_on: "2026-09-28".into(),
        }
    }

    #[test]
    fn a_present_link_passes_however_the_pair_was_ordered() {
        let (l, h) = (Uuid::from_u128(1), Uuid::from_u128(2));
        assert!(standing_edge(Ok(vec![edge(1, 2)]), l, h, Act::Unlink).is_ok());
        let (l, h) = cairn_node::chart_link::canonical_pair(h, l);
        assert!(standing_edge(Ok(vec![edge(1, 2)]), l, h, Act::Unlink).is_ok());
    }

    #[test]
    fn an_absent_link_is_gone() {
        let (l, h) = (Uuid::from_u128(1), Uuid::from_u128(3));
        let err = standing_edge(Ok(vec![edge(1, 2)]), l, h, Act::Unlink).unwrap_err();
        assert_eq!(err.text, LINK_GONE);
    }

    #[test]
    fn an_unread_edge_list_is_retryable_and_not_a_verdict() {
        let (l, h) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let err = standing_edge(Err("boom".into()), l, h, Act::Unlink).unwrap_err();
        assert_eq!(err.retry, Retry::Now);
        assert_ne!(err.text, LINK_GONE);
        assert!(err.text.contains("nothing was done"));
        let err = standing_edge(Err("boom".into()), l, h, Act::Compare).unwrap_err();
        assert_eq!(err.retry, Retry::Now);
        assert!(!err.text.contains("nothing was done"));
    }
}
