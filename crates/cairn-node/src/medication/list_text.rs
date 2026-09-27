//! Human-readable text for the CLI's `medication-list` combined-chart signal.
//!
//! WHY THIS EXISTS. ADR-0076 decision 1 says every row of a combined list must name its
//! source chart(s) — but until this file, `--json` was the only output that did: the
//! `MedicationList` CLI arm's plain-text branch printed a row's name, dose, status and
//! vouches, and NOTHING about which chart the row came from. The failure that surfaces:
//! an operator opens chart A (linked to chart B), sees amlodipine on the printed list — it
//! is actually recorded on B — and later runs `medication-cease --patient A …` against it,
//! which refuses with a "#192 patient cannot change" error that names no chart and gives
//! no hint the drug was never A's to begin with. The list itself had the information
//! (`MedicationRow::source_charts`) and never said it.
//!
//! WHY A SEPARATE PURE MODULE RATHER THAN INLINE STRINGS IN `main.rs`. Two properties need
//! checking independently of any database: that a NEVER-linked chart's output is untouched
//! (byte-identical to what this CLI printed before R1 — nobody reading a single, unlinked
//! chart should see so much as one new character), and that a LINKED chart's header and
//! per-row suffix actually name every chart involved. Both are properties of pure string
//! formatting over `ChartSet`/`Vec<Uuid>`, so they belong in functions a unit test can
//! drive directly — not buried in a 150-line `println!` arm that only a full CLI run
//! exercises.
use cairn_medication_view::ChartSet;
use uuid::Uuid;

/// The list-level header naming the chart set a combined read covers.
///
/// `None` on a never-linked chart (`charts.is_linked()` is false) — the header is new
/// information that exists only because there is more than one chart to name; printing it
/// unconditionally would mean EVERY chart's output gained a line it did not have before
/// R1, not just a linked one's. `main.rs` prints nothing at all when this returns `None`,
/// which is what keeps a never-linked chart's output byte-identical to its pre-R1 form.
pub fn combined_list_header(charts: &ChartSet) -> Option<String> {
    if !charts.is_linked() {
        return None;
    }
    Some(format!(
        "combined list across {} linked charts: {}",
        charts.members().len(),
        join_ids(charts.members())
    ))
}

/// The per-row suffix naming where one displayed line was actually recorded.
///
/// `None` on a never-linked list, for the same byte-identical reason as
/// [`combined_list_header`] — `linked` is `PatientMedicationList::charts.is_linked()`,
/// decided once per list by the caller (the same value the header was built from), rather
/// than re-derived here from `source_charts` alone: an empty `source_charts` must not be
/// mistaken for "not linked" (see below).
///
/// On a linked list this is always `Some`, even when `source_charts` is empty. That case is
/// reachable only through a race documented on `MedicationRow::source_charts` itself (a
/// group-chart read landing between two of `list_patient_medications`'s several
/// statements), and an empty suffix here would render as though the row had nothing to
/// say — exactly the silent loss of provenance this function exists to prevent. Principle 4:
/// an unknown provenance is a recordable state, not a blank.
pub fn row_source_suffix(source_charts: &[Uuid], linked: bool) -> Option<String> {
    if !linked {
        return None;
    }
    if source_charts.is_empty() {
        return Some(" — recorded on chart(s) (not read)".to_string());
    }
    Some(format!(
        " — recorded on chart(s) {}",
        join_ids(source_charts)
    ))
}

/// Render a slice of chart/thread ids as the comma-separated form both functions above
/// share, so the header and the per-row suffix can never drift into two different
/// separators or orderings for what is conceptually the same kind of list.
fn join_ids(ids: &[Uuid]) -> String {
    ids.iter()
        .map(Uuid::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// The byte-identical guarantee: a never-linked chart gets no header and no suffix at
    /// all, on any row — including one that (implausibly, but not something this function
    /// should trust) carries source charts of its own.
    #[test]
    fn a_never_linked_chart_gets_no_header_and_no_suffix() {
        let charts = ChartSet::single(u(1));
        assert_eq!(combined_list_header(&charts), None);
        assert_eq!(row_source_suffix(&[u(1)], false), None);
        assert_eq!(row_source_suffix(&[], false), None);
    }

    /// A linked chart's header names the count and every member, so the operator sees at a
    /// glance that this is not a single-chart read.
    #[test]
    fn a_linked_chart_gets_a_header_naming_every_member() {
        let charts = ChartSet::new([u(2), u(1)]).unwrap();
        let header = combined_list_header(&charts).expect("a linked list gets a header");
        assert!(
            header.contains("combined list across 2 linked charts"),
            "got: {header}"
        );
        assert!(header.contains(&u(1).to_string()), "got: {header}");
        assert!(header.contains(&u(2).to_string()), "got: {header}");
    }

    /// The case ADR-0076 decision 1 exists for: a group reconciled from threads recorded on
    /// two different charts must name BOTH, not just the first.
    #[test]
    fn a_row_with_two_source_charts_names_both() {
        let suffix = row_source_suffix(&[u(1), u(2)], true).expect("a linked row gets a suffix");
        assert_eq!(
            suffix,
            format!(" — recorded on chart(s) {}, {}", u(1), u(2))
        );
    }

    /// The race case: `source_charts` empty on a linked list says so explicitly rather than
    /// rendering an empty, silently-provenance-free suffix.
    #[test]
    fn a_linked_row_with_no_source_charts_says_so_rather_than_going_silent() {
        assert_eq!(
            row_source_suffix(&[], true),
            Some(" — recorded on chart(s) (not read)".to_string())
        );
    }
}
