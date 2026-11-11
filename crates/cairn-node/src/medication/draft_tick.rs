//! The pure tick rules of the medication draft list (batch-charting design, slice 1).
//!
//! The list is a durable unsigned set of typed lines; one Sign charts the ticked ones. Which lines
//! start ticked, and whether a clinician's own tick still stands once the check is re-run at Sign
//! time, are decided HERE from plain facts, with no database, so a reviewer can read the whole rule.
//! The default errs toward NOT ticking: an unticked line costs one click, a wrongly ticked one is
//! a medication charted without a look (paper-parity: the clinician can always tick by hand).

use crate::allergy::check::LineCheck;
use crate::allergy::view::ListStatus;
use crate::medication::line::Directions;
use cairn_event::medication::ALLERGY_SOURCE;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Everything the default-tick rule looks at, gathered by the caller (so this stays pure).
pub struct TickFacts<'a> {
    /// The clinician marked the line finished (an unfinished line is still being typed).
    pub finished: bool,
    /// At least one part, and every part has a coding.
    pub all_parts_coded: bool,
    /// `read_directions` of the line's directions.
    pub directions: &'a Directions,
    /// The allergy check's answer for the line.
    pub check: &'a LineCheck,
    /// The patient's drug-allergy list status.
    pub drug_allergies: &'a ListStatus,
    /// The line's moiety also appears on another line (draft or chart).
    pub duplicate: bool,
    /// The list has a named writer; a NULL-writer list is hand-ticked only, because a default
    /// tick would put a name on a decision nobody with that name made.
    pub named_writer: bool,
}

/// Whether a line starts ticked. Every clause is a reason to withhold; all must be clear.
pub fn default_tick(f: &TickFacts) -> bool {
    let d = f.directions;
    // A dose is read only when both halves are: "500" alone is not a dose.
    let dose_read = d.dose_amount.is_some() && d.dose_unit.is_some();
    // "As needed" and "once, now" are complete directions without a frequency word.
    let directions_read = d.frequency.is_some() || d.prn || d.stat;
    f.named_writer
        && f.finished
        && f.all_parts_coded
        && d.form.is_some()
        && dose_read
        && directions_read
        // The check ran (principle 4: "not checked" is never read as "clear").
        && !f.check.unverified
        // A mark is an allergy hit: the clinician decides, not the default. An adverse-effect
        // note alone is information, not a hit, and does not withhold.
        && f.check.marks.is_empty()
        // With the allergy list never asked, "no hit" proves nothing.
        && *f.drug_allergies != ListStatus::NotYetAsked
        && !f.duplicate
}

/// Whether a tick the clinician made by hand still stands after the check is re-run at Sign time:
/// every warning the fresh check finds must be one the clinician SAW when ticking
/// (`ticked_against`, as `(source, ref)` pairs). A new one means the world moved under the tick.
///
/// The reason rules are NOT judged here (Task 4 review, ruling R3): a Start line on a seen mark
/// with no reason, and a reason against a check that could not run, are slice 1a's PER-LINE
/// refusals (`NEEDS_A_REASON`, `CHECK_DID_NOT_RUN`) at Sign, not a changed picture, so the tick
/// stands and the line says what it needs. A check that could not run finds no warning, so it
/// holds a tick here (and refuses the line's reason there).
pub fn hand_tick_holds(fresh: &LineCheck, ticked_against: &[(String, Uuid)]) -> bool {
    fresh.against.iter().all(|id| {
        ticked_against
            .iter()
            .any(|(s, r)| s == ALLERGY_SOURCE && r == id)
    })
}

/// The moiety codes that occur on more than one line of `draft` plus `current`, restricted to
/// codes that occur on at least one draft line (two chart lines sharing a code is not a list
/// matter). `draft` has one entry per draft line: `Some(code)` only for a single-part line coded
/// under `MOIETY_SYSTEM` (the caller filters by it), `None` for uncoded or combination lines (they never group).
pub fn duplicate_codes(draft: &[Option<&str>], current: &[&str]) -> BTreeSet<String> {
    let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    for c in draft.iter().flatten().chain(current.iter()) {
        *count.entry(c).or_default() += 1;
    }
    draft
        .iter()
        .flatten()
        .filter(|c| count[**c] > 1)
        .map(|c| c.to_string())
        .collect()
}

#[cfg(test)]
#[path = "draft_tick_tests.rs"]
mod tests;
