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
use crate::hold_view::{
    cessation_detail, hold_view, local_instant, stopped_record_detail, HoldView,
};
use cairn_medication_view::{
    short_kid, withheld_reasons, MedicationRow, MedicationStatus, VouchState,
    SEPARATION_INSTRUCTION,
};
use chrono::{DateTime, Local, TimeZone, Utc};
use serde::Serialize;
use std::collections::HashSet;
use std::fmt::Display;
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
    /// The list's dose cell: "500 mg — 500mg tds", the directions alone with no structured dose,
    /// or "dose not recorded" with neither (rulings R21/R25). `dose` stays the dose alone, for
    /// readers that word the sig themselves.
    pub dose_cell: String,
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
    /// The allergy check's words on this line (allergy slice 3, M6): one per allergen that meets
    /// it, most severe first; the window draws the loud "Allergy" tag before them. Empty here:
    /// only the window, which reads the allergies, fills it (`chart_pane`).
    pub allergy_marks: Vec<String>,
    /// Previous adverse effects that meet this line, worded; drawn muted. Filled as `allergy_marks`.
    pub allergy_notes: Vec<String>,
    /// The small "not checked" tag: some live allergy could not be verified against this line
    /// (never set beside a mark). Filled as `allergy_marks`.
    pub allergy_unverified: bool,
    /// The set-aside versions beside this line (batch charting slice 2a), each worded in Rust
    /// ([`version_line`]); the window draws them collapsed under the line. Empty here: only the
    /// window, which reads them, fills it ([`versions_under`]).
    pub versions: Vec<VersionView>,
    /// On a line that restored a set-aside version (slice 2b): the line it replaced, worded
    /// ([`settle_words`]'s `Replaced`), so the stopped line is never silently gone. Filled by the
    /// window, as `versions`.
    pub replaced: Vec<String>,
}

/// One set-aside version as the line shows it, collapsed: never a current medication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionView {
    pub line: String,
    /// The version's id: what "Use this version instead" and "Checked: not current" send.
    pub version_id: String,
    /// How it was settled, oldest first, each in [`settle_words`]' words; empty = never settled.
    pub settled: Vec<String>,
    /// Still to check: never checked nor restored, beside a CURRENT line ([`versions_under`]).
    /// The summary's "{u} to check" counts these; it is a Rust answer, never a JS count of words.
    pub open: bool,
    /// The two buttons are offered: the version is on the OPENED chart (never a linked chart's,
    /// spec §2) and not restored. A checked version keeps them: a check locks nothing.
    pub can_settle: bool,
    /// A restore line was charted from this version.
    pub restored: bool,
}

/// `{who}` when the signer's name cannot be read as a person's (Global Constraints' copy).
pub const SIGNER_NOT_KNOWN: &str = "signer not known";

/// What one settle line says (slice 2b): a check or a restore of a version, or — under the new
/// line — the line a restore replaced, with its twin spelling.
#[derive(Debug, Clone, Copy)]
pub enum SettleLine<'a> {
    Checked {
        source: &'a str,
        why: Option<&'a str>,
    },
    Restored,
    Replaced {
        line: &'a str,
    },
}

/// Shown instead of a blank cell. Principle 4: an unrecorded dose is a recordable state,
/// and a blank would read either as "no dose" or as a rendering bug.
const DOSE_UNKNOWN: &str = "dose not recorded";

/// The list's dose cell from the structured dose (`None` when none was recorded) and the
/// directions as typed: "500 mg — 500mg tds"; the directions alone when there is no structured
/// dose (the parse fills a dose only when certain, so "1g IV stat" may have none, and "dose not
/// recorded — 1g IV stat" would read as if no dose was given); [`DOSE_UNKNOWN`] with neither.
/// Pure.
fn dose_cell(dose: Option<&str>, sig: &str) -> String {
    let sig = sig.trim();
    match (dose, sig.is_empty()) {
        (Some(d), false) => format!("{d} — {sig}"),
        (Some(d), true) => d.to_string(),
        (None, false) => sig.to_string(),
        (None, true) => DOSE_UNKNOWN.to_string(),
    }
}

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
    let dose = match (&row.dose_amount, &row.dose_unit) {
        (Some(amount), Some(unit)) => Some(format!("{amount} {unit}")),
        (Some(amount), None) => Some(amount.clone()),
        _ => None,
    };
    MedListRowView {
        group_id: row.group_id.to_string(),
        primary: row.display_name().to_string(),
        dose_cell: dose_cell(dose.as_deref(), row.sig.as_deref().unwrap_or_default()),
        dose: dose.unwrap_or_else(|| DOSE_UNKNOWN.to_string()),
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
        // At most one of the two sources is non-empty: the read fills the ceased fields only on
        // a ceased line and `stopped_member_reasons` only on a current one. Were both ever
        // filled, the ceased wording would win.
        cessation_detail: cessation_detail(
            row.cessation_reason.as_deref(),
            &row.other_cessation_reasons,
        )
        .or_else(|| stopped_record_detail(&row.stopped_member_reasons)),
        allergy_marks: vec![],
        allergy_notes: vec![],
        allergy_unverified: false,
        versions: vec![],
        replaced: vec![],
    }
}

/// One collapsed version (Global Constraints' copy): `"{source}: {drug line} — set aside, not
/// current"`, then `" — {why}"` when a why was given. `drug_line` is the set-aside's own twin
/// spelling (`drug_line_twin`, ruling R11), used as is. Pure.
pub fn version_line(source: &str, drug_line: &str, why: Option<&str>) -> String {
    let line = format!("{source}: {drug_line} — set aside, not current");
    match why {
        Some(why) => format!("{line} — {why}"),
        None => line,
    }
}

/// One settle line (Global Constraints' copy), signed by `who` (`None`: [`SIGNER_NOT_KNOWN`]) at
/// `at`, shown in `now`'s zone as the holds are (`local_instant`). Pure.
pub fn settle_words<Tz: TimeZone>(
    line: SettleLine<'_>,
    who: Option<&str>,
    at: &DateTime<Utc>,
    now: &DateTime<Tz>,
) -> String
where
    Tz::Offset: Display,
{
    let who = who.unwrap_or(SIGNER_NOT_KNOWN);
    let when = local_instant(&at.to_rfc3339(), now);
    match line {
        SettleLine::Checked { source, why } => {
            let said = format!("checked by {who}, {when}: not current — from {source}");
            match why {
                Some(why) => format!("{said} — {why}"),
                None => said,
            }
        }
        SettleLine::Restored => format!("restored by {who}, {when}"),
        SettleLine::Replaced { line } => format!("replaced: {line} — {who}, {when}"),
    }
}

/// The items `(thread, item)` whose thread is one of `row`'s member threads, in the order given:
/// a version under the line it was set aside beside, a "replaced:" under the restore's own line —
/// never another line's, though it be the same drug. Pure.
pub fn under_line<T: Clone>(row: &MedicationRow, items: &[(Uuid, T)]) -> Vec<T> {
    items
        .iter()
        .filter(|(to, _)| row.members.iter().any(|m| m.medication_id == *to))
        .map(|(_, item)| item.clone())
        .collect()
}

/// The versions (`(alternative_to, its view)`) under `row` ([`under_line`]), each `open` only
/// when nothing settled it and the line is current: a stopped line's versions have nothing left
/// to compete with (ruling R3), so they are not "to check", though they keep their buttons. Pure.
pub fn versions_under(row: &MedicationRow, versions: &[(Uuid, VersionView)]) -> Vec<VersionView> {
    let current = row.status == MedicationStatus::Active;
    under_line(row, versions)
        .into_iter()
        .map(|v| VersionView {
            open: current && !v.restored && v.settled.is_empty(),
            ..v
        })
        .collect()
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

#[cfg(test)]
#[path = "row_view_versions_tests.rs"]
mod versions_tests;
