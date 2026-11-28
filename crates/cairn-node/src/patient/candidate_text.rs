//! The plain-text rendering of a [`CandidateList`] that `patient-search` and `patient-register`
//! both print (§5.3/§5.8).
//!
//! It lives here, as a pure function returning lines, rather than inside `main.rs`'s
//! `println!` loop, so the exact output can be pinned by a golden test: this is a
//! wrong-chart-prevention surface and a column that shifts silently is a defect a clerk acts on.

use cairn_patient_search::{CandidateList, PatientKind};

/// Width of the `name` column in `candidate_lines`' fixed-column layout. Both the header's
/// `{:<name_w$}` and `ellipsize`'s ceiling read from here so the two can never drift apart.
pub const NAME_COLUMN_WIDTH: usize = 28;

/// Shorten `s` to at most `width` CHARACTERS, marking the cut with a trailing `…`.
///
/// Character-based, not byte-based: `&s[..width]` panics on a multi-byte boundary, and a
/// patient name is exactly where multi-byte characters live ("田中 太郎", "José"). Culture-
/// neutral in the §4.2 sense — it never inspects or parses the name, only counts characters.
///
/// The ellipsis is what keeps this honest (principle 4): a silently-clipped name reads as the
/// whole name, and on a wrong-chart-prevention surface "Nguyen Thi Minh" clipped to
/// "Nguyen Thi Min" is a precise untruth a clerk could act on. It costs one character of the
/// budget — the returned string is still at most `width` characters wide, so the column holds.
///
/// Grapheme clusters, not chars, would be the fully correct unit (a combining sequence can
/// span several `char`s and render as one column), as would East-Asian double-width handling.
/// Both need a dependency and neither can make a row WIDER than this bound, so the column
/// alignment this function exists to protect is safe either way; the residual is that a
/// double-width name may render narrower than 28 columns, never wider.
pub fn ellipsize(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    // `width - 1` leaves room for the ellipsis itself. `width == 0` cannot happen from the
    // one call site (a compile-time constant of 28), but saturating keeps the function total
    // rather than panicking if a future caller passes 0.
    let keep = width.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

/// Marks a chart that is clipped to the row above it (same person, another chart).
const LINKED_PREFIX: &str = "\u{21b3} linked: ";

/// The marker when db/054 doubts a link in the row (R6, #723): the set is disputed, so it must
/// not read as settled "one person". The arrow already says "a further chart of this row".
///
/// Deliberately SHORT (12 chars). It shares the 28-character ellipsized name cell, so no column
/// moves, but a longer prefix would truncate the very name being doubted ("Mary SM..."), and
/// that name is what the clerk most needs whole.
const DOUBTED_PREFIX: &str = "\u{21b3} in doubt: ";

/// The mark for a chart of an invented patient (ADR-0115), LEADING the name cell.
///
/// Spelled out, at the cost of twelve of the cell's twenty-eight characters. A synthetic row
/// must be impossible to take for a person's, and the reader is a clerk scanning a list for a
/// name: an abbreviation is what gets scanned past. The name is ellipsized to make room; the
/// chart id beside it is whole, and that is what identifies the chart.
const SYNTHETIC_PREFIX: &str = "[SYNTHETIC] ";

/// The mark for a chart whose registration this node does not hold, so it cannot say whether
/// the patient is invented. Never drawn as an ordinary row: "not known" is not "real"
/// (principle 4). Short, because the row is otherwise an ordinary candidate.
const UNKNOWN_KIND_PREFIX: &str = "[unregistered here] ";

/// The name cell's text for a chart: its kind first, then whatever the caller built
/// (the plain name, or one already marked as linked or in doubt). **Pure.**
///
/// The kind leads EVERY other prefix, so that after ellipsizing it is the part that
/// survives: `[SYNTHETIC] ↳ linked: Jo…` still says the one thing that must be read.
pub fn name_with_kind(kind: PatientKind, name: &str) -> String {
    match kind {
        PatientKind::Synthetic => format!("{SYNTHETIC_PREFIX}{name}"),
        PatientKind::Unknown => format!("{UNKNOWN_KIND_PREFIX}{name}"),
        PatientKind::Real => name.to_string(),
    }
}

/// One chart's fixed-column line, with `name` (already prefixed if linked) in the name cell.
/// **Pure.**
fn chart_line(c: &cairn_patient_search::Candidate, name: &str) -> String {
    let age = c
        .age
        .as_ref()
        .map(|a| a.years.to_string())
        .unwrap_or_else(|| "?".to_string());
    let last_activity = c.last_activity.as_deref().unwrap_or("-");
    let locale = c.locale.as_deref().unwrap_or("-");
    format!(
        "{:<36}  {:<name_w$}  {:>4}  {:<12}  {:<12}  {}",
        c.patient_id,
        // TRUNCATED, not merely padded. Rust's `{:<n}` is a MINIMUM width: one long name would
        // push the age/trust/last-activity columns right on that row alone, so a clerk scanning
        // DOWN the age column to tell two same-named patients apart loses the alignment exactly
        // when a name is unusual. The ellipsis keeps the truncation honest (principle 4). The
        // linked prefix is inside the ellipsized text, so it never widens the cell.
        //
        // Only the NAME needs this: `patient_id` is a UUID (always 36), `age` a small integer
        // or "?", `trust` a closed short vocabulary, `last_activity` an ISO date. `locale` is
        // unbounded (issue #347) but LAST, so it can only wrap.
        ellipsize(name, NAME_COLUMN_WIDTH),
        age,
        c.trust.as_str(),
        last_activity,
        locale,
        name_w = NAME_COLUMN_WIDTH
    )
}

/// Every line `patient-search` / `patient-register` print for `list`, in order. **Pure.**
///
/// One line per candidate, in the exact order the list carries them (`patient-register`
/// attests this same order — see `register::build_registration_body`'s doc — so the print order
/// and the attested order must never be allowed to drift apart).
///
/// The `incomplete` reason is the LAST line, deliberately AFTER every candidate row rather than
/// a header above them (ADR-0060 decision 2: partial completion must be reported, never
/// implied). A reason printed first can scroll off the top of a terminal behind a long list;
/// printed last, it is the last thing on screen however many rows precede it.
pub fn candidate_lines(list: &CandidateList) -> Vec<String> {
    let mut out = Vec::new();
    if list.people.is_empty() {
        out.push("no candidates found".to_string());
    } else {
        // `{:name_w$}` on both this header and the rows below, from ONE constant, so the
        // header can never drift out of step with the width `ellipsize` truncates to.
        out.push(format!(
            "{:<36}  {:<name_w$}  {:>4}  {:<12}  {:<12}  locale",
            "patient_id",
            "name",
            "age",
            "trust",
            "last activity",
            name_w = NAME_COLUMN_WIDTH
        ));
        // One row per PERSON; the first chart prints exactly as a lone chart always did, each
        // further (linked) chart prints beneath it in the same columns with its name cell
        // marked. This flattened order IS `displayed_charts()`, which `patient-register` attests.
        for row in &list.people {
            for (i, c) in row.members().iter().enumerate() {
                let name = if i == 0 {
                    c.display_name.clone()
                } else {
                    let prefix = if row.is_doubted() {
                        DOUBTED_PREFIX
                    } else {
                        LINKED_PREFIX
                    };
                    format!("{prefix}{}", c.display_name)
                };
                out.push(chart_line(c, &name_with_kind(c.kind, &name)));
            }
        }
    }
    // Deliberately last — see the fn doc. Do not hoist this above the loop.
    if list.incomplete {
        let reason = list
            .incomplete_reason
            .as_deref()
            .unwrap_or("(no reason given)");
        out.push(format!("! search incomplete: {reason}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_patient_search::{Age, Candidate, PersonRow, TrustState};
    use uuid::Uuid;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(0x0000_0000_0000_7000_8000_0000_0000_0000 + n)
    }

    /// A candidate with every optional column set.
    fn full(n: u128, name: &str) -> Candidate {
        Candidate {
            kind: cairn_patient_search::PatientKind::Real,
            patient_id: id(n),
            display_name: name.to_string(),
            age: Some(Age {
                years: 41,
                basis: "dob".to_string(),
            }),
            trust: TrustState::Confirmed,
            last_activity: Some("2026-09-30".to_string()),
            locale: Some("Dorrigo".to_string()),
            photo_ref: None,
        }
    }

    /// A candidate with none of the optional columns.
    fn bare(n: u128, name: &str) -> Candidate {
        Candidate {
            kind: cairn_patient_search::PatientKind::Real,
            patient_id: id(n),
            display_name: name.to_string(),
            age: None,
            trust: TrustState::Unconfirmed,
            last_activity: None,
            locale: None,
            photo_ref: None,
        }
    }

    fn list(people: Vec<PersonRow>) -> CandidateList {
        CandidateList {
            people,
            incomplete: false,
            incomplete_reason: None,
        }
    }

    #[test]
    fn a_doubted_rows_further_members_say_in_doubt() {
        let row = PersonRow::new(vec![full(0xa, "Jane CITIZEN"), bare(0xb, "Mary SMYTHE")])
            .unwrap()
            .with_doubt(true);
        let lines = candidate_lines(&list(vec![row]));
        assert!(
            lines[2].contains("\u{21b3} in doubt: Mary SMYTHE"),
            "{}",
            lines[2]
        );
    }

    const HEADER: &str = "patient_id                            name                           age  trust         last activity  locale";

    #[test]
    fn golden_two_never_linked_rows_print_exactly_as_before_r3() {
        let l = list(PersonRow::each_alone(vec![
            full(0xa, "Jane CITIZEN"),
            bare(0xb, "Doe"),
        ]));
        assert_eq!(
            candidate_lines(&l),
            vec![
                HEADER.to_string(),
                "00000000-0000-7000-8000-00000000000a  Jane CITIZEN                    41  confirmed     2026-09-30    Dorrigo".to_string(),
                "00000000-0000-7000-8000-00000000000b  Doe                              ?  unconfirmed   -             -".to_string(),
            ]
        );
    }

    /// ADR-0115: a synthetic chart's row says so FIRST, and no column moves because of it.
    #[test]
    fn a_synthetic_chart_is_marked_at_the_front_of_its_name_cell() {
        let mut invented = full(1, "Ada Training");
        invented.kind = PatientKind::Synthetic;
        let list = CandidateList {
            people: PersonRow::each_alone(vec![invented, full(2, "Bo Real")]),
            incomplete: false,
            incomplete_reason: None,
        };
        let lines = candidate_lines(&list);
        assert!(
            lines[1].contains("  [SYNTHETIC] Ada Training"),
            "the mark leads the name cell: {}",
            lines[1]
        );
        assert!(
            !lines[2].contains('['),
            "a real patient's row carries no mark: {}",
            lines[2]
        );
        // The columns after the name start where they always did, on every row.
        let age_at = |line: &str| {
            line.char_indices()
                .nth(36 + 2 + NAME_COLUMN_WIDTH)
                .map(|x| x.0)
        };
        assert_eq!(age_at(&lines[1]), age_at(&lines[2]));
        assert_eq!(lines[1].chars().count(), lines[2].chars().count());
    }

    /// The mark survives a long name: the NAME is what is cut, never the mark.
    #[test]
    fn a_long_synthetic_name_loses_its_tail_and_keeps_its_mark() {
        let cell = ellipsize(
            &name_with_kind(PatientKind::Synthetic, "Bartholomew Featherstonehaugh"),
            NAME_COLUMN_WIDTH,
        );
        assert!(cell.starts_with("[SYNTHETIC] Bartholomew"), "{cell}");
        assert!(cell.ends_with('…'));
        assert_eq!(cell.chars().count(), NAME_COLUMN_WIDTH);
    }

    /// A chart whose registration is not held here is NOT drawn as an ordinary row.
    #[test]
    fn a_chart_of_unknown_kind_is_never_drawn_as_a_real_one() {
        assert_eq!(
            name_with_kind(PatientKind::Unknown, "Cy Somewhere"),
            "[unregistered here] Cy Somewhere"
        );
        assert_eq!(
            name_with_kind(PatientKind::Real, "Cy Somewhere"),
            "Cy Somewhere"
        );
        // The kind leads every other mark, so it is what survives the cut.
        let linked = format!("{LINKED_PREFIX}Jo Smith");
        assert!(name_with_kind(PatientKind::Synthetic, &linked).starts_with("[SYNTHETIC] "));
    }

    #[test]
    fn golden_an_empty_list_says_so() {
        assert_eq!(
            candidate_lines(&CandidateList::empty()),
            vec!["no candidates found".to_string()]
        );
    }

    #[test]
    fn golden_an_incomplete_reason_is_the_last_line() {
        let mut l = list(PersonRow::each_alone(vec![bare(0xb, "Doe")]));
        l.incomplete = true;
        l.incomplete_reason = Some("a projection was unreadable".to_string());
        let lines = candidate_lines(&l);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[2], "! search incomplete: a projection was unreadable");
        // No reason supplied: the fallback text, still last.
        l.incomplete_reason = None;
        assert_eq!(
            candidate_lines(&l).last().unwrap(),
            "! search incomplete: (no reason given)"
        );
        // An empty but incomplete list still reports the partial search.
        let mut e = CandidateList::empty();
        e.incomplete = true;
        e.incomplete_reason = Some("r".to_string());
        assert_eq!(
            candidate_lines(&e),
            vec![
                "no candidates found".to_string(),
                "! search incomplete: r".to_string()
            ]
        );
    }

    #[test]
    fn a_linked_member_is_printed_under_its_row() {
        // Row [a, b]: a is printed as today; b on the next line, its name cell "↳ linked: …".
        let row =
            PersonRow::new(vec![full(0xa, "Jane CITIZEN"), bare(0xb, "Mary SMYTHE")]).unwrap();
        let l = list(vec![row, PersonRow::alone(bare(0xc, "Doe"))]);
        let lines = candidate_lines(&l);
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(lines[1].starts_with(&id(0xa).to_string()));
        assert!(lines[1].contains("Jane CITIZEN"), "{}", lines[1]);
        assert!(lines[2].starts_with(&id(0xb).to_string()));
        assert!(lines[2].contains("↳ linked: Mary SMYTHE"), "{}", lines[2]);
        // The member keeps the same columns (here: unconfirmed, no age) as a row of its own.
        assert!(lines[2].contains("unconfirmed"), "{}", lines[2]);
        assert!(lines[3].starts_with(&id(0xc).to_string()));
    }

    #[test]
    fn the_printed_chart_order_is_the_attested_order() {
        // `patient-register` signs `displayed_charts()`; what the clerk reads must be that
        // same sequence, linked members included, or they sign charts in a different order
        // than they saw them.
        let row =
            PersonRow::new(vec![full(0xa, "Jane CITIZEN"), bare(0xb, "Mary SMYTHE")]).unwrap();
        let l = list(vec![row, PersonRow::alone(bare(0xc, "Doe"))]);
        let printed: Vec<String> = candidate_lines(&l)
            .iter()
            .skip(1) // the header
            .map(|line| line.split_whitespace().next().unwrap().to_string())
            .collect();
        let attested: Vec<String> = l.displayed_charts().iter().map(Uuid::to_string).collect();
        assert_eq!(printed, attested);
    }

    #[test]
    fn a_long_linked_name_is_cut_to_the_same_column_width() {
        let long = "x".repeat(NAME_COLUMN_WIDTH + 10);
        let row = PersonRow::new(vec![bare(0xa, "A"), bare(0xb, &long)]).unwrap();
        let lines = candidate_lines(&list(vec![row]));
        // The trust column starts at the same CHARACTER offset on both lines, so the prefix
        // did not widen the name cell (bytes would differ: the arrow and ellipsis are multibyte).
        let col = |l: &str| l.find("unconfirmed").map(|byte| l[..byte].chars().count());
        assert_eq!(col(&lines[1]), col(&lines[2]), "{lines:?}");
        assert!(lines[2].contains('…'), "{}", lines[2]);
    }

    // --- the name column must not shift on a long name (final review) ---

    #[test]
    fn a_short_name_is_returned_unchanged_and_never_gains_an_ellipsis() {
        assert_eq!(ellipsize("Smith, John", NAME_COLUMN_WIDTH), "Smith, John");
        // Exactly at the boundary: still whole, still no ellipsis.
        let exact: String = std::iter::repeat_n('x', NAME_COLUMN_WIDTH).collect();
        assert_eq!(ellipsize(&exact, NAME_COLUMN_WIDTH), exact);
    }

    #[test]
    fn a_long_name_is_cut_to_the_column_width_and_says_so() {
        let long: String = std::iter::repeat_n('x', NAME_COLUMN_WIDTH + 40).collect();
        let out = ellipsize(&long, NAME_COLUMN_WIDTH);
        assert_eq!(
            out.chars().count(),
            NAME_COLUMN_WIDTH,
            "the whole point: the rendered cell must never exceed the column, or every \
             later column shifts on that row alone"
        );
        assert!(
            out.ends_with('…'),
            "a clipped name must not read as the whole name (principle 4): {out}"
        );
    }

    #[test]
    fn a_multibyte_name_is_cut_on_a_character_boundary_not_a_byte_one() {
        // Byte slicing here would PANIC mid-character. A patient name is exactly where
        // multi-byte characters live, so this is the realistic case, not an exotic one.
        let long: String = std::iter::repeat_n('田', NAME_COLUMN_WIDTH + 5).collect();
        let out = ellipsize(&long, NAME_COLUMN_WIDTH);
        assert_eq!(out.chars().count(), NAME_COLUMN_WIDTH);
        assert!(out.ends_with('…'));
        assert!(out.starts_with('田'));
    }
}
