//! The one-line allergy strip, composed from what `view::build_view` decided (split out of
//! `view.rs` to keep both files short). Pure.
use super::view::{day_text, ListStatus};

/// What the strip is composed from (one struct: the pieces are many and easily swapped).
pub(super) struct Strip<'a> {
    pub(super) drug: &'a ListStatus,
    pub(super) other: &'a ListStatus,
    /// Both lists rest on the same "all" statement.
    pub(super) one_all_row: bool,
    /// The drug list rests on an "all" statement (its other half overridden, when not `one_all_row`).
    pub(super) drug_from_all: bool,
    pub(super) drug_lines: &'a [&'a str],
    pub(super) other_lines: &'a [&'a str],
    pub(super) adverse: usize,
    /// De-labelled lines (counted quietly, after the adverse effects).
    pub(super) historical: usize,
    pub(super) unrecognised: usize,
    pub(super) unreadable: usize,
}

/// Compose the one-line strip: entries within a list join with "; ", sections with " · ".
///
/// An allergy record this terminal cannot read LEADS the strip, and then nothing says "not yet
/// asked": that record may be the very answer. A "none known" claim still shows, after it.
pub(super) fn strip_text(s: &Strip) -> String {
    let (drug, other, one_all_row) = (s.drug, s.other, s.one_all_row);
    let readable = s.unreadable == 0;
    let mut parts: Vec<String> = Vec::new();
    if !readable {
        parts.push(format!(
            "{} allergy record{} cannot be read on this terminal",
            s.unreadable,
            if s.unreadable == 1 { "" } else { "s" }
        ));
    }
    match (drug, other) {
        (ListStatus::Entries, _) => parts.push(s.drug_lines.join("; ")),
        (ListStatus::NoneKnown { asked }, _) if one_all_row => {
            parts.push(format!("No known allergies (asked {})", day_text(*asked)))
        }
        (ListStatus::NoneKnown { asked }, _) if s.drug_from_all => parts.push(format!(
            "No known drug allergies (asked {})",
            day_text(*asked)
        )),
        (ListStatus::NoneKnown { asked }, _) => {
            parts.push(format!("NKDA (asked {})", day_text(*asked)))
        }
        (ListStatus::NotYetAsked, _) if !readable => {}
        (ListStatus::NotYetAsked, ListStatus::NotYetAsked) => {
            parts.push("Allergies not yet asked".into())
        }
        (ListStatus::NotYetAsked, _) => parts.push("Drug allergies not yet asked".into()),
    }
    match (drug, other) {
        (_, ListStatus::Entries) => {
            // On the strip "Other:" already says it is an allergy; other wordings stay in full.
            let lines: Vec<&str> = s
                .other_lines
                .iter()
                .map(|l| l.strip_prefix("Allergy: ").unwrap_or(l))
                .collect();
            parts.push(format!("Other: {}", lines.join("; ")))
        }
        // Other allergies rest on a statement of their own: a different one from the drug list's,
        // or an "all" statement whose drug half a drug allergy overrides.
        (_, ListStatus::NoneKnown { asked }) if !one_all_row => parts.push(format!(
            "no known other allergies (asked {})",
            day_text(*asked)
        )),
        (ListStatus::NoneKnown { .. }, ListStatus::NotYetAsked) if readable => {
            parts.push("other allergies: not yet asked".into())
        }
        _ => {}
    }
    let (adverse, unrecognised) = (s.adverse, s.unrecognised);
    if adverse > 0 {
        parts.push(format!(
            "+{adverse} adverse effect{}",
            if adverse == 1 { "" } else { "s" }
        ));
    }
    if s.historical > 0 {
        parts.push(format!("+{} historical", s.historical));
    }
    if unrecognised > 0 {
        parts.push(format!(
            "+{unrecognised} unrecognised entr{}",
            if unrecognised == 1 { "y" } else { "ies" }
        ));
    }
    parts.join(" · ")
}
