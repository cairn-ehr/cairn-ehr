//! The chart SET on screen: the window's half of ADR-0076 decisions 1 and 3.
//!
//! A chart linked to other charts of the same person opens as ONE combined record (decision 1):
//! the medication list is read over every chart in the link component, and the identity header
//! lists each member's OWN line — no winner chosen, because the disagreement between two
//! duplicates is often the very typo that made them. This module turns those member identities
//! into the lines the header shows ([`member_line`]).
//!
//! And because the set can change under a clinician's eyes — a link or unlink landing while the
//! list is being reviewed — every chart command names the set that was DISPLAYED, and refuses
//! when it no longer matches (decision 3). That is PR #674's rule ("a chart command acts on the
//! chart on screen", `AppState::displayed_patient`) widened from one chart to a set, and
//! [`check_displayed_set`] is where it lives, once, as a pure function.
//!
//! # Two checks, both wanted
//!
//! The window checks the displayed set against its OWN fresh read (here), and the node's
//! sign-off orchestrator checks it again against ITS first read. They are two snapshots taken at
//! two moments: the window's is the early one, testable against `AppState::mock` with no
//! database; the orchestrator's is the authoritative one, taken inside the act itself. Neither
//! replaces the other. A CEASE gets only the first: `cease_medication` writes one thread and
//! has no notion of a chart set, so the window's check is the whole of decision 3 for it (see
//! `commands::cease_impl`).
pub mod change;

use crate::hold::plan::release_plan;
use crate::state::AppState;
use cairn_gui_tab_medications::view::{build_view, MedListView};
use cairn_medication_view::{ChartSet, MedicationRow, PatientMedicationList};
use cairn_node::patient::person::ChartIdentity;
use serde::Serialize;
use uuid::Uuid;

/// Why [`check_displayed_set`] refused. A TYPE, not two sentences compared as text: callers
/// (the link commands, the duplicates admit, sign-off, cease) must tell a CHANGED set from an
/// unreadable one by variant, so rewording a sentence can never silently change which
/// refusal a command gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartSetError {
    /// The webview's list of displayed charts could not be read (empty or unparseable). Not a
    /// change: nothing about the record moved, the window itself sent something it should not
    /// have. The way out is "Read this chart again", which redraws the set every command sends
    /// back — the same read reopening the chart would do, in one act instead of three.
    Unreadable,
    /// The set the record now resolves to differs from the one on screen. The screen is
    /// behind the record, so the right answer is a re-read (`Retry::AfterReread`), which the
    /// window starts by itself: the clinician need not press anything.
    Changed,
}

impl ChartSetError {
    /// The sentence for this refusal. Only the tests read it bare; the window reads
    /// [`ChartSetError::into_view`].
    #[cfg(test)]
    pub fn text(&self) -> &'static str {
        match self {
            Self::Unreadable => UNREADABLE,
            Self::Changed => CHANGED,
        }
    }

    /// The refusal as the funnel's error view: a change is `Retry::AfterReread` (the window
    /// re-reads the whole chart), an unreadable list a plain refusal.
    pub fn into_view(self) -> crate::funnel::view::ErrorView {
        match self {
            Self::Unreadable => crate::link::judgement_view::refused(UNREADABLE),
            Self::Changed => crate::link::judgement_view::stale(CHANGED),
        }
    }
}

/// The sentence for [`ChartSetError::Unreadable`]: a fault in what the window sent, mended by a
/// re-read (it redraws the set), so it names the button that does one. Its view is `refused`
/// (`Retry::Never`), not `stale`: nothing about the record moved, so the window must not re-read
/// by itself and say the record changed — the clinician presses the button the sentence names.
const UNREADABLE: &str = "this window could not tell which charts are on screen — nothing was \
     done; press \"Read this chart again\"";

/// The sentence for a set that changed while its list was on screen. It names no remedy: the
/// window re-reads the chart itself, so the clinician has nothing to press, and a sentence
/// sending them to "Read this chart again" would ask for a read already under way.
pub(crate) const CHANGED: &str =
    "the linked charts changed while this list was on screen — nothing was done";

/// Refuse unless the charts the webview DISPLAYED are exactly the set just read; on a match,
/// return the displayed set.
///
/// `read` is the set a fresh read of the open chart resolved; `displayed` is what the webview
/// sent back — the `charts` field of the `MedListView` it rendered. Order does not matter (both
/// sides go through `ChartSet`, which sorts and de-duplicates); membership does. An empty or
/// unparseable list is refused as "could not tell", never treated as a match: a command that
/// cannot say what it saw must not act.
///
/// Returns the DISPLAYED set rather than `()` so a caller hands the node's orchestrator what
/// the clinician actually saw, visibly at the call site — equal to `read` here, but the
/// orchestrator's own compare is then literally against the screen, not against a set a
/// reviewer has to prove equal to it.
pub fn check_displayed_set(
    read: &ChartSet,
    displayed: &[String],
) -> Result<ChartSet, ChartSetError> {
    let parsed: Vec<Uuid> = displayed
        .iter()
        .map(|id| id.parse::<Uuid>())
        .collect::<Result<_, _>>()
        .map_err(|_| ChartSetError::Unreadable)?;
    let shown = ChartSet::new(parsed).ok_or(ChartSetError::Unreadable)?;
    if &shown == read {
        Ok(shown)
    } else {
        Err(ChartSetError::Changed)
    }
}

/// The sign-off report's sentence naming the charts a combined gesture read across, or `None`
/// for a single chart.
///
/// The CLI prints the same fact after a combined sign-off (`main.rs`, `medication-sign-off`),
/// and for the same reason: "Signed 3 medication thread(s)" on a combined list could be read
/// as "recorded on the chart I opened". Each line was signed on the chart it was recorded on
/// (ADR-0076 decision 2), and the report says so. `None` for a never-linked chart, whose report
/// is unchanged.
pub fn signed_across_message(charts: &ChartSet) -> Option<String> {
    charts.is_linked().then(|| {
        format!(
            "This gesture read across {} linked charts; each line was signed on the chart it \
             was recorded on.",
            charts.members().len()
        )
    })
}

/// Which member threads a cease gesture on one displayed line writes, and which it holds back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CeasePlan {
    /// `(thread, the chart its own statement is on)` — each cessation is written to its
    /// thread's own chart (ADR-0076 decision 2).
    pub write: Vec<(Uuid, Uuid)>,
    /// Threads deliberately NOT stopped, each with the sentence the report shows for it.
    pub held_back: Vec<String>,
}

/// Plan a cease of `row` from the chart the clinician opened.
///
/// An ordinary line stops every member thread, each on its own chart. A line withheld as a
/// WRONG-CHART HAZARD (`MedicationRow::is_wrong_chart_hazard`) stops only the threads on the opened
/// chart: the node itself says this line may carry another person's drug — the group reaches
/// a chart outside the set, or the set holds a link the node doubts and the line is not
/// recorded only on the opened chart, or a synced event on it named another chart (#836) —
/// so "stop this drug" can only
/// be trusted to mean it for the patient in front of the clinician. The other threads are
/// held back and NAMED, never skipped in silence (ADR-0060 decision 2); they can be stopped
/// from their own chart. Without this, Stop on a line spanning a doubted link would write a
/// cessation onto a chart the node believes may be someone else's.
///
/// This holds back more than strictly needed when the other thread is on a linked chart of
/// the same person (a hazard line reaching OUTSIDE the set also spans the set): stopping less
/// and saying so is the safe error. A never-linked chart is unaffected — all its members are
/// on the opened chart. A doubted-set line on another member only (#697 (b)) is held back
/// entirely. Pure, so the rule is tested without a database.
pub fn cease_plan(row: &MedicationRow, opened: Uuid) -> CeasePlan {
    let mut plan = CeasePlan {
        write: vec![],
        held_back: vec![],
    };
    for m in &row.members {
        if !row.is_wrong_chart_hazard() || m.patient_id == opened {
            plan.write.push((m.medication_id, m.patient_id));
        } else {
            plan.held_back.push(format!(
                "thread {} on chart {}: NOT stopped from here — this line may carry another \
                 person's drug, so only this chart's own threads are stopped; stop it from its \
                 own chart",
                m.medication_id, m.patient_id
            ));
        }
    }
    plan
}

/// One member chart's line under the identity header of a combined record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemberLine {
    /// The member chart's id — the same id a row's source label names.
    pub patient_id: String,
    /// The display name as the line shows it — the name, or its worded absence — so a link line
    /// can name both charts without a second read.
    pub name: String,
    /// `born <date>`, or its worded absence — with `name`, what a banner entry's heading
    /// names the record by (#737), again without a second read.
    pub born: String,
    /// The whole line as the clinician reads it; see [`member_line`].
    pub text: String,
}

/// Render one member's own identity line.
///
/// Every absence is NAMED rather than left blank (principle 4): a header reading "born " with
/// nothing after it is indistinguishable from a rendering fault, and "(no name recorded)" is a
/// recordable state the clinician needs to see — the wording matches the single-chart header's
/// (`funnel::view`). The chart id is always last and always whole: it is what ties this line to
/// the source label on each medication row.
///
/// A chart whose registration this node does not HOLD (`ChartIdentity::held` — a link naming
/// a chart that has not fully arrived here) is worded differently: an absent name or date is
/// then UNKNOWN, not "not recorded" — nothing says it was never recorded, only that it has not
/// arrived — and the line says the registration is missing. A name or date that did arrive
/// is still shown.
pub fn member_line(identity: &ChartIdentity) -> MemberLine {
    let (no_name, no_dob) = if identity.held {
        ("(no name recorded)", "date of birth not recorded")
    } else {
        ("(name unknown)", "date of birth unknown")
    };
    let name = identity.name.as_deref().unwrap_or(no_name);
    let born = identity
        .birth_date
        .as_deref()
        .map(|d| format!("born {d}"))
        .unwrap_or_else(|| no_dob.to_string());
    let not_held = if identity.held {
        ""
    } else {
        " · registration not yet received on this node"
    };
    MemberLine {
        patient_id: identity.patient_id.to_string(),
        name: name.to_string(),
        born: born.clone(),
        text: format!(
            "{name} · {born} · identity {}{not_held} · chart {}",
            identity.trust, identity.patient_id
        ),
    }
}

/// What `med_list` hands the webview: the list, and — when the chart is linked to others — one
/// identity line per member chart, shown under the identity header (ADR-0076 decision 1).
#[derive(Debug, Serialize)]
pub struct ChartPane {
    pub list: MedListView,
    /// Empty unless the list is a combined read over linked charts whose names were read.
    pub members: Vec<MemberLine>,
    /// Set when the chart is linked but its member identities could NOT be read. The list is
    /// still shown — a clinician must always be able to read (availability over consistency) —
    /// and this says, where the member lines would have been, that the list is combined and
    /// what is missing: the names, dates of birth AND identity states, all three (they are read
    /// together). The last matters most — a member `under-review` is exactly what should stop a
    /// clinician trusting the combination, and its line is gone. Without this warning a combined
    /// list would read as a single chart.
    pub members_error: Option<String>,
    /// The standing links joining the record's charts, one line each (R2b-2), then one line per
    /// bridged "different people" judgement (R6) — empty for a chart linked to nothing.
    pub links: Vec<crate::link::record_links::RecordLinkView>,
    /// Set when those links could not be read (the list is then empty), or when only the
    /// "unverified here" notes could not be (the lines show, without notes); the member lines
    /// still show either way.
    pub links_error: Option<String>,
    /// The possible-duplicate banner (R5a); empty and hidden when checked and none open.
    pub duplicates: crate::duplicates::view::DuplicateSection,
    /// On a re-read only: the one sentence naming the charts that joined or left the record
    /// since the set the window had on screen (#797, [`change::set_change_sentence`]). `None`
    /// when nothing changed or the window sent no set to compare.
    pub set_change: Option<String>,
}

/// The row sentence on a hazard line whose Withhold and Stop [`chart_pane`] did not draw (#841):
/// the remedy the refused press used to give, so the way forward is not hidden with the buttons.
pub(crate) const NOT_ON_OPENED_CHART: &str = "this line has no record on the chart you opened — \
     stop or withhold it from its own chart";

/// The row sentence on a hazard line whose Release [`chart_pane`] did not draw (#844): every hold
/// on it sits on another chart's thread. Its own sentence, because Withhold and Stop may still
/// stand (the line has a thread here), so [`NOT_ON_OPENED_CHART`] would be false.
pub(crate) const HOLD_NOT_ON_OPENED_CHART: &str = "this line's hold is not on the chart you \
     opened — release it from its own chart";

/// Assemble the pane from one chart read and the outcome of reading its member names.
///
/// Pure, so the availability rule is tested with no database: whatever happened to the member
/// read, the list itself always goes to the screen. A failure becomes `members_error`, worded to
/// say the list is combined, over how many charts, and that each member's identity state is
/// unknown — the facts the missing header lines would have conveyed.
///
/// `opened` is the chart the clinician opened: Withhold, Stop and Release are drawn only on a
/// line their command could write from there (#841, #844).
pub fn chart_pane(
    list: &PatientMedicationList,
    opened: Uuid,
    members: Result<Vec<MemberLine>, String>,
    links: Result<crate::link::record_links::RecordLinks, String>,
    duplicates: crate::duplicates::view::DuplicateSection,
    set_change: Option<String>,
) -> ChartPane {
    let (members, members_error) = match members {
        Ok(members) => (members, None),
        Err(e) => (
            vec![],
            Some(format!(
                "The linked charts' names, dates of birth and identity states could not be read \
                 — this list covers {} charts, and whether any of them is under review is \
                 unknown: {e}",
                list.charts.members().len()
            )),
        ),
    };
    let (links, links_error) =
        crate::link::record_links::links_section(links, &members, list.charts.is_linked());
    let mut view = build_view(list);
    // #841: a hazard line with no thread on the opened chart can only refuse a Withhold and can
    // stop nothing (a cease holds every thread back), so neither is offered — each asked by its
    // command's own rule, so they cannot drift. Hazard lines only: on an ordinary line a refusal
    // is an anomaly (its own thread missing from its members), and the press names it and its
    // remedy where a hidden button would say nothing. `build_view` keeps `list.rows`' order, so
    // the rows zip one to one.
    for (shown, row) in view.rows.iter_mut().zip(&list.rows) {
        debug_assert_eq!(shown.group_id, row.group_id.to_string());
        if !row.is_wrong_chart_hazard() {
            continue;
        }
        let offered = (shown.can_withhold, shown.can_cease);
        shown.can_withhold &= crate::hold::plan::withhold_target(row, opened).is_ok();
        shown.can_cease &= !cease_plan(row, opened).write.is_empty();
        // The refused press used to say where to go; with the buttons gone, the line says it.
        if offered != (shown.can_withhold, shown.can_cease) {
            shown.flags.push(NOT_ON_OPENED_CHART.to_string());
        }
        // #844: the window releases every hold the row drew, so ask the plan with all of them.
        let drawn: Vec<Uuid> = row.holds.iter().map(|h| h.withhold_event_id).collect();
        if shown.can_release && release_plan(row, &drawn, opened).write.is_empty() {
            shown.can_release = false;
            // Only when every hold is on one of the line's own threads. A hold on a NON-member
            // thread sits on a chart outside the record (a mis-reconciled group), so it may be
            // another person's own W: "release it from its own chart" would send the clinician
            // to end it. The row's separation remedy speaks for that line instead.
            let on_members = row.holds.iter().all(|h| {
                row.members
                    .iter()
                    .any(|m| m.medication_id == h.medication_id)
            });
            if on_members {
                shown.flags.push(HOLD_NOT_ON_OPENED_CHART.to_string());
            }
        }
    }
    ChartPane {
        list: view,
        members,
        members_error,
        links,
        links_error,
        duplicates,
        set_change,
    }
}

/// The member lines for a chart set, or none when it is not linked.
///
/// None for a single chart: its identity is already the header, and a one-line "linked charts"
/// list would claim a link that does not exist. A LINKED set with no database to read from
/// (fixture mode — unreachable today, since `AppState::mock` serves single-chart fixtures only)
/// is an error, not an empty list, for the reason that follows. A failed read is
/// an ERROR, not an empty list: a combined record whose header silently lost its member lines
/// would read as a single chart while its rows came from several. The caller
/// ([`chart_pane`]) turns that error into a warning beside the list, never a failed open.
pub async fn linked_members(
    state: &AppState,
    charts: &ChartSet,
) -> Result<Vec<MemberLine>, String> {
    if !charts.is_linked() {
        return Ok(vec![]);
    }
    chart_identities(state, charts.members()).await
}

/// Each named chart's own identity line, read BY ID — the reader [`linked_members`] uses for
/// the header, and the re-read uses for the charts that LEFT the record (#797), which are no
/// longer part of any set this chart reads as.
///
/// By id, not by link component: `cairn_node::patient::person::chart_identities` reads each
/// chart's held state, name, date of birth and trust independently, so a chart that is no
/// longer linked to anything here — or whose registration never arrived (`held` false) — is
/// still read, and worded by [`member_line`]. No ids, no read. No database (fixture mode) is an
/// error: the caller words each chart as unread rather than guessing a line.
pub async fn chart_identities(state: &AppState, ids: &[Uuid]) -> Result<Vec<MemberLine>, String> {
    let Some(charts) = ChartSet::new(ids.iter().copied()) else {
        return Ok(vec![]);
    };
    let Some(db) = state.db.as_ref() else {
        return Err("there is no database to read the linked charts' identities from".into());
    };
    let db = db.lock().await.map_err(|e| e.to_string())?;
    let identities = cairn_node::patient::person::chart_identities(&*db, &charts)
        .await
        .map_err(|e| format!("{e:#}"))?;
    Ok(identities.iter().map(member_line).collect())
}

/// The re-read's "what changed" sentence (#797): compare the set the window had on screen
/// (`seen_charts`, as the webview sent it) with the set just read (`now`), and word the
/// difference with [`change::set_change_sentence`].
///
/// `members` are the pane's own member lines (read over `now`), reused for the charts that
/// JOINED; only the charts that LEFT are read here, by id ([`chart_identities`]), and a failed
/// read words each of them unread ([`change::DepartedLine::Unread`]) — the change is still
/// said, never dropped. Nothing is read when there is nothing to compare ([`change::seen_set`])
/// or nothing left.
pub async fn set_change(
    state: &AppState,
    seen_charts: Option<&[String]>,
    opened: Uuid,
    now: &ChartSet,
    members: &[MemberLine],
) -> Option<String> {
    let seen = change::seen_set(seen_charts, opened)?;
    let gone = change::departed_ids(&seen, now);
    let departed: Vec<change::DepartedLine> = if gone.is_empty() {
        vec![]
    } else {
        match chart_identities(state, &gone).await {
            Ok(lines) => lines.into_iter().map(change::DepartedLine::Read).collect(),
            Err(_) => gone.into_iter().map(change::DepartedLine::Unread).collect(),
        }
    };
    change::set_change_sentence(&seen, now, members, &departed)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// Every chart command actually CALLS the set check (ADR-0076 decision 3).
///
/// Declared beside the check (not in `commands.rs`) so a reader of `check_displayed_set` finds
/// the pins that every command calls it: the two writing commands refuse a changed set and the
/// read hands the webview the set to send back. A refactor that dropped the check from one
/// command would otherwise stay green while signing a list nobody reviewed.
#[cfg(test)]
#[path = "command_tests.rs"]
mod command_tests;
