//! The pure rules of a "set aside" version on the medication draft list (batch charting slice 2a).
//!
//! A clinician may chart a line as a VERSION of a companion: "this is the same drug as that other
//! line, and that one is set aside, not current". A version only makes sense beside a companion
//! of the same drug, so the rules are: the line is a single-part line coded to a moiety
//! ([`MOIETY_SYSTEM`]), and the companion is another draft line, or a line already charted and
//! current, of the very same moiety. Uncoded lines and combinations get no companion (we cannot
//! say which drug they are), so no "Set aside" button. These functions take plain facts and
//! touch no database, so a reviewer can read the whole rule; the window decides what to show
//! from them, and Sign re-asks [`version_holds`] on a fresh read (the mark is a stored claim,
//! and the companion may have been edited to another drug since).
//!
//! Also here: [`info_source_of`], the one table from a "From:" preset label to the wire
//! category. The category is never guessed from the free text the clinician typed.

use crate::allergy::check::MOIETY_SYSTEM;
use crate::medication::draft::DraftLine;
use uuid::Uuid;

/// What a version is set aside beside: another draft line, or a charted current medication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Companion {
    Draft(Uuid),
    Charted(Uuid),
}

/// The moiety code of a single-part line coded under [`MOIETY_SYSTEM`]; `None` for uncoded
/// lines, combinations (several parts) and other coding systems.
fn moiety_of(line: &DraftLine) -> Option<&str> {
    let parts = line.parts.as_array()?;
    let [part] = parts.as_slice() else {
        return None;
    };
    let coding = part.get("coding")?;
    if coding.get("system")?.as_str()? != MOIETY_SYSTEM {
        return None;
    }
    coding.get("code")?.as_str()
}

/// Lines (draft or current charted) of the same single-part moiety as `line`, excluding itself
/// and any draft line that is itself set aside (a set-aside line is no one's companion).
/// `current` is `(medication_id, moiety code)` for each charted current line. Empty means no
/// "Set aside" button. Draft candidates come first, in list order.
pub fn companion_candidates(
    line: &DraftLine,
    draft: &[DraftLine],
    current: &[(Uuid, &str)],
) -> Vec<Companion> {
    let Some(code) = moiety_of(line) else {
        return vec![];
    };
    let drafts = draft
        .iter()
        .filter(|d| d.line_id != line.line_id && d.set_aside_of.is_none())
        .filter(|d| moiety_of(d) == Some(code))
        .map(|d| Companion::Draft(d.line_id));
    let charted = current
        .iter()
        .filter(|(_, c)| *c == code)
        .map(|(id, _)| Companion::Charted(*id));
    drafts.chain(charted).collect()
}

/// `line:<uuid>` / `med:<uuid>` to a [`Companion`]; `None` for anything else.
fn parse_mark(mark: &str) -> Option<Companion> {
    let (kind, id) = mark.split_once(':')?;
    let id = id.parse().ok()?;
    match kind {
        "line" => Some(Companion::Draft(id)),
        "med" => Some(Companion::Charted(id)),
        _ => None,
    }
}

/// At Sign: `Ok(companion)` while `line.set_aside_of` still names a candidate, `Err(reason)`
/// otherwise, each worded for what happened (ruling R13; the window shows it as "version kept:
/// {reason}"): not set aside, a malformed mark, a charted companion no longer current (stopped,
/// or not on this read), a list companion no longer on the list (removed) or itself set aside (a
/// chained mark), or a companion still there but now another drug. A gone line is never called
/// another drug, nor a line still on the list gone.
pub fn version_holds(
    line: &DraftLine,
    draft: &[DraftLine],
    current: &[(Uuid, &str)],
) -> Result<Companion, &'static str> {
    let mark = line.set_aside_of.as_deref().ok_or("not set aside")?;
    let named = parse_mark(mark).ok_or("the set-aside mark is not a line")?;
    if companion_candidates(line, draft, current).contains(&named) {
        return Ok(named);
    }
    let still_there = match named {
        Companion::Draft(id) => draft
            .iter()
            .find(|d| d.line_id == id)
            .map(|d| d.set_aside_of.is_none()),
        Companion::Charted(id) => current.iter().any(|(m, _)| *m == id).then_some(true),
    };
    // Some(true): the line is there and not set aside; Some(false): a list line there but itself
    // set aside (a chained mark); None: gone.
    Err(match (still_there, named) {
        (Some(true), _) => "the line it was set aside beside is no longer the same drug",
        (Some(false), _) => "its line is itself set aside",
        (None, Companion::Draft(_)) => "its line is no longer on the list",
        (None, Companion::Charted(_)) => "its line is no longer current",
    })
}

/// The wire category of a "From:" preset label; `None` for no preset or one not in the list.
pub fn info_source_of(preset: Option<&str>) -> Option<&'static str> {
    match preset? {
        "Patient" | "Carer / family" => Some("patient-reported"),
        "GP letter" | "Pharmacy" => Some("external-record"),
        "Packets / Webster pack seen" => Some("clinician-observed"),
        "Other" => Some("unknown"),
        _ => None,
    }
}

#[cfg(test)]
#[path = "draft_versions_tests.rs"]
mod tests;
