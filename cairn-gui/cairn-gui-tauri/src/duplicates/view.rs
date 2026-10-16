//! Every sentence the possible-duplicate banner shows, as pure functions (repair path R5a,
//! #680; design page "R5a — the banner, designed 2026-10-06").
//!
//! The banner is AMBIENT (§5.12): it never takes focus, never re-pops, and is never a modal.
//! What it may never do is be ABSENT when something is unknown — an empty section must only
//! ever mean "checked, none open". So every failed read below becomes a worded line, and
//! [`check_lines`] says when this record's check has not run, reusing R4's own sentences.
use crate::chart_set::MemberLine;
use crate::link::view::medication_lines;
use cairn_gui_tab_medications::view::MedListView;
use cairn_medication_view::{ChartSet, PatientMedicationList};
use cairn_node::duplicate_check::{chart_line, status_line, CheckState};
use serde::Serialize;
use uuid::Uuid;

/// At most this many entries are drawn (each costs a medication read at chart open); the rest
/// are counted in `DuplicateSection::more`, never dropped silently.
pub const MAX_SHOWN: usize = 3;
/// An entry's status line, drawn under its heading — on the banner and in the tray
/// (`worklist/view.rs`) alike; each heading names whose record(s) it is (#737, #796).
pub const NOT_YET_REVIEWED: &str = "Possible duplicate — not yet reviewed";
/// Above the other record's lines — read over EVERY chart of that record, so "record", not
/// "chart": they are someone else's until a human links the two. The compare panel's own
/// heading, word for word (index.html `#link-other-meds-heading`).
pub const OTHER_CHART_LABEL: &str = "On the other record — not part of this one until linked";
/// A pair the db/016 veto floor finds a disagreement in, read NOW (`OpenProposal::vetoed` in
/// cairn-node). Which facts — and whether "verified" applies — is the compare panel's to say
/// (R2b-1's rule), read fresh; never worded here from stored JSON.
pub const VETO_NOTE: &str =
    "Some recorded facts disagree between these charts — Review shows which.";
/// An accepted pair's status (#736): a human already said "same person" through C2, and the
/// link has not been applied yet. "Not yet reviewed" would be false.
pub const ACCEPTED_NOT_LINKED: &str = "Accepted as the same person — not yet linked";
/// Another writer's un-attested unlink stands for the pair (ADR-0078). Shown, never hidden: the
/// disagreement is exactly what the human should see (principle 4).
pub const DISPUTED_NOTE: &str =
    "Recorded as not the same person, without a clinician's confirmation on record here.";
/// "Different people" on a pair a human already accepted as the same person (#736).
pub const ACCEPTED_NOT_OVERRULED: &str = "a clinician has already accepted these as the same \
     person, and that judgement is not overruled from here — nothing was done";

/// What the node read says about one entry's pairs. `Default` (all false) is the plain
/// "possible duplicate, not yet reviewed" entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EntryFlags {
    pub vetoed: bool,
    pub accepted: bool,
    pub disputed: bool,
}

/// "Different people" after every pair between the two records was already judged or resolved
/// (Review, and Link over a banner comparison, get [`NOT_SHOWN_OR_RESOLVED`] instead). No "reload
/// the chart": the window has no reload control, and re-reads the BANNER itself after a decided
/// refusal (#745 point 3; link.js's `sendJudgement` and `compare`, via `redrawBannerAlone`) — never
/// the whole chart, which the refusal does not say is stale.
pub const NOTHING_OPEN: &str = "this possible duplicate has already been judged or resolved — \
     nothing was done";
/// Review — or Link over a banner comparison — of a chart no list showed and no open proposal
/// joins to this record (any more). The window re-reads the banner itself, as for [`NOTHING_OPEN`].
pub const NOT_SHOWN_OR_RESOLVED: &str = "that chart is not in a list on screen, and is not an \
     open possible duplicate of this record — nothing was done; to compare it, search for it";
/// The button's label; `key_locked_for` names it (a locked key names the button pressed).
pub const DIFFERENT_PEOPLE_BUTTON: &str = "Different people — not the same person";

/// The banner, as `med_list` hands it to the webview inside `ChartPane`.
///
/// Deliberately NO `Default`: the all-empty section is not a neutral placeholder, it is the
/// claim "checked, none open" — so it is built only by name, [`DuplicateSection::checked_none_open`].
///
/// Serialized by hand (below) so that it also sends `silent` — [`DuplicateSection::is_silent`] —
/// and the webview hides the banner on that one flag rather than re-deriving it (#745).
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// True only when the section says nothing at all — the one case the webview hides it, which
    /// means "checked, none open". The destructuring is EXHAUSTIVE on purpose: a field added to
    /// this struct does not compile here until someone decides whether it un-silences the banner
    /// (#745 point 5 — the rule used to be a hand-written conjunction in duplicates.js).
    pub fn is_silent(&self) -> bool {
        let DuplicateSection {
            entries,
            more,
            error,
            check_lines,
        } = self;
        entries.is_empty() && more.is_none() && error.is_none() && check_lines.is_empty()
    }

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

impl Serialize for DuplicateSection {
    /// The four fields as they are, plus `silent`. Exhaustive like `is_silent`, so a new field
    /// cannot be left off the wire unnoticed.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let DuplicateSection {
            entries,
            more,
            error,
            check_lines,
        } = self;
        let mut wire = serializer.serialize_struct("DuplicateSection", 5)?;
        wire.serialize_field("entries", entries)?;
        wire.serialize_field("more", more)?;
        wire.serialize_field("error", error)?;
        wire.serialize_field("check_lines", check_lines)?;
        wire.serialize_field("silent", &self.is_silent())?;
        wire.end()
    }
}

/// One possible duplicate, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DuplicateEntryView {
    /// The chart Review compares against.
    pub review_chart: String,
    /// Whose record this entry is about, for its `<h3>` — see `entry_heading` (#737).
    pub heading: String,
    /// "Not yet reviewed", or accepted as the same person (#736); the line under the heading.
    pub status: String,
    /// One line per chart of the other record (`member_line`'s text).
    pub identity_lines: Vec<String>,
    /// Unread identity, veto note, dispute note (an un-attested unlink stands, ADR-0078).
    pub notes: Vec<String>,
    /// `false` for an accepted pair: "Different people" would silently overrule an earlier
    /// human (#736). The backend refuses it anyway (`AcceptedAsSame`); this only hides the button.
    pub offers_different_people: bool,
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

/// One banner entry from what was read about the other record: its identity lines (one per
/// chart), its `flags` — the veto floor finds a disagreement now (a note), an un-attested unlink
/// stands (the dispute note), a human already accepted the pair (the accepted status, and no
/// "Different people") — and its current medications. A failed read is a note, never a missing
/// entry; an entry always names at least a chart id.
pub fn entry_view(
    review_chart: Uuid,
    flags: EntryFlags,
    identities: Result<Vec<MemberLine>, String>,
    meds: Result<MedListView, String>,
) -> DuplicateEntryView {
    let mut notes = vec![];
    let heading = entry_heading(review_chart, identities.as_deref().unwrap_or_default());
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
    if flags.vetoed {
        notes.push(VETO_NOTE.into());
    }
    if flags.disputed {
        notes.push(DISPUTED_NOTE.into());
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
        heading,
        // An accepted pair is worded by its status, not as unreviewed (#736).
        status: if flags.accepted {
            ACCEPTED_NOT_LINKED
        } else {
            NOT_YET_REVIEWED
        }
        .into(),
        identity_lines,
        notes,
        offers_different_people: !flags.accepted,
        medications_heading: OTHER_CHART_LABEL.into(),
        medications,
        medication_notes,
    }
}

/// Whose record an entry is about, for its `<h3>`. **Pure.**
///
/// The heading exists to tell entries apart in a screen reader's heading list (#737). It is the
/// first chart's name and date of birth (what the clinician reads) and its WHOLE chart id, then
/// how many more charts the record holds (each still has its own identity line). The id is not
/// decoration: an exact name-and-birth-date match is the matcher's strongest proposal, so two
/// entries on one banner can share both, and only the chart tells them apart — as `member_line`
/// always ends with it. "First" is the lowest id (`ChartSet` sorts), so a re-render of the same
/// record gives the same heading. With no identity to name (unread, or none recorded) it is the
/// chart id alone; the entry's note says why.
fn entry_heading(review_chart: Uuid, lines: &[MemberLine]) -> String {
    let Some(first) = lines.first() else {
        return format!("Chart {review_chart}");
    };
    with_linked_count(
        chart_who(&first.name, &first.born, &first.patient_id),
        lines.len(),
    )
}

/// One chart as a heading names it: name, a second fact (the banner's date of birth, the tray's
/// age), and the WHOLE chart id. **Pure.** Shared by the banner's heading and the tray's
/// (`worklist/view.rs`), so the two cannot drift into different shapes.
pub(crate) fn chart_who(name: &str, fact: &str, chart: &str) -> String {
    format!("{name} · {fact} · chart {chart}")
}

/// `who` (a record's first chart), then how many more charts the record holds. **Pure.** Shared
/// by the banner's heading and the tray's (`worklist/view.rs`), so both count the same way.
pub(crate) fn with_linked_count(who: String, charts: usize) -> String {
    match charts.saturating_sub(1) {
        0 => who,
        1 => format!("{who} · and 1 linked chart"),
        n => format!("{who} · and {n} linked charts"),
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

/// The banner when even the record's chart set could not be read (the banner-alone read, #745):
/// its error line, never a silent section — hidden, it would claim "checked, none open". **Pure.**
pub fn unread_set_section(error: String) -> DuplicateSection {
    section_view(
        Err(format!(
            "which charts this record combines could not be read: {error}"
        )),
        vec![],
    )
}

/// A banner read on its own (#745) while the record's member names could not be read. A check
/// line then names a linked chart only by its number, so one more line says why; with no check
/// line there is nothing to explain, and a silent section stays silent. (A chart open shows the
/// same failure as the pane's own linked-charts warning; the banner alone has no pane.) **Pure.**
pub fn note_unread_names(mut section: DuplicateSection, error: &str) -> DuplicateSection {
    if !section.check_lines.is_empty() {
        section.check_lines.push(format!(
            "The linked charts' names could not be read, so a linked chart above may be named \
             only by its number: {error}"
        ));
    }
    section
}

/// The record's check lines. A checked record has none, unless the node is `Stalled`, or is
/// `CatchingUp` while a change notice waits (#742, below).
///
/// R4's `chart_line` says "This chart: …" — ambiguous on a linked record — so only the OPENED
/// chart uses it; every other member is named. The node's status line follows a member line
/// (it says why "not yet run" is not moving) when the node is `NeverRun` or `CatchingUp`, and
/// always when it is `Stalled`. It is NEVER added for `Current`: a linked member not held on this
/// node reads pending forever (`cairn_chart_check_pending` is TRUE for a chart not held here), and
/// "not yet run" followed by "up to date" would contradict itself on every healthy node. An
/// unreadable status is always shown.
///
/// `CatchingUp` also shows it on a fully checked record while a CHANGE notice waits (#742, R7):
/// a pair is proposed only by a check that runs once both charts' evidence is in place, so a
/// duplicate registered (or edited) after this record's check is proposed only when ITS own check
/// runs, and without the line "no banner" would mean "not yet known". A change notice for one of
/// THIS record's charts already shows that chart's pending line, so the node-wide flag adds the
/// line only for changes elsewhere. A config-only re-check does not: it was queued before this
/// record's check ran, so that check saw it. Known over-warns, open: a bulk `change` backlog (a
/// rebuild, a restore, a first pull; #790) and a chart whose check keeps failing (#791) both put
/// the line on every checked record — the first for one sweep, the second until it is fixed.
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
                CheckState::CatchingUp { change_waiting, .. } => {
                    change_waiting || !lines.is_empty()
                }
                CheckState::NeverRun { .. } => !lines.is_empty(),
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

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "view_catchup_tests.rs"]
mod catchup_tests;
