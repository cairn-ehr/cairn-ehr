//! Display helpers shared by more than one surface: [`short_kid`] for every surface that shows
//! an actor key id, and [`shown_name`] for every list that shows a coded drug (the medication
//! list and the allergy list).
//!
//! WHY `short_kid` IS ONE FUNCTION AND NOT TWO. The med-list window shows a key id in two places
//! at once: the signature column (from the view model) and the "signing as …" line (from
//! the window's own lock state). Those came from two hand-written copies of the same
//! truncation rule. Two copies is one edit away from a chart that renders the same person
//! as two different-looking ids on the same screen — which reads as two clinicians, on a
//! surface whose whole job is saying *whose* signature a drug carries.

/// How many characters of a key id a human is shown.
///
/// Long enough to tell colleagues on one node apart, short enough to read at a glance. The
/// full id is always available in the event log; this is a label, never an identifier the
/// system matches on.
pub const DISPLAYED_KID_CHARS: usize = 8;

/// The first [`DISPLAYED_KID_CHARS`] characters of an actor key id.
///
/// Sliced on a CHAR boundary rather than a byte index: attester ids are hex in practice,
/// but this is display code reached from both the chart renderer and the lock line, and a
/// panic over a label would take the whole window down with it. A shorter id (or an empty
/// one) is returned unchanged rather than padded — inventing characters in an identity
/// label is worse than a short one.
pub fn short_kid(kid: &str) -> &str {
    match kid.char_indices().nth(DISPLAYED_KID_CHARS) {
        Some((byte_index, _)) => &kid[..byte_index],
        None => kid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ordinary case: a hex key id is cut to the displayed length.
    #[test]
    fn a_key_id_is_cut_to_the_displayed_length() {
        assert_eq!(short_kid("abcdef0123456789"), "abcdef01");
        assert_eq!(
            short_kid("abcdef0123456789").chars().count(),
            DISPLAYED_KID_CHARS
        );
    }

    /// Every degenerate input a label can arrive with must return, not panic — this is
    /// reached from the chart renderer, where a panic is a blank window.
    #[test]
    fn a_short_or_empty_id_is_returned_unaltered() {
        assert_eq!(short_kid("abc"), "abc");
        assert_eq!(short_kid(""), "");
        assert_eq!(short_kid("abcdef01"), "abcdef01");
    }

    /// The reason this slices on char boundaries: a byte index into multi-byte characters
    /// panics, and a fixture attester id is deliberately NOT hex (house rule 6).
    #[test]
    fn a_non_ascii_id_is_cut_without_panicking() {
        assert_eq!(short_kid("ααααααααββ"), "αααααααα");
        assert_eq!(short_kid("fixture-clinician-b"), "fixture-");
    }
}

/// How a coded drug is shown: its listed (generic) name, plus the term the clinician signed in
/// brackets when that is another word — "epinephrine (Adrenaline)", "amoxicillin + clavulanic
/// acid (Augmentin)" (charting design §3.3, #1073, ADR-0106). The same word in another case is
/// no other word: "amoxicillin", never "amoxicillin (Amoxicillin)". Shared by the medication list
/// and the allergy list, so both bracket alike. Pure.
pub fn shown_name(display: &str, term: &str) -> String {
    let term = term.trim();
    if term.is_empty() || term.to_lowercase() == display.to_lowercase() {
        display.to_string()
    } else {
        format!("{display} ({term})")
    }
}

impl crate::row::MedicationRow {
    /// The name a reader sees in the list's name cell: [`shown_name`] of the coded display and
    /// the term when the row is coded, else the term as asserted. Sorting stays on
    /// [`display_name`](crate::row::MedicationRow::display_name): the bracket only ever FOLLOWS
    /// that name, so it never moves a row.
    pub fn shown_name(&self) -> String {
        match self.coding_display.as_deref() {
            Some(display) => shown_name(display, &self.term),
            None => self.display_name().to_string(),
        }
    }
}

#[cfg(test)]
mod shown_name_tests {
    use super::shown_name;

    #[test]
    fn shown_name_adds_the_typed_word_only_when_different() {
        assert_eq!(
            shown_name("amoxicillin + clavulanic acid", "Augmentin"),
            "amoxicillin + clavulanic acid (Augmentin)"
        );
        assert_eq!(shown_name("amoxicillin", "Amoxicillin"), "amoxicillin");
        assert_eq!(shown_name("amoxicillin", "  "), "amoxicillin");
        // What "adren" really signs (drugref's MeSH spelling), as the list then reads it.
        assert_eq!(
            shown_name("epinephrine", "Adrenaline"),
            "epinephrine (Adrenaline)"
        );
    }
}
