//! Every sentence the possible-duplicate banner shows, as pure functions (repair path R5a,
//! #680; design page "R5a — the banner, designed 2026-10-06").
//!
//! The banner is AMBIENT (§5.12): it never takes focus, never re-pops, and is never a modal.
//! What it may never do is be ABSENT when something is unknown — an empty section must only
//! ever mean "checked, none open". So every failed read below becomes a worded line, and
//! [`check_lines`] says when this record's check has not run, reusing R4's own sentences.
use crate::chart_set::MemberLine;
use crate::funnel::view::{ErrorView, Retry};
use crate::link::view::{judgement_error_from, medication_lines, refused, LinkReportView};
use cairn_gui_tab_medications::view::MedListView;
use cairn_medication_view::{ChartSet, PatientMedicationList};
use cairn_node::chart_link::LinkEffect;
use cairn_node::duplicate_check::{chart_line, status_line, CheckState};
use serde::Serialize;
use uuid::Uuid;

/// At most this many entries are drawn (each costs a medication read at chart open); the rest
/// are counted in `DuplicateSection::more`, never dropped silently.
pub const MAX_SHOWN: usize = 3;
pub const HEADING: &str = "Possible duplicate — not yet reviewed";
/// Above the other record's lines — read over EVERY chart of that record, so "record", not
/// "chart": they are someone else's until a human links the two. The compare panel's own
/// heading, word for word (index.html `#link-other-meds-heading`).
pub const OTHER_CHART_LABEL: &str = "On the other record — not part of this one until linked";
/// A pair the db/016 veto floor finds a disagreement in, read NOW (`OpenProposal::vetoed` in
/// cairn-node). Which facts — and whether "verified" applies — is the compare panel's to say
/// (R2b-1's rule), read fresh; never worded here from stored JSON.
pub const VETO_NOTE: &str =
    "Some recorded facts disagree between these charts — Review shows which.";
/// "Different people" after every pair between the two records was already judged or resolved
/// (Review, and Link over a banner comparison, get [`NOT_SHOWN_OR_RESOLVED`] instead).
pub const NOTHING_OPEN: &str = "this possible duplicate has already been judged or resolved — \
     nothing was done; reload the chart";
/// Review — or Link over a banner comparison — of a chart no list showed and no open proposal
/// joins to this record (any more).
pub const NOT_SHOWN_OR_RESOLVED: &str = "that chart is not in a list on screen, and is not an \
     open possible duplicate of this record — reload the chart, or search again";
/// The button's label; `key_locked_for` names it (a locked key names the button pressed).
pub const DIFFERENT_PEOPLE_BUTTON: &str = "Different people — not the same person";

/// The banner, as `med_list` hands it to the webview inside `ChartPane`.
///
/// Deliberately NO `Default`: the all-empty section is not a neutral placeholder, it is the
/// claim "checked, none open" — so it is built only by name, [`DuplicateSection::checked_none_open`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DuplicateSection {
    pub entries: Vec<DuplicateEntryView>,
    /// "N more …" when entries beyond [`MAX_SHOWN`] exist.
    pub more: Option<String>,
    /// The proposals could not be read at all.
    pub error: Option<String>,
    /// This record's check lines (pending charts, the node's status when it explains them).
    pub check_lines: Vec<String>,
}

impl DuplicateSection {
    /// The section that says nothing, which the webview hides: "this record was checked, and no
    /// possible duplicate of it is open". Only ever returned when that is what was read.
    pub fn checked_none_open() -> Self {
        DuplicateSection {
            entries: vec![],
            more: None,
            error: None,
            check_lines: vec![],
        }
    }
}

/// One possible duplicate, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DuplicateEntryView {
    /// The chart Review compares against.
    pub review_chart: String,
    pub heading: String,
    /// One line per chart of the other record (`member_line`'s text).
    pub identity_lines: Vec<String>,
    /// Unread identity, veto note.
    pub notes: Vec<String>,
    pub medications_heading: String,
    /// The other record's CURRENT lines (the compare panel's own wording, `medication_lines`).
    pub medications: Vec<String>,
    /// Its list's notes, or why it could not be read.
    pub medication_notes: Vec<String>,
}

/// One member chart's "has the check run since its identity changed?" answer.
pub struct ChartCheck {
    pub chart: Uuid,
    pub pending: Result<bool, String>,
}

/// One pair of a "Different people" judgement, as the window saw its outcome.
pub struct PairResult {
    pub low: Uuid,
    pub high: Uuid,
    pub outcome: Result<LinkEffect, ErrorView>,
}

/// One banner entry from what was read about the other record: its identity lines (one per
/// chart), whether the veto floor finds a disagreement now, and its current medications. A
/// failed read is a note, never a missing entry; an entry always names at least a chart id.
pub fn entry_view(
    review_chart: Uuid,
    vetoed: bool,
    identities: Result<Vec<MemberLine>, String>,
    meds: Result<MedListView, String>,
) -> DuplicateEntryView {
    let mut notes = vec![];
    let identity_lines = match identities {
        Ok(lines) if !lines.is_empty() => lines.into_iter().map(|l| l.text).collect(),
        Ok(_) => {
            notes.push("No name or date of birth is recorded for the other record.".into());
            vec![format!("chart {review_chart}")]
        }
        Err(e) => {
            notes.push(format!(
                "The other record's name and date of birth could not be read: {e}"
            ));
            vec![format!("chart {review_chart}")]
        }
    };
    if vetoed {
        notes.push(VETO_NOTE.into());
    }
    let (medications, medication_notes) = match meds {
        Ok(list) => medication_lines(&list),
        Err(e) => (
            vec![],
            vec![format!("Its medications could not be read here: {e}")],
        ),
    };
    DuplicateEntryView {
        review_chart: review_chart.to_string(),
        heading: HEADING.into(),
        identity_lines,
        notes,
        medications_heading: OTHER_CHART_LABEL.into(),
        medications,
        medication_notes,
    }
}

/// The other record's medications for its entry — but only when the list was read over the SAME
/// chart set the entry's identity lines name. **Pure.**
///
/// The identity lines are read over `record` under the window's lock; the medications after it
/// is released, through the read opening that chart gives, which re-derives the record. A link
/// landing in between would draw one set's names over another set's drugs. That is worded as
/// an unread list — the entry, and its Review, still stand.
pub fn medications_of(
    record: &ChartSet,
    read: Result<PatientMedicationList, String>,
) -> Result<MedListView, String> {
    let list = read?;
    if &list.charts != record {
        return Err("the other record changed while the banner was read — reload the chart".into());
    }
    Ok(cairn_gui_tab_medications::view::build_view(&list))
}

/// `entries` is `(the shown entries, how many exist)` or the read's error.
pub fn section_view(
    entries: Result<(Vec<DuplicateEntryView>, usize), String>,
    check_lines: Vec<String>,
) -> DuplicateSection {
    match entries {
        Err(e) => DuplicateSection {
            entries: vec![],
            more: None,
            error: Some(format!("Could not check for possible duplicates: {e}")),
            check_lines,
        },
        Ok((shown, total)) => {
            let hidden = total.saturating_sub(shown.len());
            let more = match hidden {
                0 => None,
                1 => Some("1 more possible duplicate of this record is not shown here.".into()),
                n => Some(format!(
                    "{n} more possible duplicates of this record are not shown here."
                )),
            };
            DuplicateSection {
                entries: shown,
                more,
                error: None,
                check_lines,
            }
        }
    }
}

/// The record's check lines. A checked record on a node that is not stalled has none.
///
/// R4's `chart_line` says "This chart: …" — ambiguous on a linked record — so only the OPENED
/// chart uses it; every other member is named. The node's status line follows a member line
/// (it says why "not yet run" is not moving) when the node is `NeverRun` or `CatchingUp`, and
/// always when it is `Stalled`. It is NEVER added for `Current`: a linked member not held on this
/// node reads pending forever (`cairn_chart_check_pending` is TRUE for a chart not held here), and
/// "not yet run" followed by "up to date" would contradict itself on every healthy node. An
/// unreadable status is always shown.
pub fn check_lines(
    opened: Uuid,
    checks: &[ChartCheck],
    members: &[MemberLine],
    status: Result<CheckState, String>,
) -> Vec<String> {
    let label = |chart: Uuid| -> String {
        if chart == opened {
            return "This chart".into();
        }
        let id = chart.to_string();
        match members.iter().find(|m| m.patient_id == id) {
            Some(m) => format!("Linked chart {} (chart {chart})", m.name),
            None => format!("Linked chart {chart}"),
        }
    };
    let mut lines = vec![];
    for c in checks {
        match &c.pending {
            Ok(false) => {}
            Ok(true) if c.chart == opened => lines.push(chart_line(true).to_string()),
            Ok(true) => lines.push(format!(
                "{}: duplicate check not yet run since its identity details last changed.",
                label(c.chart)
            )),
            Err(e) => lines.push(format!(
                "{}: duplicate check status unknown — {e}",
                label(c.chart)
            )),
        }
    }
    match status {
        Err(e) => lines.push(format!("Duplicate check status unknown: {e}")),
        Ok(state) => {
            let explains = match state {
                CheckState::Stalled { .. } => true,
                CheckState::Current { .. } => false,
                _ => !lines.is_empty(),
            };
            if explains {
                lines.push(status_line(&state));
            }
        }
    }
    lines
}

/// Fixture mode has no proposals and no worker: say so, honestly.
pub fn fixture_section() -> DuplicateSection {
    DuplicateSection {
        check_lines: vec![status_line(&CheckState::NeverRun { waiting: 0 })],
        ..DuplicateSection::checked_none_open()
    }
}

/// A failed "Different people" pair, classified exactly as a failed link or unlink is — but
/// named for what was pressed. Never "The unlink was …": the two charts were never linked, and
/// that wording would send the clinician looking for a link to undo.
pub fn different_people_error_view(e: &anyhow::Error) -> ErrorView {
    judgement_error_from(
        "\"different people\" judgement",
        cairn_gui_live::error::data_error_from(e),
    )
}

/// The outcome line for "Different people".
///
/// - Nothing judged → [`NOTHING_OPEN`].
/// - ONE pair, and it failed → that pair's own error, unchanged (so the webview applies its
///   retry advice: an outage — "not confirmed" — keeps the button, a verdict takes it).
/// - SEVERAL pairs, all failed → ONE error naming every pair's failure — never just the first,
///   which could hide a later pair's "commit outcome unknown" behind a refusal (the #713
///   hazard) — carrying the most retryable class among them (`combined_retry`).
/// - Anything recorded → one sentence per pair, and `reload`: the banner must re-read, since
///   a pair that was not confirmed may or may not still be open.
///
/// The `reload` is `true` even when every recorded pair was `Outranked` (see
/// [`LinkReportView::reload`]): whatever was recorded, the banner must be re-read.
pub fn different_people_report(results: Vec<PairResult>) -> Result<LinkReportView, ErrorView> {
    match results.as_slice() {
        // `record_different_people` says "nothing open" as `NothingOpen`, never as an empty
        // list; this arm only keeps the function total, with the same honest sentence.
        [] => return Err(refused(NOTHING_OPEN)),
        [PairResult {
            outcome: Err(e), ..
        }] => return Err(e.clone()),
        _ => {}
    }
    if results.iter().all(|r| r.outcome.is_err()) {
        return Err(ErrorView {
            text: report_sentence(&results),
            retry: combined_retry(results.iter().filter_map(|r| r.outcome.as_ref().err())),
        });
    }
    Ok(LinkReportView {
        sentence: report_sentence(&results),
        reload: true,
    })
}

/// Every pair's sentence, in order, as one line.
fn report_sentence(results: &[PairResult]) -> String {
    results
        .iter()
        .map(pair_sentence)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The retry advice for several failed pairs: the MOST retryable among them — `Now` over
/// `AfterOperator` over `Never`. **Pure.**
///
/// Why the most retryable is the cautious choice here: a commit-unknown pair is worded "not
/// confirmed" and gets `Now` (`judgement_error_from`: no refusal marker and no SQLSTATE, so
/// `data_error_from` reads an outage) — its judgement may or may not have been recorded. A
/// `Never` beside it would tell the webview the outcome was DECIDED and take the button away,
/// claiming a verdict that does not exist. Keeping the button cannot re-sign blindly: "Different
/// people" re-reads the open pairs fresh on every press, and `unlink_charts` moves a pair's
/// proposal in the same transaction as its event — so a pair whose lost commit did land is no
/// longer open, and is never signed twice; a refused pair is refused again, before anything is
/// signed. The sentence names the event, so the clinician can also look before pressing.
fn combined_retry<'a>(errors: impl Iterator<Item = &'a ErrorView>) -> Retry {
    let rank = |r: Retry| match r {
        Retry::Never => 0,
        Retry::AfterOperator => 1,
        Retry::Now => 2,
    };
    // Never empty (only called with two or more failures); were it ever, nothing was decided,
    // so the fallback keeps the button rather than claim a verdict.
    errors
        .map(|e| e.retry)
        .max_by_key(|r| rank(*r))
        .unwrap_or(Retry::Now)
}

/// `text` ending in a full stop, so pair sentences joined by a space read as sentences.
fn full_stop(text: &str) -> String {
    if text.ends_with('.') {
        text.to_string()
    } else {
        format!("{text}.")
    }
}

/// One pair's sentence. A failed pair is "Not confirmed" — never "NOT recorded" and never "it
/// stays on the banner": a commit-unknown error may have recorded it, and only the banner's
/// re-read says whether the pair is still open.
fn pair_sentence(r: &PairResult) -> String {
    let (low, high) = (r.low, r.high);
    match &r.outcome {
        Ok(LinkEffect::TookEffect) => format!(
            "Recorded: charts {low} and {high} are different people. This pair is \
             closed on this node, and on every node once the judgement syncs."
        ),
        Ok(LinkEffect::StillJoined) => format!(
            "Recorded that charts {low} and {high} are different people — but they read as one \
             record through other links, so nothing was split. The links joining them are listed \
             under \"How these charts are linked\"."
        ),
        Ok(LinkEffect::Outranked) => format!(
            "Recorded that charts {low} and {high} are different people, but a judgement already \
             standing for that pair outranks it; the charts stay as that judgement left them."
        ),
        Err(e) => full_stop(&format!(
            "Not confirmed for charts {low} and {high}: {}",
            e.text
        )),
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
