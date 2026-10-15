//! The review panel's commands — read ONE pair of a combined record side by side, and undo a
//! link from it (repair path R2b-2, ADR-0076 decision 4; #699 (a); R6, #716).
//!
//! Paper counterpart: lay the two folders' front sheets side by side, then either unclip them
//! and annotate "not the same person", or initial the clip "same person". Two acts: the line's
//! one door on "How these charts are linked" ("Review this link…" / "Review this judgement…")
//! opens the review panel (`compare_linked`), whose verdicts the panel decides (`unlink_view.rs`'s
//! `link_review_view`). The verdict click IS the signature under the unlocked key (ADR-0053);
//! there is no confirmation dialog (principle 3). This file holds the read and the Unlink; the
//! "same person" verdict's command builds on [`resolve_edge`] beside it.
//!
//! Every command applies the chart-command rules IN THIS ORDER, each pinned by a test: the chart
//! on screen (`displayed_patient`), the displayed set (`check_displayed_set`), the pair is still
//! what the line showed — a standing link, or a standing separation (`shown_pair`). A verdict
//! then checks fixture mode, then the key; the Compare, which writes nothing, needs neither (with
//! no database it refuses). The pair is named by its two charts; the node is told which chart it
//! is judged FROM, so a judgement on charts not held here is filed under the open chart (#699 (a)).

use super::record_links::{read_record_links, PairKind, RecordLinks};
use super::unlink_view::{
    link_review_view, unlink_error_view, unlink_report, LinkReviewView, UnlinkParts, LINK_GONE,
};
use super::view::{key_locked_for, refused, LinkReportView, THIS_CHANGED};
use super::{as_text, chart_set_of};
use crate::chart_set::{check_displayed_set, CHANGED};
use crate::funnel::view::{ErrorView, Retry};
use crate::state::{AppState, Now};
use cairn_medication_view::ChartSet;
use cairn_node::patient::edges::DoubtKind;
use uuid::Uuid;

/// Which command is asking [`resolve_edge`] — it decides only how a CHANGED set and an unread
/// links list are worded: a Compare writes nothing; an Unlink or a Confirm is about to sign.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Act {
    Compare,
    Unlink,
    /// The "same person" verdict. Built by R6 Task 7's `confirm_link`; the attribute below
    /// fails clippy once it is, so it cannot outlive its reason.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "R6 Task 7's `confirm_link` constructs it; delete this then"
        )
    )]
    Confirm,
}

impl Act {
    /// A verdict (Unlink, Confirm) is made from a comparison, so a changed set makes that
    /// comparison stale; a Compare is not.
    fn is_verdict(self) -> bool {
        self != Act::Compare
    }
}

/// The pair as the list line showed it, as re-read now: what it is and what is known about it.
/// The review panel decides its verdicts from this (`confirm_view::same_verdict`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownPair {
    /// A standing link, or a bridged separation.
    pub kind: PairKind,
    /// Whether the standing judgement is a clinician's (db/018's stored definition). Always
    /// true for a separation: db/054 reports only an attested "different people".
    pub attested: bool,
    /// The day the standing judgement was recorded (UTC, `YYYY-MM-DD`).
    pub recorded_on: String,
}

/// What [`resolve_edge`] establishes: every field named, because three chart ids of one type
/// side by side is a swap that compiles — and at the node call a swap would sign an attested
/// judgement about the wrong pair (PR #711 review).
pub(super) struct ResolvedEdge {
    /// The chart on screen: the one the judgement is made FROM.
    pub(super) opened: Uuid,
    /// Its displayed set, as checked against what the webview sent.
    pub(super) set: ChartSet,
    /// The pair's lower chart (canonical order).
    pub(super) low: Uuid,
    /// The pair's higher chart.
    pub(super) high: Uuid,
    /// The pair as re-read now — still the kind the line showed.
    pub(super) shown: ShownPair,
}

/// The screen checks every review command shares, in the order the module doc gives.
///
/// For Compare a changed set keeps the list's own wording ("reload the chart"); for a verdict
/// `charts` is the set the COMPARISON was made from, so a change means the comparison is stale
/// and reads [`THIS_CHANGED`] (the R2b-1 rule). An unparseable id and a pair the record no
/// longer shows as `kind` both read [`LINK_GONE`]: nothing is guessed, nothing is signed.
///
/// Takes `state.db`'s lock itself (through `chart_set_of` and `read_record_links`), so the
/// caller must not hold it.
pub(super) async fn resolve_edge(
    state: &AppState,
    act: Act,
    patient_id: &str,
    charts: &[String],
    low: &str,
    high: &str,
    kind: PairKind,
) -> Result<ResolvedEdge, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    let set = check_displayed_set(&chart_set_of(state, patient).await?, charts).map_err(|e| {
        if act.is_verdict() && e == CHANGED {
            refused(THIS_CHANGED)
        } else {
            refused(e)
        }
    })?;
    let (Ok(low), Ok(high)) = (low.parse::<Uuid>(), high.parse::<Uuid>()) else {
        return Err(refused(LINK_GONE));
    };
    let (low, high) = cairn_node::chart_link::canonical_pair(low, high);
    let links = read_record_links(state, &set).await;
    let shown = shown_pair(links, low, high, kind, act)?;
    Ok(ResolvedEdge {
        opened: patient,
        set,
        low,
        high,
        shown,
    })
}

/// The membership stage as a pure function: `low`/`high` must already be canonical. The pair
/// must still be what the line showed — a `Link` among the standing links, a `Separated` among
/// db/054's separations (a Clash doubt is a property of a link, never a separation). A failed
/// read is a window fault, not a verdict on the pair — retryable, worded per act (a Compare
/// changed nothing, so it does not say "nothing was done"); a pair the record no longer shows
/// that way reads [`LINK_GONE`].
pub(super) fn shown_pair(
    links: Result<RecordLinks, String>,
    low: Uuid,
    high: Uuid,
    kind: PairKind,
    act: Act,
) -> Result<ShownPair, ErrorView> {
    let links = links.map_err(|e| ErrorView {
        text: if act.is_verdict() {
            format!("Could not read the links joining this record's charts — nothing was done: {e}")
        } else {
            format!("Could not read the links joining this record's charts: {e}")
        },
        retry: Retry::Now,
    })?;
    let found = match kind {
        PairKind::Link => links
            .edges
            .iter()
            .find(|e| (e.low, e.high) == (low, high))
            .map(|e| ShownPair {
                kind,
                attested: e.attested,
                recorded_on: e.recorded_on.clone(),
            }),
        PairKind::Separated => links
            .doubts
            .iter()
            .find(|d| d.kind == DoubtKind::Separated && (d.low, d.high) == (low, high))
            .map(|d| ShownPair {
                kind,
                attested: true,
                recorded_on: d.recorded_on.clone(),
            }),
    };
    found.ok_or_else(|| refused(LINK_GONE))
}

/// "Review this link…" / "Review this judgement…" — read the pair's two charts side by side,
/// with the verdicts that apply. Read-only.
///
/// `kind` is the line's own kind, as the webview got it; an unknown one is refused before
/// anything is read (it cannot name a pair). The facts, the findings and which judgement stands
/// are read under ONE lock, after [`resolve_edge`] has released its own.
pub async fn compare_linked_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    low: &str,
    high: &str,
    kind: &str,
) -> Result<LinkReviewView, ErrorView> {
    let Some(kind) = PairKind::parse(kind) else {
        return Err(refused(LINK_GONE));
    };
    let edge = resolve_edge(state, Act::Compare, patient_id, &charts, low, high, kind).await?;
    // Fixture charts are never linked, so `resolve_edge` cannot pass without a database;
    // refuse defensively rather than invent a comparison.
    let Some(db) = state.db.as_ref() else {
        return Err(refused(LINK_GONE));
    };
    let db = db.lock().await;
    let (l, h) = (ChartSet::single(edge.low), ChartSet::single(edge.high));
    let standing = cairn_node::chart_link::standing_link(&*db, edge.low, edge.high).await;
    let parts = UnlinkParts {
        low: as_text(cairn_node::patient::compare::chart_facts(&*db, &l).await),
        high: as_text(cairn_node::patient::compare::chart_facts(&*db, &h).await),
        findings: as_text(cairn_node::patient::compare::cross_vetoes(&*db, &l, &h).await),
        standing: as_text(standing).map(|s| s.map(|s| s.content_address)),
    };
    Ok(link_review_view(
        parts,
        &edge.set,
        edge.low,
        edge.high,
        &edge.shown,
    ))
}

/// "Unlink — not the same person" — the attested judgement on ONE link, then what it did.
pub async fn unlink_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    low: &str,
    high: &str,
) -> Result<LinkReportView, ErrorView> {
    // An unlink applies to a standing link only: a separated pair is already unlinked.
    let edge = resolve_edge(
        state,
        Act::Unlink,
        patient_id,
        &charts,
        low,
        high,
        PairKind::Link,
    )
    .await?;
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
        edge.low,
        edge.high,
        Some(edge.opened),
        &reviewer,
        &state.node_origin,
    )
    .await
    .map_err(|e| unlink_error_view(&e))?;
    Ok(unlink_report(
        outcome.effect,
        edge.low,
        edge.high,
        &edge.set,
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
    kind: String,
) -> Result<LinkReviewView, ErrorView> {
    compare_linked_impl(&state, &patient_id, charts, &low, &high, &kind).await
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
#[path = "unlink_tests.rs"]
mod tests;
