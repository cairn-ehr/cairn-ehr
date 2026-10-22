//! Tests for `medication/list_text.rs` (moved out of line to keep that file under 500 lines):
//! the CLI wording of each wrong-chart reason, each with its own remedy.
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

#[test]
fn a_doubted_link_row_names_the_link_judgement_not_separation() {
    let text = row_hazard_lines(&list_row(1, false, true), &BTreeMap::new()).join("\n");
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
    let targets = BTreeMap::from([(u(1), vec![u(1), u(2)])]);
    assert_eq!(
        row_hazard_lines(&list_row(1, true, false), &targets),
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

/// An ACTIVE, unsigned list row withheld for the given reasons (only the fields
/// `withheld_rows` reads matter).
fn list_row(group: u128, outside_set: bool, doubted_link: bool) -> MedicationRow {
    let wrong_chart = WrongChartReasons {
        outside_set,
        doubted_link,
        patient_conflict: false,
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
        members: vec![cairn_medication_view::MemberVouch {
            medication_id: u(group),
            vouch: cairn_medication_view::VouchState::Absent,
            patient_id: u(2),
        }],
        reconciliation_flagged: false,
        coding_conflict: false,
        cross_patient: wrong_chart.any(),
        wrong_chart,
        source_charts: vec![],
        holds: vec![],
        cessation_reason: None,
    }
}

#[test]
fn the_doubted_link_note_is_printed_once_for_the_whole_list() {
    let rows = [list_row(1, false, true), list_row(2, true, true)];
    let note = doubted_link_note(&rows).expect("two doubted rows: one note");
    assert_eq!(note.matches(DOUBTED_LINK_INSTRUCTION).count(), 1, "{note}");
    assert!(note.starts_with("! "), "{note}");
    // #803: "withheld" now means a clinical hold, so this note says "left out".
    assert!(note.contains("are left out of sign-off"), "{note}");
}

/// Residual R2: a ceased line needs no signature, so it is never withheld — a doubted
/// record whose only off-chart line is ceased must not claim lines are withheld.
#[test]
fn a_ceased_doubted_row_alone_means_no_note() {
    let mut ceased = list_row(1, false, true);
    ceased.status = cairn_medication_view::MedicationStatus::Ceased;
    assert_eq!(doubted_link_note(&[ceased]), None);
}

/// PR #717 review: a doubted row that sign-off does NOT withhold (ceased, or already
/// signed before the doubt arose) gets no note below the list — so its own line must not
/// point at one, nor claim it "CANNOT be signed" (there is nothing on it to sign). The doubt
/// itself is still said: the line is still not vouched to be this patient's.
#[test]
fn a_doubted_row_nothing_withholds_does_not_point_at_a_note() {
    let mut ceased = list_row(1, false, true);
    ceased.status = cairn_medication_view::MedicationStatus::Ceased;
    let mut signed = list_row(2, false, true);
    signed.members[0].vouch = cairn_medication_view::VouchState::Fresh { by: "dr_b".into() };
    for row in [ceased, signed] {
        let text = row_hazard_lines(&row, &BTreeMap::new()).join("\n");
        assert!(text.contains("this record's links are in doubt"), "{text}");
        assert!(!text.contains("see the note below"), "{text}");
        assert!(!text.contains("CANNOT be signed"), "{text}");
        assert_eq!(doubted_link_note(std::slice::from_ref(&row)), None);
    }
}

#[test]
fn a_row_that_is_no_hazard_prints_no_warning() {
    assert!(row_hazard_lines(&list_row(1, false, false), &BTreeMap::new()).is_empty());
}

/// The report prints one block per reason, so a withheld line whose reason set is empty
/// must still be counted somewhere — worded as the pre-#697 outside case — rather than
/// vanishing from a report that then reads as complete.
#[test]
fn a_withheld_line_with_no_recorded_reason_is_still_reported() {
    let line = WithheldLine {
        group_id: u(1),
        reasons: WrongChartReasons::default(),
    };
    let text = withheld_signoff_lines(&[line], &BTreeMap::new()).join("\n");
    assert!(
        text.contains("1 medication line(s) still need a signature"),
        "{text}"
    );
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
    let lines = row_hazard_lines(&list_row(1, true, true), &targets);
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
            patient_conflict: false,
        },
    };
    let text = withheld_signoff_lines(&[w(1, true, false), w(2, false, true)], &BTreeMap::new())
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

/// #836: a line a synced event naming another patient's chart landed on says that cause and
/// its own remedy (stop and record again), never separation or the link judgement.
fn conflict_row(group: u128) -> MedicationRow {
    let mut row = list_row(group, false, false);
    row.wrong_chart.patient_conflict = true;
    row.cross_patient = true;
    row
}

#[test]
fn a_conflict_row_names_its_cause_and_remedy() {
    let text = row_hazard_lines(&conflict_row(1), &BTreeMap::new()).join("\n");
    assert!(text.contains("another patient's chart"), "{text}");
    assert!(text.contains(CONFLICT_INSTRUCTION), "{text}");
    assert!(!text.contains("medication-separate"), "{text}");
    assert!(!text.contains("in doubt"), "{text}");
}

#[test]
fn a_signoff_report_words_a_conflict_with_its_own_remedy() {
    let line = WithheldLine {
        group_id: u(1),
        reasons: conflict_row(1).wrong_chart,
    };
    let text = withheld_signoff_lines(&[line], &BTreeMap::new()).join("\n");
    assert!(
        text.contains("1 medication line(s) still need a signature but were NOT signed"),
        "{text}"
    );
    assert!(text.contains(CONFLICT_INSTRUCTION), "{text}");
    assert!(
        text.contains(&u(1).to_string()),
        "the line is named: {text}"
    );
    assert!(!text.contains("medication-separate"), "{text}");
}
