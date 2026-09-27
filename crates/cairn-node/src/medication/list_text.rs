//! Human-readable text for the CLI's `medication-list` combined-chart signal.
//!
//! WHY THIS EXISTS. ADR-0076 decision 1 says every row of a combined list must name its
//! source chart(s) — but until this file, `--json` was the only output that did: the
//! `MedicationList` CLI arm's plain-text branch printed a row's name, dose, status and
//! vouches, and NOTHING about which chart the row came from. The failure that surfaces:
//! an operator opens chart A (linked to chart B), sees amlodipine on the printed list — it
//! is actually recorded on B — and later runs `medication-cease A <thread>` against it,
//! which refuses with db/031's "#192 patient cannot change" error: it names the two charts
//! only as bare uuids in a thread-level message, with no hint that the list it came from
//! was combined or that the drug was never A's. The list itself had the information
//! (`MedicationRow::source_charts`) and never said it.
//!
//! WHY A SEPARATE PURE MODULE RATHER THAN INLINE STRINGS IN `main.rs`. Two properties need
//! checking independently of any database: that a NEVER-linked chart gains no header, member
//! line or row suffix from the combined read (nobody reading a single, unlinked chart should
//! see so much as one new character of it), and that a LINKED chart's header and per-row
//! suffix actually name every chart involved. Both are properties of pure string
//! formatting over `ChartSet`/`Vec<Uuid>`, so they belong in functions a unit test can
//! drive directly — not buried in a 150-line `println!` arm that only a full CLI run
//! exercises.
use crate::patient::person::ChartIdentity;
use cairn_medication_view::ChartSet;
use uuid::Uuid;

/// The list-level header naming the chart set a combined read covers.
///
/// `None` on a never-linked chart (`charts.is_linked()` is false) — the header is new
/// information that exists only because there is more than one chart to name; printing it
/// unconditionally would mean EVERY chart's output gained a line it did not have before
/// R1, not just a linked one's. `main.rs` prints nothing at all when this returns `None`,
/// which is what keeps the combined read from adding anything to a never-linked chart's output.
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
/// `None` on a never-linked list, for the same nothing-added reason as
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

/// One line per member chart of a combined list, naming its identity STATE — printed under
/// [`combined_list_header`].
///
/// WHY THE CLI NEEDS IT. A link this node's hard veto doubts still combines the read, and both
/// charts then read `under-review`; the window shows that on each member line, and this is the
/// CLI's equivalent. Without it an operator reading `medication-list` sees "combined list
/// across 2 linked charts" and nothing saying the combination is itself in question. Names
/// and dates are deliberately NOT printed: the header's job is the state of the link, and the
/// CLI has never printed demographics on this verb. A chart this node does not hold says so
/// (its `unknown` state would otherwise read as an unexplained gap).
pub fn member_lines(members: &[ChartIdentity]) -> Vec<String> {
    members
        .iter()
        .map(|m| {
            let not_held = if m.held {
                ""
            } else {
                " (chart not yet received on this node)"
            };
            format!("  chart {}: identity {}{not_held}", m.patient_id, m.trust)
        })
        .collect()
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

    /// The nothing-added guarantee: a never-linked chart gets no header and no suffix at
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

    /// Each member names its own state, and a chart not held here says why it is `unknown`.
    #[test]
    fn member_lines_name_each_charts_identity_state() {
        let member = |n: u128, held: bool, trust: &str| ChartIdentity {
            patient_id: u(n),
            held,
            name: Some("never printed".into()),
            birth_date: None,
            trust: trust.into(),
        };
        let lines = member_lines(&[member(1, true, "under-review"), member(2, false, "unknown")]);
        assert_eq!(
            lines,
            vec![
                format!("  chart {}: identity under-review", u(1)),
                format!(
                    "  chart {}: identity unknown (chart not yet received on this node)",
                    u(2)
                ),
            ]
        );
        assert!(lines.iter().all(|l| !l.contains("never printed")));
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
