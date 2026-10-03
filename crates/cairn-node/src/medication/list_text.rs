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
//!
//! The withheld-line wording lives here for the same reason: when #697 split "withheld" into two
//! reasons (a group spanning patients, a record whose links are in doubt), each needed its own
//! sentence AND its own remedy, and that branching is string logic a unit test can drive
//! without a database.
use crate::patient::person::ChartIdentity;
use cairn_medication_view::{
    format_hazard_groups, ChartSet, MedicationRow, WithheldLine, WrongChartReasons,
    DOUBTED_LINK_INSTRUCTION, SEPARATION_INSTRUCTION,
};
use std::collections::BTreeMap;
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
/// CLI has never printed demographics on this verb. A chart whose registration this node does
/// not hold says so (its `unknown` state would otherwise read as an unexplained gap).
pub fn member_lines(members: &[ChartIdentity]) -> Vec<String> {
    members
        .iter()
        .map(|m| {
            let not_held = if m.held {
                ""
            } else {
                " (registration not yet received on this node)"
            };
            format!("  chart {}: identity {}{not_held}", m.patient_id, m.trust)
        })
        .collect()
}

/// The CLI's warning lines under one withheld row of `medication-list` — one per reason (#697),
/// then the group's member threads once. For a line reaching outside the set those threads are
/// the separation remedy's arguments (this row lists only this record's half of a cross-patient
/// group, #338 finding 1); for a doubted-link line they only say which threads are held — that
/// remedy is a judgement of the links and takes CHART ids, which the list header names.
///
/// The outside-set warning carries its remedy inline. The doubted-link warning carries its
/// CAUSE only and points below the list: its remedy ([`DOUBTED_LINK_INSTRUCTION`]) is one
/// for the whole record, not per line, so [`doubted_link_note`] prints it ONCE — repeating its
/// ~1000 characters under every withheld row buried the list it was explaining (final review).
pub fn row_hazard_lines(
    why: WrongChartReasons,
    group: Uuid,
    targets: &BTreeMap<Uuid, Vec<Uuid>>,
) -> Vec<String> {
    let mut out = Vec::new();
    if why.outside_set {
        out.push(format!(
            "    ! this group's member threads span more than one patient — the dose shown may \
             belong to the other patient, so this line CANNOT be signed off (issue #334). {}",
            SEPARATION_INSTRUCTION
        ));
    }
    if why.doubted_link {
        out.push(DOUBTED_ROW_LINE.to_string());
    }
    if why.any() {
        out.push(format!("      {}", format_hazard_groups(&[group], targets)));
    }
    out
}

/// The doubted-link warning under one row: the cause, true for every line it is printed under.
/// "Cannot yet vouch that it is this patient's" rather than "may be another person's": the
/// line's own chart may be human-linked to the opened one while ANOTHER link is doubted. "Until
/// the links are no longer in doubt" rather than "until they are judged": in the A–C–X bridge
/// every link can have a human judgement and the record still holds a doubt (db/054 case (c)).
const DOUBTED_ROW_LINE: &str =
    "    ! this record's links are in doubt, and this line is not recorded only on the chart you \
     opened — the node cannot yet vouch that it is this patient's, so it CANNOT be signed off \
     until the record's links are no longer in doubt — see the note below the list (issue #697).";

/// The ONE note printed after `medication-list`'s rows when any row is withheld for a doubted
/// link, carrying the remedy [`row_hazard_lines`] points to; `None` when no row is (so a list
/// without a doubted link — every never-linked chart among them — gains nothing). Pure over the
/// rows, so `main.rs` only prints it.
pub fn doubted_link_note(rows: &[MedicationRow]) -> Option<String> {
    let any_doubted = rows
        .iter()
        .filter_map(MedicationRow::withheld_because)
        .any(|why| why.doubted_link);
    any_doubted.then(|| {
        format!(
            "! This record's links are in doubt, so the line(s) marked above are withheld from \
             sign-off (issue #697). {DOUBTED_LINK_INSTRUCTION}"
        )
    })
}

/// The sign-off outcome's report of lines withheld from the gesture — printed in EVERY
/// outcome, never folded into the success line: "signed off 11" over a twelfth outstanding
/// line reads as a finished chart. One block per reason, each with its own remedy (#697).
pub fn withheld_signoff_lines(
    withheld: &[WithheldLine],
    targets: &BTreeMap<Uuid, Vec<Uuid>>,
) -> Vec<String> {
    let pick = |f: fn(&WithheldLine) -> bool| -> Vec<Uuid> {
        withheld
            .iter()
            .filter(|l| f(l))
            .map(|l| l.group_id)
            .collect()
    };
    let mut out = Vec::new();
    let outside = pick(|l| l.reasons.outside_set);
    if !outside.is_empty() {
        out.push(format!(
            "! {} medication line(s) still need a signature but were NOT signed: their group's \
             member threads span more than one patient, so the dose displayed may belong to the \
             other patient (issue #334). {} Then sign off again.",
            outside.len(),
            SEPARATION_INSTRUCTION
        ));
        out.push(format!("    {}", format_hazard_groups(&outside, targets)));
    }
    let doubted = pick(|l| l.reasons.doubted_link);
    if !doubted.is_empty() {
        out.push(format!(
            "! {} medication line(s) still need a signature but were NOT signed: this record's \
             links are in doubt, and these lines are not recorded only on the chart you opened, \
             so the node cannot yet vouch that they are this patient's; they will NOT be signed \
             until the record's links are no longer in doubt (issue #697). {} Then sign off \
             again.",
            doubted.len(),
            DOUBTED_LINK_INSTRUCTION
        ));
        out.push(format!("    {}", format_hazard_groups(&doubted, targets)));
    }
    out
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
                    "  chart {}: identity unknown (registration not yet received on this node)",
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

    fn both() -> WrongChartReasons {
        WrongChartReasons {
            outside_set: true,
            doubted_link: true,
        }
    }

    #[test]
    fn a_doubted_link_row_names_the_link_judgement_not_separation() {
        let why = WrongChartReasons {
            outside_set: false,
            doubted_link: true,
        };
        let text = row_hazard_lines(why, u(1), &BTreeMap::new()).join("\n");
        assert!(text.contains("in doubt"), "{text}");
        assert!(text.contains("see the note below the list"), "{text}");
        assert!(
            !text.contains(DOUBTED_LINK_INSTRUCTION),
            "the remedy is printed once, below the list, not under every row: {text}"
        );
        assert!(!text.contains("medication-separate"), "{text}");
        assert!(!text.contains("more than one patient"), "{text}");
    }

    /// The outside-set warning is byte-for-byte what it was before the doubted-link reason
    /// existed: a list that holds no doubted link must not change by one character.
    #[test]
    fn the_outside_set_row_warning_is_unchanged() {
        let why = WrongChartReasons {
            outside_set: true,
            doubted_link: false,
        };
        let targets = BTreeMap::from([(u(1), vec![u(1), u(2)])]);
        assert_eq!(
            row_hazard_lines(why, u(1), &targets),
            vec![
                format!(
                    "    ! this group's member threads span more than one patient — the dose \
                     shown may belong to the other patient, so this line CANNOT be signed off \
                     (issue #334). {SEPARATION_INSTRUCTION}"
                ),
                format!("      group {} (member threads: {}, {})", u(1), u(1), u(2)),
            ]
        );
    }

    /// A list row withheld for the given reasons (only the fields `withheld_because` reads).
    fn list_row(group: u128, outside_set: bool, doubted_link: bool) -> MedicationRow {
        let wrong_chart = WrongChartReasons {
            outside_set,
            doubted_link,
        };
        MedicationRow {
            group_id: u(group),
            display_chart: u(1),
            term: "metformin".into(),
            coding_display: None,
            formulation: None,
            dose_amount: None,
            dose_unit: None,
            sig: None,
            started_value: None,
            started_precision: None,
            status: cairn_medication_view::MedicationStatus::Active,
            members: vec![],
            reconciliation_flagged: false,
            coding_conflict: false,
            cross_patient: wrong_chart.any(),
            wrong_chart,
            source_charts: vec![],
        }
    }

    #[test]
    fn the_doubted_link_note_is_printed_once_for_the_whole_list() {
        let rows = [list_row(1, false, true), list_row(2, true, true)];
        let note = doubted_link_note(&rows).expect("two doubted rows: one note");
        assert_eq!(note.matches(DOUBTED_LINK_INSTRUCTION).count(), 1, "{note}");
        assert!(note.starts_with("! "), "{note}");
    }

    #[test]
    fn no_doubted_row_means_no_note() {
        assert_eq!(doubted_link_note(&[]), None);
        assert_eq!(
            doubted_link_note(&[list_row(1, false, false), list_row(2, true, false)]),
            None
        );
    }

    #[test]
    fn a_row_with_both_reasons_prints_both_and_its_threads_once() {
        let targets = BTreeMap::from([(u(1), vec![u(1), u(2)])]);
        let lines = row_hazard_lines(both(), u(1), &targets);
        let text = lines.join("\n");
        assert!(
            text.contains("medication-separate") && text.contains("see the note below the list"),
            "{text}"
        );
        assert_eq!(text.matches(&u(2).to_string()).count(), 1, "{text}");
    }

    #[test]
    fn a_signoff_report_words_each_reason_with_its_own_remedy() {
        let w = |n, outside_set, doubted_link| WithheldLine {
            group_id: u(n),
            reasons: WrongChartReasons {
                outside_set,
                doubted_link,
            },
        };
        let text =
            withheld_signoff_lines(&[w(1, true, false), w(2, false, true)], &BTreeMap::new())
                .join("\n");
        assert!(
            text.contains("1 medication line(s) still need a signature but were NOT signed: their"),
            "{text}"
        );
        assert!(
            text.contains("medication-separate") && text.contains("unlink-charts"),
            "{text}"
        );
        assert!(text.contains("Then sign off again."), "{text}");
        assert!(
            text.contains("were NOT signed: this record's links are in doubt"),
            "{text}"
        );
        assert!(
            text.contains("will NOT be signed until the record's links are no longer in doubt"),
            "{text}"
        );
        assert!(!text.contains("from this chart"), "{text}");
        assert!(withheld_signoff_lines(&[], &BTreeMap::new()).is_empty());
    }
}
