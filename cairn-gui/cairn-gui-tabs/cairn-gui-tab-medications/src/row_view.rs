//! One drug line of the med-list view: what a single row of the table says.
//!
//! Kept apart from `view.rs` so that the two questions this crate
//! answers stay in two places a reviewer can hold at once: HERE, what one line says about one
//! drug — its name, its dose, whose signature it carries, what is wrong with it; in `view.rs`,
//! what the chart as a whole says — how many threads the gesture signs, and what the chart
//! cannot show.
//!
//! The one thing a row does NOT decide for itself is whether the gesture will sign it. That
//! comes in as `targets`, the set `view::build_view` computed with ONE call to the shared
//! `sign_off_targets` rule — the rule the node's orchestrator uses — so the badge on a row and
//! the count on the button can never disagree (see the `view` module doc).
use crate::hold_view::{cessation_detail, hold_view, stopped_record_detail, HoldView};
use cairn_medication_view::{
    short_kid, withheld_reasons, MedicationRow, MedicationStatus, VouchState,
    SEPARATION_INSTRUCTION,
};
use chrono::{DateTime, Local};
use serde::Serialize;
use std::collections::HashSet;
use uuid::Uuid;

/// One rendered drug line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MedListRowView {
    /// Stable id for the DOM and for the cease, withhold and release commands.
    pub group_id: String,
    /// The drug's coded name when it has one, else the term exactly as asserted.
    pub primary: String,
    /// Dose as "500 mg", or an explicit statement that none was recorded.
    pub dose: String,
    pub formulation: String,
    pub sig: String,
    pub started: String,
    /// "current", "withheld" or "ceased" — WORDING, for the table. Code that needs to know
    /// whether a drug is current reads [`MedListRowView::current`] instead: a reworded or
    /// translated label must never change which drugs a caller counts as current (PR #707
    /// review). The one reader of the words is main.js's row styling (`data-ceased`,
    /// `data-withheld`), tied to these exact words by the tauri crate's
    /// `the_row_draws_the_w_and_its_controls`.
    pub status_label: String,
    /// True while the drug is active — the typed fact `status_label` words.
    pub current: bool,
    /// Whose signature this line carries, and whether it is out of date.
    pub vouch_label: String,
    /// True when the sign-off gesture will sign this row. Derived from the SAME
    /// `sign_off_targets` the orchestrator uses — never recomputed here.
    pub will_be_signed: bool,
    /// False for an already-ceased drug. The window narrows it further (#841): on a wrong-chart
    /// hazard line with no thread on the opened chart there is nothing Stop could write, and
    /// only the Tauri `chart_pane`, which knows the opened chart, can tell.
    pub can_cease: bool,
    /// Advisory worklist labels (duplicate suspicion, anchor conflict, wrong-chart hazard).
    pub flags: Vec<String>,
    /// Which chart(s) this drug was recorded on — the ids joined by ", " — and `Some` ONLY
    /// when the list is a combined read over linked charts (ADR-0076 decision 1: "every row
    /// names its source chart(s)"; two paper folders clipped together, and the clinician
    /// reads both). On a never-linked chart it is `None`, so that chart gains no label — the
    /// label is information only where there is a choice of folder.
    pub source: Option<String>,
    /// The holds on this line (ADR-0081), in the workstation's local time. A hold sits BESIDE
    /// the status: `status_label` words it ("withheld") but `current`, `can_cease` and
    /// `will_be_signed` do not move.
    pub holds: Vec<HoldView>,
    /// True on a current line: a "W" can be written on it. Narrowed by the window as
    /// `can_cease` is (#841), to lines `withhold_target` accepts from the opened chart.
    pub can_withhold: bool,
    /// True when there is a hold to release (a release also cancels a pending hold). Narrowed
    /// by the window on a hazard line to lines `release_plan` can write from the opened chart (#844).
    pub can_release: bool,
    /// `"Stopped: <reason>"` on a ceased line whose cessation gave a reason; on a current line,
    /// `"A stopped record of this drug gave: <reason>"` when a stopped member thread gave one
    /// (#861). The status word reveals it either way.
    pub cessation_detail: Option<String>,
}

/// Shown instead of a blank cell. Principle 4: an unrecorded dose is a recordable state,
/// and a blank would read either as "no dose" or as a rendering bug.
const DOSE_UNKNOWN: &str = "dose not recorded";

/// Build one row's view from one chart row.
///
/// `targets` is the set of THREAD ids the sign-off gesture will sign, computed once for the
/// whole chart by the caller. A row is "will be signed" when any of its member threads is in
/// it — a reconciled group is one row but several threads.
///
/// `linked` is `PatientMedicationList::charts.is_linked()`, decided once per chart by the
/// caller: whether this row must name its source chart(s) (see [`MedListRowView::source`]).
pub(crate) fn build_row(
    row: &MedicationRow,
    targets: &HashSet<Uuid>,
    linked: bool,
    now: &DateTime<Local>,
) -> MedListRowView {
    let holds: Vec<HoldView> = row.holds.iter().map(|h| hold_view(h, now)).collect();
    MedListRowView {
        group_id: row.group_id.to_string(),
        primary: row.display_name().to_string(),
        dose: match (&row.dose_amount, &row.dose_unit) {
            (Some(amount), Some(unit)) => format!("{amount} {unit}"),
            (Some(amount), None) => amount.clone(),
            _ => DOSE_UNKNOWN.to_string(),
        },
        formulation: row.formulation.clone().unwrap_or_default(),
        sig: row.sig.clone().unwrap_or_default(),
        started: row.started_value.clone().unwrap_or_default(),
        status_label: match row.status {
            MedicationStatus::Active if row.is_withheld() => "withheld".into(),
            MedicationStatus::Active => "current".into(),
            MedicationStatus::Ceased => "ceased".into(),
        },
        current: row.status == MedicationStatus::Active,
        vouch_label: vouch_label(row),
        will_be_signed: row
            .members
            .iter()
            .any(|m| targets.contains(&m.medication_id)),
        can_cease: row.status == MedicationStatus::Active,
        flags: flags(row),
        source: linked.then(|| source_label(&row.source_charts)),
        can_withhold: row.status == MedicationStatus::Active,
        can_release: !holds.is_empty(),
        holds,
        // At most one of the two is non-empty: the read fills the ceased fields only on a
        // ceased line and `stopped_member_reasons` only on a current one.
        cessation_detail: cessation_detail(
            row.cessation_reason.as_deref(),
            &row.other_cessation_reasons,
        )
        .or_else(|| stopped_record_detail(&row.stopped_member_reasons)),
    }
}

/// A linked row whose `source_charts` came back empty — reachable only through the race
/// documented on `MedicationRow::source_charts` (a group-chart read landing between two of
/// `list_patient_medications`'s several statements), never in normal operation. Rendered
/// explicitly rather than as an empty string: `Some("")` is what `linked.then(||
/// source_label(...))` used to produce here, and `main.js` treats an empty string as
/// falsy — so the row's provenance label would silently vanish from the screen with
/// nothing to say anything had gone wrong. Principle 4: acknowledged uncertainty ("we do
/// not know") must always outrank a silent gap that reads as "nothing to say".
const SOURCE_UNREAD: &str = "(chart not read)";

/// The source-chart label: every chart owning a member thread of the group, in the row's own
/// (sorted) order. ALL of them, not just one: a group spanning two charts was recorded on
/// both, and naming only the first would hide the second folder.
fn source_label(charts: &[Uuid]) -> String {
    if charts.is_empty() {
        return SOURCE_UNREAD.to_string();
    }
    charts
        .iter()
        .map(Uuid::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whose signature this line carries.
///
/// A reconciled group has several member threads, which can disagree. The honest summary
/// names the worst state rather than picking one member's — a group is not signed off
/// until every member is.
fn vouch_label(row: &MedicationRow) -> String {
    let unsigned = row
        .members
        .iter()
        .filter(|m| m.vouch == VouchState::Absent)
        .count();
    let stale: Vec<&str> = row
        .members
        .iter()
        .filter_map(|m| match &m.vouch {
            VouchState::Stale { by } => Some(by.as_str()),
            _ => None,
        })
        .collect();
    if unsigned > 0 {
        // No signature at all is the worse state of the two, so it wins the summary: a
        // group reported as "signed but out of date" reads as needing a refresh, while an
        // unsigned member has never been vouched by anyone.
        return "not signed".to_string();
    }
    if let Some(by) = stale.first() {
        return format!("signed by {} — out of date", short_kid(by));
    }
    match row.members.first().and_then(|m| m.vouch.attester()) {
        Some(by) => format!("signed by {}", short_kid(by)),
        // No members at all. Not expected from the read path, but "not signed" is the
        // honest reading of "nothing here vouches for this line".
        None => "not signed".to_string(),
    }
}

fn flags(row: &MedicationRow) -> Vec<String> {
    let mut out = Vec::new();
    if row.reconciliation_flagged {
        out.push("possible duplicate — not yet reconciled".to_string());
    }
    if row.coding_conflict {
        out.push("two different drug identities in this group".to_string());
    }
    if let Some(why) = row.hazard_reasons() {
        // Per-row, and in the row's own words, because this is where the clinician is
        // looking when they wonder why the line has no signature badge. One sentence per
        // REASON (#697): each names its own cause, and the report names each remedy.
        if why.outside_set {
            // The displayed dose comes from a whole-group pick that ignores patient, so it
            // may be the other patient's (issue #334). The remedy goes on the row, as the
            // CLI's does: the report covers only lines sign-off withholds, and `chart_pane`
            // may hide Release on this line (#844), so a signed or ceased line would otherwise
            // show a W nobody here can end and no way forward (PR #845 review). "Cannot be
            // signed" only where sign-off withholds the line (the PR #717 lesson).
            let signing = if withheld_reasons(row).is_some() {
                ", so this line cannot be signed"
            } else {
                ""
            };
            out.push(format!(
                "shared with another patient's record — the dose shown may not be this \
                 patient's{signing}. {SEPARATION_INSTRUCTION}"
            ));
        }
        if why.doubted_link {
            // #697 (b): the record's links are in doubt (db/054: an un-attested link the hard
            // identity check flagged or trips now, or a clinician's unlink between charts other
            // links still join), and this line is not on the opened chart alone — signing it
            // could vouch for a possible stranger's drug. "Cannot yet vouch that it is this
            // patient's", not "may be another person's": the line's own chart may be
            // human-linked to this one while ANOTHER link is doubted. "No longer in doubt", not
            // "judged": in the A–C–X bridge every link can carry a human judgement and the
            // record still holds a doubt. The "cannot be signed until" half only when sign-off
            // withholds the line (`withheld_reasons`): a ceased or already-signed line needs no
            // signature, and promising one later would be untrue (PR #717 review).
            let cause = "this record's links are in doubt, and this line is not recorded only \
                         on the chart you opened — the node cannot yet vouch that it is this \
                         patient's";
            out.push(if withheld_reasons(row).is_some() {
                format!("{cause}, so it cannot be signed until the record's links are no longer in doubt")
            } else {
                cause.to_string()
            });
        }
        if why.patient_conflict {
            // #836: a synced event naming another person's chart landed on this line. The
            // remedy goes on the row, chosen by status: the report beside the list covers only
            // lines sign-off withholds, so a ceased or already-signed conflict line would
            // otherwise show its warning with no way forward. "Cannot be signed" only where
            // sign-off withholds the line (the PR #717 lesson, as for the doubted link above).
            let signing = if withheld_reasons(row).is_some() {
                ", so this line cannot be signed"
            } else {
                ""
            };
            out.push(format!(
                "an event on this line was recorded for another patient's chart{signing}. {}",
                cairn_medication_view::conflict_remedy(row.status)
            ));
        }
    }
    out
}

#[cfg(test)]
#[path = "row_view_tests.rs"]
mod tests;
