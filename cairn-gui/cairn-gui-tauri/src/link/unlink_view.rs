//! Every sentence the review panel shows, as pure functions (R2b-2, R6). The same rule as
//! `view.rs`: on this panel the wording IS the safety content.
//!
//! The panel is R2b-2's side-by-side comparison, opened from a line's one door on "How these
//! charts are linked" ("Review this link…" / "Review this judgement…"). Since R6 it is a REVIEW
//! panel: [`link_review_view`] decides which verdicts it offers on the pair the line showed —
//! "Not the same person — unlink" (worded here) and the "same person" verdict
//! (`confirm_view.rs`'s [`same_verdict`]). Both verdicts send back the content address of the
//! judgement standing on the pair as the panel read it ([`LinkReviewView::standing`]), and the
//! node signs neither if that judgement has changed since.

use super::confirm_view::{same_verdict, standing_agrees, PAIR_CHANGED_MID_READ};
use super::record_links::PairKind;
use super::unlink::ShownPair;
use super::view::{
    fact_rows, finding_line, heading, refused, ColumnView, FactRowView, LinkReportView,
};
use crate::funnel::view::ErrorView;
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::{LinkEffect, StandingLink};
use cairn_node::patient::compare::{ChartFacts, VetoFinding};
use serde::Serialize;
use uuid::Uuid;

/// Refused because the link is no longer one of the record's standing links — a peer's unlink
/// landed, or this clinician already undid it — so there is nothing left to judge. Also the
/// answer, by design, to a link id that does not parse, to a pair kind the window did not send
/// it, to a separation that no longer stands, and to a Compare with no database: in each case
/// there is no standing pair to act on, and nothing is guessed.
pub const LINK_GONE: &str =
    "that link is no longer part of this record — nothing was done; reload the chart";

/// Refused because the verdict arrived without the content address of the judgement standing on
/// the pair — the address the review panel read and the node re-checks before signing. The panel
/// never offers a verdict without one, so this is a webview that sent something else; the pair
/// itself may be fine, so it is never worded as [`LINK_GONE`].
pub const STANDING_UNREAD: &str = "the comparison did not read which judgement stands on this \
     pair — nothing was done; review it again";

/// Both verdicts' guard on the standing address the panel sent back (`confirm.rs`, `unlink.rs`):
/// without it the node has nothing to re-check the pair against, so nothing is signed.
pub fn require_standing(standing: &[u8]) -> Result<(), ErrorView> {
    if standing.is_empty() {
        Err(refused(STANDING_UNREAD))
    } else {
        Ok(())
    }
}

/// The reads a review panel is built from, each of which may have failed alone.
pub struct UnlinkParts {
    /// The lower chart's identity facts.
    pub low: Result<Vec<ChartFacts>, String>,
    /// The higher chart's identity facts.
    pub high: Result<Vec<ChartFacts>, String>,
    /// The veto findings between the two charts.
    pub findings: Result<Vec<VetoFinding>, String>,
    /// The judgement standing on the pair (`standing_link`). Its content address is what either
    /// verdict sends back so the node can refuse it if the pair changed since this read — kept
    /// only when it agrees with the line ([`standing_agrees`]). `Ok(None)`: no assertion stands
    /// on the pair, though the line showed one — it changed between the two reads.
    pub standing: Result<Option<StandingLink>, String>,
}

/// What the review panel hands the webview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkReviewView {
    /// Veto findings, worded; hard ones first. Empty means render nothing (never "no conflicts").
    pub findings: Vec<String>,
    /// The low chart's column, then the high chart's.
    pub columns: Vec<ColumnView>,
    /// The fact rows, a cell per column.
    pub rows: Vec<FactRowView>,
    /// The record's chart set this comparison was made FROM — sent back with either verdict,
    /// which refuses a changed one.
    pub charts: Vec<String>,
    /// The pair's lower chart id, sent back with either verdict.
    pub low: String,
    /// The pair's higher chart id, sent back with either verdict.
    pub high: String,
    /// What the pair is (a standing link, or a bridged separation), sent back with the verdict.
    pub kind: PairKind,
    /// What could NOT be read. Non-empty means neither verdict is offered.
    pub problems: Vec<String>,
    /// True only when everything was read, the pair is a standing link, and the standing
    /// judgement's address was read: never offer a judgement on a partial comparison, a
    /// separated pair is already unlinked, and the Unlink cannot be sent without the address.
    pub can_unlink: bool,
    /// The "same person" button's text ([`super::confirm_view::CONFIRM_SAME`] /
    /// [`super::confirm_view::LINK_THEM`]), or `None` when that verdict does not apply or the
    /// panel is partial.
    pub same_label: Option<String>,
    /// A sentence beside the verdicts: why "same person" is not offered, or what it overrides.
    pub same_note: Option<String>,
    /// True only when everything was read, a "same person" verdict applies, and the standing
    /// judgement's address was read (the verdict cannot be sent without it).
    pub can_confirm: bool,
    /// The standing judgement's content address — empty when unread. Sent back with either
    /// verdict. A content address, not key material: it serializes as a number array.
    pub standing: Vec<u8>,
}

/// Assemble the panel from whatever was read; the availability rule is `comparison_view`'s:
/// show what was read, name what was not, offer no verdict unless everything was read.
///
/// `shown` is the pair as the list line showed it (`unlink.rs`'s `shown_pair`): its kind decides
/// whether Unlink applies, and with the two charts' held flags it decides the "same person"
/// verdict ([`same_verdict`]).
pub fn link_review_view(
    parts: UnlinkParts,
    charts: &ChartSet,
    low: Uuid,
    high: Uuid,
    shown: &ShownPair,
) -> LinkReviewView {
    let mut problems = vec![];
    let mut take = |r: Result<Vec<ChartFacts>, String>, chart: Uuid| match r {
        Ok(v) => v,
        Err(e) => {
            problems.push(format!(
                "Chart {chart}'s identity facts could not be read: {e}"
            ));
            vec![]
        }
    };
    let low_facts = take(parts.low, low);
    let high_facts = take(parts.high, high);
    let findings = match parts.findings {
        Ok(f) => f.iter().map(finding_line).collect(),
        Err(e) => {
            problems.push(format!(
                "The check for disagreeing facts could not be run: {e}"
            ));
            vec![]
        }
    };
    let standing = match parts.standing {
        Ok(Some(judgement)) if standing_agrees(&judgement, shown) => judgement.content_address,
        // The line showed a standing judgement (a link, or a clinician's "different people"); the
        // panel's read, under another lock, finds a different one — or none at all. Something
        // landed between the two reads: offer NO verdict (a problem withholds both) and never
        // send an address back that the clinician was not shown.
        Ok(_) => {
            problems.push(PAIR_CHANGED_MID_READ.into());
            vec![]
        }
        Err(e) => {
            problems.push(format!(
                "Which judgement stands on this pair could not be read: {e}"
            ));
            vec![]
        }
    };
    let both: Vec<ChartFacts> = low_facts.into_iter().chain(high_facts).collect();
    // Empty when a facts read failed — which already put a problem on the panel, so no verdict
    // is offered whatever `same_verdict` makes of it.
    let held: Vec<(Uuid, bool)> = both.iter().map(|f| (f.patient_id, f.held)).collect();
    let same = same_verdict(shown, low, high, &held);
    let whole = problems.is_empty();
    // A note that explains what the button overrides goes with the button; a note that says why
    // there is NO button (a chart not held here) is true whatever failed to read, so it stays.
    let same_note = match (&same.label, whole) {
        (Some(_), false) => None,
        _ => same.note,
    };
    // A partial read offers NO verdict — a judgement needs the whole picture: the label goes
    // too, so the webview never draws a "same person" button that cannot be pressed.
    let same_label = if whole { same.label } else { None };
    let has_standing = !standing.is_empty();
    LinkReviewView {
        findings,
        columns: both
            .iter()
            .map(|f| ColumnView {
                patient_id: f.patient_id.to_string(),
                heading: heading(f),
            })
            .collect(),
        rows: fact_rows(&both),
        charts: charts.members().iter().map(Uuid::to_string).collect(),
        low: low.to_string(),
        high: high.to_string(),
        kind: shown.kind,
        can_unlink: whole && shown.kind == PairKind::Link && has_standing,
        can_confirm: same_label.is_some() && has_standing,
        same_label,
        same_note,
        standing,
        problems,
    }
}

/// What the unlink did, as the outcome line says it. `before` is the record compared from,
/// `after` the record now (read in the judgement's own transaction). Never "Unlinked" for an
/// unlink that did not split the record (R2a: recorded is not took effect).
pub fn unlink_report(
    effect: LinkEffect,
    low: Uuid,
    high: Uuid,
    before: &ChartSet,
    after: &ChartSet,
) -> LinkReportView {
    let mut view = effect_report(effect, low, high, before, after);
    let joined = ids_not_in(after, before);
    let left = ids_not_in(before, after);
    if effect == LinkEffect::StillJoined && !(joined.is_empty() && left.is_empty()) {
        // The record changed while the clinician judged (a peer's concurrent link/unlink). The
        // generic StillJoined sentence says "this record did not change", which would now
        // contradict what we append, so restate it without that claim and name the change.
        view.sentence = format!(
            "Recorded that charts {low} and {high} are different people — but they still read \
             as one record through other links, so this unlink did not split them. The links \
             still joining them are listed under \"How these charts are linked\"."
        );
        if !left.is_empty() {
            view.sentence = format!(
                "{} Meanwhile chart(s) {} left this record — review them.",
                view.sentence,
                left.join(", ")
            );
        }
        if !joined.is_empty() {
            view.sentence = format!(
                "{} Meanwhile chart(s) {} joined this record — review them.",
                view.sentence,
                joined.join(", ")
            );
        }
        return view;
    }
    // Charts that joined the record concurrently were not in the comparison: name them,
    // as `link_report` does, so they are reviewed rather than assumed. (TookEffect already
    // names its own leavers inside `effect_report`.)
    if !joined.is_empty() {
        view.sentence = format!(
            "{} The record now also includes chart(s) {} that were not in the comparison — \
             review them.",
            view.sentence,
            joined.join(", ")
        );
    }
    view
}

/// The ids in `from` that `other` does not have.
fn ids_not_in(from: &ChartSet, other: &ChartSet) -> Vec<String> {
    from.members()
        .iter()
        .filter(|c| !other.contains(c))
        .map(Uuid::to_string)
        .collect()
}

/// The sentence for each [`LinkEffect`], before [`unlink_report`] adds anything about charts
/// the comparison did not show.
fn effect_report(
    effect: LinkEffect,
    low: Uuid,
    high: Uuid,
    before: &ChartSet,
    after: &ChartSet,
) -> LinkReportView {
    match effect {
        LinkEffect::TookEffect => {
            let left = ids_not_in(before, after);
            let sentence = if left.is_empty() {
                // An unlink that took effect but split nothing contradicts itself; say so
                // rather than claim a split (mirrors `view.rs`'s `effect_report`, its link
                // StillJoined arm). Worded to be true wherever it lands: when the answer arrives
                // with another chart open, nothing is re-read for it.
                format!(
                    "Recorded that charts {low} and {high} are different people, but this record \
                     did not change the way an unlink should — check what the chart now combines."
                )
            } else {
                format!(
                    "Unlinked — chart(s) {} are no longer part of this record.",
                    left.join(", ")
                )
            };
            LinkReportView {
                sentence,
                reload: true,
            }
        }
        LinkEffect::StillJoined => LinkReportView {
            sentence: format!(
                "Recorded that charts {low} and {high} are different people — but they still read \
                 as one record through other links, so this record did not change. The links \
                 still joining them are listed under \"How these charts are linked\"."
            ),
            reload: true,
        },
        LinkEffect::Outranked => LinkReportView {
            sentence: "Recorded, but NOT in effect: a later judgement on this pair says these \
                       are the same person. The two judgements disagree — settle it with the \
                       person who made the other one. Unlinking again would normally record a newer \
                       judgement that overrules theirs — it would not settle the disagreement."
                .into(),
            reload: false,
        },
    }
}

/// A failed unlink, classified exactly as a failed link is.
pub fn unlink_error_view(e: &anyhow::Error) -> ErrorView {
    crate::link::view::judgement_error_view("unlink", e)
}

#[cfg(test)]
#[path = "unlink_view_tests.rs"]
mod tests;
