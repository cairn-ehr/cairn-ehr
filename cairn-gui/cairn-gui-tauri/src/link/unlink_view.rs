//! Every sentence the "Not the same person" panel shows, as pure functions (R2b-2). The same
//! rule as `view.rs`: on this panel the wording IS the safety content.

use super::view::{
    fact_rows, finding_line, heading, judgement_error_from, ColumnView, FactRowView, LinkReportView,
};
use crate::funnel::view::ErrorView;
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::LinkEffect;
use cairn_node::patient::compare::{ChartFacts, VetoFinding};
use serde::Serialize;
use uuid::Uuid;

/// Refused because the link is no longer one of the record's standing links — a peer's unlink
/// landed, or this clinician already undid it — so there is nothing left to judge.
pub const LINK_GONE: &str =
    "that link is no longer part of this record — nothing was done; reload the chart";

/// The three reads an unlink comparison is built from, each of which may have failed alone.
pub struct UnlinkParts {
    /// The lower chart's identity facts.
    pub low: Result<Vec<ChartFacts>, String>,
    /// The higher chart's identity facts.
    pub high: Result<Vec<ChartFacts>, String>,
    /// The veto findings between the two charts.
    pub findings: Result<Vec<VetoFinding>, String>,
}

/// What the unlink comparison hands the webview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnlinkComparisonView {
    /// Veto findings, worded; hard ones first. Empty means render nothing (never "no conflicts").
    pub findings: Vec<String>,
    /// The low chart's column, then the high chart's.
    pub columns: Vec<ColumnView>,
    /// The fact rows, a cell per column.
    pub rows: Vec<FactRowView>,
    /// The record's chart set this comparison was made FROM — sent back with the unlink,
    /// which refuses a changed one.
    pub charts: Vec<String>,
    /// The link's lower chart id, sent back with the unlink.
    pub low: String,
    /// The link's higher chart id, sent back with the unlink.
    pub high: String,
    /// What could NOT be read. Non-empty means `can_unlink` is false.
    pub problems: Vec<String>,
    /// True only when everything was read: never offer a judgement on a partial comparison.
    pub can_unlink: bool,
}

/// Assemble the panel from whatever was read; the availability rule is `comparison_view`'s:
/// show what was read, name what was not, offer no unlink unless everything was read.
pub fn unlink_comparison_view(
    parts: UnlinkParts,
    charts: &ChartSet,
    low: Uuid,
    high: Uuid,
) -> UnlinkComparisonView {
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
    let both: Vec<ChartFacts> = low_facts.into_iter().chain(high_facts).collect();
    UnlinkComparisonView {
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
        can_unlink: problems.is_empty(),
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
                // rather than claim a split (mirrors `effect_report`'s link StillJoined arm).
                format!(
                    "Recorded that charts {low} and {high} are different people, but this record \
                     did not change the way an unlink should — the chart is being re-read so you \
                     can see what it now combines."
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
    judgement_error_from("unlink", cairn_gui_live::error::data_error_from(e))
}

#[cfg(test)]
#[path = "unlink_view_tests.rs"]
mod tests;
