//! What changed in the record while it was on screen (#797), as ONE sentence worded in Rust.
//!
//! A record is a set of linked charts of one person (ADR-0076 decision 1). A colleague's link
//! or unlink can change that set while a clinician holds the chart. When the window re-reads
//! (`commands::med_list_impl`, after a stale refusal or the clinician's own act), it sends back
//! the set it had on screen, and this module compares that set with the one just read.
//!
//! # When anything is said (the maintainer's rule)
//!
//! "The chart I hold in my hand … I need only to be informed of what changed between me taking
//! the chart and finishing amends that I cannot know." So:
//!
//! - nothing is said when the sets are equal, or when the window sent no usable set — an
//!   absent, empty or unparseable list, or one that is not this record's at all
//!   ([`seen_set`]). A guessed change is worse than none;
//! - the clinician's OWN Link / Unlink / Confirm is never announced back to them: the webview
//!   sends its HELD set as the seen set — what it last drew, moved on by the clinician's own
//!   acts using each report's `charts_after`, read inside the act's own transaction
//!   (`chart_stale.js`'s `applyOwnAct`) — so only a colleague's change differs from it.
//!
//! The sentence names charts JOINING or LEAVING the record, by each chart's own identity line
//! (`MemberLine::text`), and nothing else: it is never handed the medication list, so a drug
//! name cannot reach it. A colleague's concurrent change to a medication line is shown by the
//! re-read but not named here (that belongs with #804's differences-only view).
//!
//! Everything here is pure, so the rule is tested with no database; the one read it needs
//! (the identity of a chart that LEFT, which the pane no longer carries) is
//! `chart_set::chart_identities`, called by the command.
use super::MemberLine;
use cairn_medication_view::ChartSet;
use uuid::Uuid;

/// The identity of one chart that was on screen and is no longer part of the record.
///
/// A chart that left is not in the pane's member lines (those are read over the NEW set), so
/// its line is read separately — and that read can fail. A failed read is `Unread`, never a
/// dropped chart: a sentence that silently omitted a departed chart would understate what left.
/// It keeps the read's error, so an outage reads as one (#881).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepartedLine {
    /// Its own identity line, read by id.
    Read(MemberLine),
    /// Its identity could not be read here, and why; the sentence names it by id.
    Unread(Uuid, String),
}

/// The set the window says it had on screen, or `None` when there is nothing honest to compare.
///
/// `None` when the window sent no set (`seen_charts` absent: the first read of a chart, or a
/// caller that does not compare), an empty one, or one with an unparseable id — "could not
/// tell" is never treated as "changed". Also `None` when the set does not hold `opened`, the
/// chart being read: every set the window drew for this chart held it (a record always
/// contains its own chart), so such a set was another record's screen — the window moved to a
/// different chart — and comparing it would announce a whole different patient as "changed".
pub fn seen_set(seen_charts: Option<&[String]>, opened: Uuid) -> Option<ChartSet> {
    let ids: Vec<Uuid> = seen_charts?
        .iter()
        .map(|id| id.parse::<Uuid>())
        .collect::<Result<_, _>>()
        .ok()?;
    ChartSet::new(ids).filter(|set| set.contains(&opened))
}

/// The charts in `seen` that `now` no longer holds, in `ChartSet` order — the ids whose
/// identity the caller must read separately (see [`DepartedLine`]).
pub fn departed_ids(seen: &ChartSet, now: &ChartSet) -> Vec<Uuid> {
    seen.members()
        .iter()
        .filter(|id| !now.contains(id))
        .copied()
        .collect()
}

/// The charts in `now` that `seen` did not hold, in `ChartSet` order.
fn joined_ids(seen: &ChartSet, now: &ChartSet) -> Vec<Uuid> {
    departed_ids(now, seen)
}

/// How a chart whose identity line is not available is named: by its whole id, saying why.
fn unread_line(id: Uuid) -> String {
    format!("chart {id} (its name could not be read here)")
}

/// The line for a joined chart: its own member line from `lines`, else worded unread.
fn joined_text(id: Uuid, lines: &[MemberLine]) -> String {
    let wanted = id.to_string();
    lines
        .iter()
        .find(|line| line.patient_id == wanted)
        .map(|line| line.text.clone())
        .unwrap_or_else(|| unread_line(id))
}

/// The line for a departed chart: its `Read` line from `lines`, else worded unread (an
/// explicit `Unread`, or no entry at all — both mean its name was not read). An `Unread`'s
/// error is said once for all of them, by [`unread_reasons`].
fn departed_text(id: Uuid, lines: &[DepartedLine]) -> String {
    let wanted = id.to_string();
    lines
        .iter()
        .find_map(|line| match line {
            DepartedLine::Read(member) if member.patient_id == wanted => Some(member.text.clone()),
            _ => None,
        })
        .unwrap_or_else(|| unread_line(id))
}

/// Why the departed charts' names could not be read, each distinct error once, in `lines` order
/// (#881): one failed read leaves every departed chart unread for the same reason, and saying
/// it inside each chart's line would repeat it. Only charts in `gone` count.
fn unread_reasons(gone: &[Uuid], lines: &[DepartedLine]) -> Vec<String> {
    let mut reasons: Vec<String> = vec![];
    for line in lines {
        if let DepartedLine::Unread(id, why) = line {
            if gone.contains(id) && !reasons.contains(why) {
                reasons.push(why.clone());
            }
        }
    }
    reasons
}

/// The one sentence saying what changed between the set on screen (`seen`) and the set just
/// read (`now`), or `None` when they are equal.
///
/// `joined` and `departed` are LOOKUP TABLES, not the change itself: the change is computed
/// here from the two sets, and each chart that moved is named by its line from the table —
/// `joined` is the pane's own member lines (read over `now`, so no second read), `departed`
/// the separately read lines of charts that left. A chart with no line in its table (the read
/// failed, or the pane has no member lines) is still named, by id ([`unread_line`]); a line
/// for a chart that did not move is ignored.
///
/// Each half appears only when it has a chart; joined first. Lines are in `ChartSet` order
/// (sorted by id), so the same change always reads the same way.
pub fn set_change_sentence(
    seen: &ChartSet,
    now: &ChartSet,
    joined: &[MemberLine],
    departed: &[DepartedLine],
) -> Option<String> {
    if seen == now {
        return None;
    }
    let came: Vec<String> = joined_ids(seen, now)
        .into_iter()
        .map(|id| joined_text(id, joined))
        .collect();
    let gone = departed_ids(seen, now);
    let went: Vec<String> = gone.iter().map(|id| departed_text(*id, departed)).collect();
    let reasons = unread_reasons(&gone, departed);
    let mut sentence = String::from("This record changed while it was on screen.");
    if !came.is_empty() {
        sentence.push_str(&format!(" Now part of it: {}.", came.join("; ")));
    }
    if !went.is_empty() {
        sentence.push_str(&format!(" No longer part of it: {}.", went.join("; ")));
    }
    if !reasons.is_empty() {
        let unread = departed
            .iter()
            .filter(|line| matches!(line, DepartedLine::Unread(id, _) if gone.contains(id)))
            .count();
        let names = if unread == 1 { "name" } else { "names" };
        let why = reasons.join("; ");
        // A reason that is already a sentence keeps its own full stop, and gets no second.
        let stop = if why.ends_with('.') { "" } else { "." };
        sentence.push_str(&format!(" Why the {names} could not be read: {why}{stop}"));
    }
    Some(sentence)
}

#[cfg(test)]
#[path = "change_tests.rs"]
mod tests;
