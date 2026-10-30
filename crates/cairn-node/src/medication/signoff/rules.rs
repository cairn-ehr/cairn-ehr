//! The pure set rules the whole-list sign-off checks before and between its two reads
//! (ADR-0060 decision 5, ADR-0076 decisions 2 and 3): which chart each thread signs on,
//! whether the chart set or the lines to sign moved, and the unions that keep a line
//! reported when either read saw it.
//!
//! Split out of `signoff.rs` (#810) so the orchestrator stays readable; nothing here touches
//! a database, so every rule is tested in `tests.rs` without one.
use super::FailedLine;
use cairn_medication_view::{ChartSet, MedicationRow};
use std::collections::HashMap;
use uuid::Uuid;

/// Render a set of uuids (threads or charts) for a clinician-facing message: `"none"` when
/// empty, otherwise a comma-separated list. Pure and reusable rather than inlined at each
/// call site, so the mismatch diagnostics' symmetric halves (added / removed, shown / now)
/// stay visibly identical instead of risking silent drift between hand-written formats.
fn format_ids<'a>(ids: impl Iterator<Item = &'a Uuid>) -> String {
    let rendered: Vec<String> = ids.map(|id| id.to_string()).collect();
    if rendered.is_empty() {
        "none".to_string()
    } else {
        rendered.join(", ")
    }
}

/// The sorted, deduplicated union of two uuid lists.
///
/// Used for the incompleteness signal across the two reads, where the safe direction is to
/// over-report: a group missing at EITHER moment is a group the clinician was not shown,
/// and dropping it because the other read happened not to see it would put the silence back
/// that #334 exists to break.
pub(super) fn union_sorted(a: &[Uuid], b: &[Uuid]) -> Vec<Uuid> {
    let mut out: Vec<Uuid> = a.iter().chain(b.iter()).copied().collect();
    out.sort();
    out.dedup();
    out
}

/// The lines elsewhere at EITHER read, each once (by line), the first read's copy and order
/// kept.
///
/// The same safe direction as `union_sorted`: a re-home landing between the two reads can move
/// an already-signed line onto another chart WITHOUT changing the target set, and only the
/// second read sees it. Dropping it would put back the silence #849 removed. Pure.
pub(super) fn union_elsewhere(
    first: Vec<cairn_medication_view::ElsewhereLine>,
    second: Vec<cairn_medication_view::ElsewhereLine>,
) -> Vec<cairn_medication_view::ElsewhereLine> {
    let mut out = first;
    for line in second {
        if !out.iter().any(|l| l.group_id == line.group_id) {
            out.push(line);
        }
    }
    out
}

/// Every thread on the list → the chart that thread lives on (ADR-0076 decision 2).
///
/// Read from each row's members (`MemberVouch::patient_id`, the thread's own statement's
/// chart), NOT from `MedicationRow::display_chart`: that field is only the chart the GROUP
/// displays under — the view's display winner — which for a group spanning two linked
/// charts is one of them, and would put the other chart's thread on the wrong chart.
///
/// Pure, so the rule "a thread signs on its own chart" is tested without a database.
pub(super) fn thread_charts(rows: &[MedicationRow]) -> HashMap<Uuid, Uuid> {
    rows.iter()
        .flat_map(|row| row.members.iter())
        .map(|member| (member.medication_id, member.patient_id))
        .collect()
}

/// The chart `thread` must be attested under, or the `FailedLine` explaining why it
/// cannot be. There is deliberately no fallback: a missing entry is reported, never
/// answered with the opened chart (see `sign_off_medication_list`).
pub(super) fn chart_of_thread(
    charts_of: &HashMap<Uuid, Uuid>,
    thread: Uuid,
) -> Result<Uuid, FailedLine> {
    charts_of.get(&thread).copied().ok_or_else(|| FailedLine {
        medication_id: thread,
        error: "its chart could not be read (the thread is not a member of any line on this \
                list), so it was not signed rather than signed on a guessed chart"
            .to_string(),
    })
}

/// Refuse the gesture when the chart set moved (ADR-0076 decision 3): `Ok` exactly when
/// `before == now`, otherwise the clinician-facing refusal naming both sets.
///
/// Shared by both compares — the displayed set against the first read, and the first read
/// against the second — so the two refusals say the same thing in the same words; `window`
/// is the only part that differs ("while this list was on screen" / "while it was being
/// signed"). Sets compare by value: `ChartSet` is sorted and deduplicated on construction,
/// so two reads of one link component are equal whatever order the database returned.
pub(super) fn ensure_same_charts(
    before: &ChartSet,
    now: &ChartSet,
    window: &str,
) -> anyhow::Result<()> {
    if before == now {
        return Ok(());
    }
    Err(crate::db_diagnosis::stale_picture_refusal(format!(
        "the linked charts changed {window} (shown: {}; now: {}); nothing was signed",
        format_ids(before.members().iter()),
        format_ids(now.members().iter()),
    )))
}

/// Refuse the gesture when the lines to sign moved between the two reads (ADR-0060 decision
/// 5): `Ok` exactly when `expected == actual`, otherwise a refusal naming what was added and
/// removed. It does not ask "is this chart perfect?" — that is always answered by reporting —
/// but "is this the same list the human reviewed?".
///
/// Both inputs are `sign_off_targets` output (sorted, deduplicated), so equality is by value.
/// It names WHAT changed, not just how many: two counts that happen to match (a thread swapped
/// for another) would otherwise read as a true but useless "3 vs 3".
///
/// A stale picture (#797), like `ensure_same_charts`: the screen is behind the record, so the
/// window re-reads by itself. The sentence therefore says only what happened; the remedy is
/// the scope's job, never a control named in the text.
pub(super) fn ensure_same_targets(expected: &[Uuid], actual: &[Uuid]) -> anyhow::Result<()> {
    if expected == actual {
        return Ok(());
    }
    Err(crate::db_diagnosis::stale_picture_refusal(format!(
        "the medication list changed while it was being signed (thread(s) added: {}; \
         removed: {}); nothing was signed",
        format_ids(actual.iter().filter(|t| !expected.contains(t))),
        format_ids(expected.iter().filter(|t| !actual.contains(t))),
    )))
}
