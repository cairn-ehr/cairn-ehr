//! One drug line of the med-list view: what a single row of the table says.
//!
//! Kept apart from `view.rs` so that the two questions this crate
//! answers stay in two places a reviewer can hold at once: HERE, what one line says about one
//! drug — its name, its dose, whose signature it carries, what is wrong with it; in `view.rs`,
//! what the chart as a whole says — how many threads the gesture signs, and what the chart
//! cannot show.
//!
//! The one thing a row does NOT decide for itself is whether the gesture will sign it. That
//! comes in as `targets`, the set `view::build_view` computed with ONE call to the shared
//! `sign_off_targets` rule — the rule the node's orchestrator uses — so the badge on a row and
//! the count on the button can never disagree (see the `view` module doc).
use cairn_medication_view::{short_kid, MedicationRow, MedicationStatus, VouchState};
use serde::Serialize;
use std::collections::HashSet;
use uuid::Uuid;

/// One rendered drug line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MedListRowView {
    /// Stable id for the DOM and for the cease command.
    pub group_id: String,
    /// The drug's coded name when it has one, else the term exactly as asserted.
    pub primary: String,
    /// Dose as "500 mg", or an explicit statement that none was recorded.
    pub dose: String,
    pub formulation: String,
    pub sig: String,
    pub started: String,
    /// "current" or "ceased".
    pub status_label: String,
    /// Whose signature this line carries, and whether it is out of date.
    pub vouch_label: String,
    /// True when the sign-off gesture will sign this row. Derived from the SAME
    /// `sign_off_targets` the orchestrator uses — never recomputed here.
    pub will_be_signed: bool,
    /// False for an already-ceased drug.
    pub can_cease: bool,
    /// Advisory worklist labels (duplicate suspicion, anchor conflict, wrong-chart hazard).
    pub flags: Vec<String>,
    /// Which chart(s) this drug was recorded on — the ids joined by ", " — and `Some` ONLY
    /// when the list is a combined read over linked charts (ADR-0076 decision 1: "every row
    /// names its source chart(s)"; two paper folders clipped together, and the clinician
    /// reads both). On a never-linked chart it is `None`, so that chart gains no label — the
    /// label is information only where there is a choice of folder.
    pub source: Option<String>,
}

/// Shown instead of a blank cell. Principle 4: an unrecorded dose is a recordable state,
/// and a blank would read either as "no dose" or as a rendering bug.
const DOSE_UNKNOWN: &str = "dose not recorded";

/// Build one row's view from one chart row.
///
/// `targets` is the set of THREAD ids the sign-off gesture will sign, computed once for the
/// whole chart by the caller. A row is "will be signed" when any of its member threads is in
/// it — a reconciled group is one row but several threads.
///
/// `linked` is `PatientMedicationList::charts.is_linked()`, decided once per chart by the
/// caller: whether this row must name its source chart(s) (see [`MedListRowView::source`]).
pub(crate) fn build_row(
    row: &MedicationRow,
    targets: &HashSet<Uuid>,
    linked: bool,
) -> MedListRowView {
    MedListRowView {
        group_id: row.group_id.to_string(),
        primary: row.display_name().to_string(),
        dose: match (&row.dose_amount, &row.dose_unit) {
            (Some(amount), Some(unit)) => format!("{amount} {unit}"),
            (Some(amount), None) => amount.clone(),
            _ => DOSE_UNKNOWN.to_string(),
        },
        formulation: row.formulation.clone().unwrap_or_default(),
        sig: row.sig.clone().unwrap_or_default(),
        started: row.started_value.clone().unwrap_or_default(),
        status_label: match row.status {
            MedicationStatus::Active => "current".into(),
            MedicationStatus::Ceased => "ceased".into(),
        },
        vouch_label: vouch_label(row),
        will_be_signed: row
            .members
            .iter()
            .any(|m| targets.contains(&m.medication_id)),
        can_cease: row.status == MedicationStatus::Active,
        flags: flags(row),
        source: linked.then(|| source_label(&row.source_charts)),
    }
}

/// A linked row whose `source_charts` came back empty — reachable only through the race
/// documented on `MedicationRow::source_charts` (a group-chart read landing between two of
/// `list_patient_medications`'s several statements), never in normal operation. Rendered
/// explicitly rather than as an empty string: `Some("")` is what `linked.then(||
/// source_label(...))` used to produce here, and `main.js` treats an empty string as
/// falsy — so the row's provenance label would silently vanish from the screen with
/// nothing to say anything had gone wrong. Principle 4: acknowledged uncertainty ("we do
/// not know") must always outrank a silent gap that reads as "nothing to say".
const SOURCE_UNREAD: &str = "(chart not read)";

/// The source-chart label: every chart owning a member thread of the group, in the row's own
/// (sorted) order. ALL of them, not just one: a group spanning two charts was recorded on
/// both, and naming only the first would hide the second folder.
fn source_label(charts: &[Uuid]) -> String {
    if charts.is_empty() {
        return SOURCE_UNREAD.to_string();
    }
    charts
        .iter()
        .map(Uuid::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whose signature this line carries.
///
/// A reconciled group has several member threads, which can disagree. The honest summary
/// names the worst state rather than picking one member's — a group is not signed off
/// until every member is.
fn vouch_label(row: &MedicationRow) -> String {
    let unsigned = row
        .members
        .iter()
        .filter(|m| m.vouch == VouchState::Absent)
        .count();
    let stale: Vec<&str> = row
        .members
        .iter()
        .filter_map(|m| match &m.vouch {
            VouchState::Stale { by } => Some(by.as_str()),
            _ => None,
        })
        .collect();
    if unsigned > 0 {
        // No signature at all is the worse state of the two, so it wins the summary: a
        // group reported as "signed but out of date" reads as needing a refresh, while an
        // unsigned member has never been vouched by anyone.
        return "not signed".to_string();
    }
    if let Some(by) = stale.first() {
        return format!("signed by {} — out of date", short_kid(by));
    }
    match row.members.first().and_then(|m| m.vouch.attester()) {
        Some(by) => format!("signed by {}", short_kid(by)),
        // No members at all. Not expected from the read path, but "not signed" is the
        // honest reading of "nothing here vouches for this line".
        None => "not signed".to_string(),
    }
}

fn flags(row: &MedicationRow) -> Vec<String> {
    let mut out = Vec::new();
    if row.reconciliation_flagged {
        out.push("possible duplicate — not yet reconciled".to_string());
    }
    if row.coding_conflict {
        out.push("two different drug identities in this group".to_string());
    }
    if row.cross_patient {
        // Per-row, and in the row's own words, because this is where the clinician is
        // looking when they wonder why the line has no signature badge. The message names
        // the DOSE risk specifically: the displayed dose comes from a whole-group pick that
        // ignores patient, so it may be the other patient's (issue #334).
        out.push(
            "shared with another patient's record — the dose shown may not be this \
             patient's, so this line cannot be signed"
                .to_string(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::test_rows::{chart, member, row};
    use crate::view::build_view;
    use cairn_medication_view::{MedicationStatus, VouchState};

    #[test]
    fn a_coded_drug_displays_its_coded_name_over_the_free_text_term() {
        let mut r = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        r.term = "little white pill".into();
        r.coding_display = Some("metformin hydrochloride".into());
        let view = build_view(&chart(vec![r]));
        assert_eq!(view.rows[0].primary, "metformin hydrochloride");
    }

    /// Principle 4: a vague term is a legitimate recorded value, never blanked out.
    #[test]
    fn an_uncoded_drug_displays_its_free_text_term_unaltered() {
        let mut r = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        r.term = "little white pill".into();
        let view = build_view(&chart(vec![r]));
        assert_eq!(view.rows[0].primary, "little white pill");
    }

    #[test]
    fn a_fresh_vouch_names_its_signatory() {
        let rows = vec![row(
            1,
            MedicationStatus::Active,
            vec![member(
                1,
                VouchState::Fresh {
                    by: "abcdef0123456789".into(),
                },
            )],
        )];
        let view = build_view(&chart(rows));
        assert!(
            view.rows[0].vouch_label.contains("abcdef01"),
            "the clinician must see WHOSE signature it is: {}",
            view.rows[0].vouch_label
        );
    }

    #[test]
    fn a_stale_vouch_says_so() {
        let rows = vec![row(
            1,
            MedicationStatus::Active,
            vec![member(
                1,
                VouchState::Stale {
                    by: "abcdef0123456789".into(),
                },
            )],
        )];
        let view = build_view(&chart(rows));
        assert!(
            view.rows[0].vouch_label.contains("out of date"),
            "got: {}",
            view.rows[0].vouch_label
        );
    }

    #[test]
    fn a_ceased_row_is_shown_marked_and_never_targeted() {
        let rows = vec![row(
            1,
            MedicationStatus::Ceased,
            vec![member(1, VouchState::Absent)],
        )];
        let view = build_view(&chart(rows));
        assert_eq!(view.rows.len(), 1, "a struck line stays on the chart");
        assert_eq!(view.rows[0].status_label, "ceased");
        assert!(!view.rows[0].will_be_signed);
        assert!(
            !view.rows[0].can_cease,
            "a ceased drug cannot be ceased again"
        );
    }

    #[test]
    fn advisory_flags_are_surfaced_as_row_labels() {
        let mut r = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        r.reconciliation_flagged = true;
        r.coding_conflict = true;
        let view = build_view(&chart(vec![r]));
        assert_eq!(view.rows[0].flags.len(), 2, "got: {:?}", view.rows[0].flags);
    }

    #[test]
    fn the_dose_reads_as_amount_and_unit() {
        let view = build_view(&chart(vec![row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        )]));
        assert_eq!(view.rows[0].dose, "500 mg");
    }

    /// Principle 4 again: an unknown dose is shown as unknown, never as a blank that reads
    /// like "no dose" or as a fabricated default.
    #[test]
    fn an_absent_dose_is_shown_as_unknown() {
        let mut r = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        r.dose_amount = None;
        r.dose_unit = None;
        assert_eq!(
            build_view(&chart(vec![r])).rows[0].dose,
            "dose not recorded"
        );
    }

    /// The withheld line is SHOWN — hiding a drug is the worse failure — but it is not a
    /// sign-off target, and the row has to say so where the clinician is looking.
    #[test]
    fn a_cross_patient_line_is_shown_flagged_and_not_signed() {
        let mut r = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        r.cross_patient = true;
        let view = build_view(&chart(vec![r]));
        assert_eq!(view.rows.len(), 1, "the drug must still be visible");
        assert!(!view.rows[0].will_be_signed);
        assert!(
            view.rows[0]
                .flags
                .iter()
                .any(|f| f.contains("another patient")),
            "the row must say why it cannot be signed: {:?}",
            view.rows[0].flags
        );
    }

    /// A group is not signed off until every member is: the summary names the WORST member
    /// state, so a half-signed reconciled pair never reads as done.
    #[test]
    fn a_partly_signed_group_does_not_read_as_signed() {
        let reconciled = row(
            1,
            MedicationStatus::Active,
            vec![
                member(1, VouchState::Fresh { by: "dr_b".into() }),
                member(2, VouchState::Absent),
            ],
        );
        let view = build_view(&chart(vec![reconciled]));
        assert_eq!(view.rows[0].vouch_label, "not signed");
        assert!(view.rows[0].will_be_signed);
    }

    // ---- ADR-0076 decision 1: every row of a combined list names its source chart(s).

    /// A linked list is two paper folders clipped together, and the clinician reads BOTH — so
    /// every line says which folder it came from. A never-linked chart says nothing extra:
    /// it reads exactly as it did before R1 (the plan's cognitive-load budget).
    #[test]
    fn a_linked_list_labels_each_row_with_its_source() {
        use cairn_medication_view::{fixtures::sample_chart, ChartSet};
        use uuid::Uuid;

        let opened = Uuid::from_u128(1); // the fixture chart
        let linked = Uuid::from_u128(0xB);

        // Unlinked: no row carries a label, whatever its sources.
        let single = build_view(&sample_chart());
        assert!(
            single.rows.iter().all(|r| r.source.is_none()),
            "a never-linked chart reads exactly as before: {:?}",
            single.rows
        );
        assert_eq!(single.charts, vec![opened.to_string()]);

        // Linked: the relabelled row names the OTHER chart; every row names its own.
        let mut list = sample_chart();
        list.charts = ChartSet::new([opened, linked]).unwrap();
        list.rows[0].source_charts = vec![linked];
        let view = build_view(&list);
        assert_eq!(view.rows[0].source, Some(linked.to_string()));
        for r in &view.rows[1..] {
            let source = r
                .source
                .as_deref()
                .expect("every row of a linked list is labelled");
            assert!(!source.contains(&linked.to_string()), "{source}");
            assert!(source.contains(&opened.to_string()), "{source}");
        }
        // A group spanning two charts names both, in the row's own (sorted) order.
        let cross = view.rows.last().unwrap();
        assert_eq!(
            cross.source,
            Some(format!("{}, {}", opened, Uuid::from_u128(2)))
        );
        assert_eq!(view.charts, vec![opened.to_string(), linked.to_string()]);
    }

    /// The race case named on `MedicationRow::source_charts` itself: a group-chart read
    /// landing between the read model's several statements can leave a row on a LINKED
    /// list with an empty `source_charts`. Before this fix, `linked.then(|| source_label(…))`
    /// turned that into `Some("")` — an empty string `main.js` treats as falsy, so the row
    /// silently lost its provenance label on screen with no visible sign anything was
    /// wrong. Render an explicit marker instead, so the clinician sees "we don't know",
    /// never a blank that reads as "nothing to say" (principle 4: acknowledged uncertainty
    /// beats a silent gap).
    #[test]
    fn a_linked_row_with_no_source_charts_says_so_rather_than_going_silent() {
        let mut r = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        r.source_charts = vec![];
        let view = super::build_row(&r, &std::collections::HashSet::new(), true);
        assert_eq!(
            view.source,
            Some("(chart not read)".to_string()),
            "an empty source_charts on a linked list must say so explicitly, not render as \
             an empty (falsy) string"
        );
    }
}
