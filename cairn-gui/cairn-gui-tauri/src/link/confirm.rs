//! The review panel's "same person" verdict — record a clinician's attested judgement that the
//! two charts of ONE pair inside the open record are the same person (repair path R6, #716).
//!
//! Two cases, one act. On a DOUBTED link ("Same person — confirm") the link already stands but
//! no clinician vouches for it — a machine made it, or a veto or a clinician's "different people"
//! elsewhere in the record casts doubt on it — and the clinician's judgement now does. On a
//! BRIDGED separation ("Same person — link them") a clinician once recorded "different people"
//! but other links still join the two charts; the new judgement overlays theirs, both stay on
//! record. Either way the node writes an ordinary attested `identity.link.asserted` through the
//! existing `chart_link` judgement path: no new event type, no wire change.
//!
//! Paper counterpart: lay the two folders side by side and initial the clip "same person". The
//! verdict click IS that initial — the signature under the unlocked key (ADR-0053). There is no
//! confirmation dialog (principle 3): the comparison the clinician just read is the check.
//!
//! The rule order: the pair kind the panel sent must be one we know (an unknown one names no
//! pair, so nothing is guessed); then [`resolve_edge`] — the chart on screen, the displayed set
//! (a changed one makes the comparison stale), the pair still being what the line showed; then
//! fixture mode; then the standing address is present ([`require_standing`]); then the key (a
//! locked key is not a verdict: the button stays for after the unlock). The screen rules are
//! pinned by this file's tests (and `resolve_edge`'s own in `unlink_tests.rs`); the write-side
//! arms — fixture, key, the node call — are pinned by the node's DB tests
//! (`cairn-node/tests/confirm_link.rs`), since the window has no DB-backed tests yet (#738).
//!
//! Why `standing` travels: the panel read the content address of the judgement standing on the
//! pair when it was drawn (`compare_linked`). The node re-reads the record AND the pair under
//! the chart-link lock before signing (`confirm_link_as_shown`, #708) and refuses if the pair's
//! standing judgement is no longer that one — so neither a colleague's unlink nor a colleague's
//! confirm that landed while this clinician was reading is ever signed over.

use super::confirm_view::{confirm_report, CONFIRM_SAME, LINK_THEM};
use super::record_links::PairKind;
use super::unlink::{resolve_edge, Act};
use super::unlink_view::{require_standing, LINK_GONE};
use super::view::{judgement_error_view, key_locked_for, refused, LinkReportView};
use crate::funnel::view::ErrorView;
use crate::state::{AppState, Now};

/// The button the clinician pressed, by pair kind — named in a locked-key refusal so they know
/// which button to press again after the unlock.
fn verdict_label(kind: PairKind) -> &'static str {
    match kind {
        PairKind::Link => CONFIRM_SAME,
        PairKind::Separated => LINK_THEM,
    }
}

/// "Same person — confirm" / "Same person — link them": the attested judgement on ONE pair of
/// the open record, then what it did.
///
/// `charts`, `low`, `high`, `kind` and `standing` are exactly what the review panel
/// (`LinkReviewView`) handed the webview; the webview sends them back untouched.
pub async fn confirm_link_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    low: &str,
    high: &str,
    kind: &str,
    standing: Vec<u8>,
) -> Result<LinkReportView, ErrorView> {
    let kind = PairKind::parse(kind).ok_or_else(|| refused(LINK_GONE))?;
    // 1–3: the chart on screen, the displayed set, the pair still what the panel showed — as
    // the kind the panel sent, so a confirm on a link is never applied to a separation.
    let edge = resolve_edge(state, Act::Confirm, patient_id, &charts, low, high, kind).await?;
    // 4: fixture mode (unreachable in practice: fixture charts are never linked — as unlink).
    if state.is_mock() {
        return Err(refused(
            "fixture mode: this window is showing mock data and cannot write",
        ));
    }
    // An empty address means the panel never read which judgement stands: nothing to sign over.
    require_standing(&standing)?;
    // 5: the key. A confirm is a clinical act, so taking the key counts as activity.
    let (human_sk, human_kid) = state
        .live_key(Now::read())
        .await
        .ok_or_else(|| key_locked_for(verdict_label(kind)))?;
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
    // The node re-reads the record AND the pair under the chart-link lock before signing.
    let outcome = cairn_node::chart_link::confirm_link_as_shown(
        &mut db,
        edge.low,
        edge.high,
        &edge.set,
        &standing,
        &reviewer,
        &state.node_origin,
    )
    .await
    .map_err(|e| judgement_error_view("confirmation", &e))?;
    Ok(confirm_report(outcome.effect, kind, edge.low, edge.high))
}

// ---- Tauri forwarder (camelCase JS keys → snake_case parameters). ----

/// JS keys: `patientId, charts, low, high, kind, standing`.
#[tauri::command]
pub async fn confirm_link(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    low: String,
    high: String,
    kind: String,
    standing: Vec<u8>,
) -> Result<LinkReportView, ErrorView> {
    confirm_link_impl(&state, &patient_id, charts, &low, &high, &kind, standing).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use uuid::Uuid;

    /// The mock window's one chart (copied from `unlink_tests.rs`, file-local by design).
    fn fixture() -> Uuid {
        cairn_gui_data::mock::fixtures::FIXTURE_UUID
            .parse()
            .unwrap()
    }
    fn on_screen() -> (String, Vec<String>) {
        (fixture().to_string(), vec![fixture().to_string()])
    }

    #[tokio::test]
    async fn confirm_is_bound_to_the_chart_on_screen() {
        let state = AppState::mock(Some(fixture()));
        let err = confirm_link_impl(
            &state,
            &Uuid::from_u128(9).to_string(),
            vec![],
            "1",
            "2",
            "link",
            vec![],
        )
        .await
        .unwrap_err();
        assert!(err.text.contains("not the chart"));
    }

    #[tokio::test]
    async fn confirm_refuses_a_changed_record_as_a_stale_comparison() {
        let state = AppState::mock(Some(fixture()));
        let (p, _) = on_screen();
        let err = confirm_link_impl(
            &state,
            &p,
            vec![p.clone(), Uuid::from_u128(7).to_string()],
            &Uuid::from_u128(1).to_string(),
            &Uuid::from_u128(2).to_string(),
            "link",
            vec![],
        )
        .await
        .unwrap_err();
        assert_eq!(err.text, crate::link::view::THIS_CHANGED);
    }

    #[test]
    fn a_locked_key_names_the_button_per_kind() {
        assert_eq!(verdict_label(PairKind::Link), CONFIRM_SAME);
        assert_eq!(verdict_label(PairKind::Separated), LINK_THEM);
    }

    #[tokio::test]
    async fn an_unknown_kind_is_refused_not_guessed() {
        let state = AppState::mock(Some(fixture()));
        let (p, charts) = on_screen();
        let err = confirm_link_impl(
            &state,
            &p,
            charts,
            &p,
            &Uuid::from_u128(2).to_string(),
            "merge",
            vec![],
        )
        .await
        .unwrap_err();
        assert_eq!(err.text, crate::link::unlink_view::LINK_GONE);
    }

    #[tokio::test]
    async fn confirm_refuses_a_link_the_record_no_longer_has() {
        let state = AppState::mock(Some(fixture()));
        let (p, charts) = on_screen();
        let err = confirm_link_impl(
            &state,
            &p,
            charts,
            &p,
            &Uuid::from_u128(2).to_string(),
            "link",
            vec![],
        )
        .await
        .unwrap_err();
        assert_eq!(err.text, crate::link::unlink_view::LINK_GONE);
    }
}
