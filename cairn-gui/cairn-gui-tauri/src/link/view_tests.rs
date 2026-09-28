//! Tests for `link/view.rs`, kept in a sibling file so the implementation stays under the
//! house 500-line guideline (fix round 1, task 4 review).
use super::*;
use cairn_node::patient::compare::{FieldFact, NameFact};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn held(n: u128) -> ChartFacts {
    ChartFacts {
        patient_id: id(n),
        held: true,
        trust: "confirmed".into(),
        names: vec![NameFact {
            value: "N".into(),
            use_: Some("legal".into()),
            provenance: "patient-stated".into(),
        }],
        // `precision: Some("day")` renders identically to the pre-controller-ruling
        // fixture: day precision is the case that must NOT grow the "(N precision, …)"
        // suffix (only a coarser facet does — see `a_coarse_dob_names_its_precision`).
        dob: Some(FieldFact {
            value: "1950-07-01".into(),
            provenance: "document-verified".into(),
            precision: Some("day".into()),
        }),
        ..ChartFacts::default()
    }
}

fn meds() -> MedListView {
    cairn_gui_tab_medications::view::build_view(&cairn_medication_view::fixtures::sample_chart())
}

fn parts() -> ComparisonParts {
    ComparisonParts {
        left: Ok(vec![held(1)]),
        right: Ok(vec![held(2)]),
        findings: Ok(vec![]),
        other_medications: Ok(meds()),
    }
}

#[test]
fn a_full_comparison_is_linkable_and_names_the_other_record() {
    let v = comparison_view(parts(), &ChartSet::single(id(1)), &ChartSet::single(id(2)));
    assert!(v.can_link && v.problems.is_empty());
    assert_eq!(v.left_count, 1);
    assert_eq!(v.columns.len(), 2);
    assert_eq!(
        v.other_charts,
        vec![id(2).to_string()],
        "sent back with the link"
    );
    assert_eq!(
        v.left_charts,
        vec![id(1).to_string()],
        "the compared left set, sent back with the link too"
    );
    assert!(
        v.rows.iter().all(|r| r.cells.len() == 2),
        "every row has a cell per chart"
    );
}

/// Review Focus 4: a comparison read only in part shows what it has, names what it lacks,
/// and offers NO link — a judgement needs the whole picture.
#[test]
fn a_partial_comparison_names_what_is_missing_and_cannot_link() {
    let mut p = parts();
    p.other_medications = Err("connection reset".into());
    let v = comparison_view(p, &ChartSet::single(id(1)), &ChartSet::single(id(2)));
    assert!(!v.can_link);
    assert_eq!(v.problems.len(), 1);
    assert!(
        v.problems[0].contains("medications"),
        "names the part that is missing"
    );
    assert_eq!(
        v.columns.len(),
        2,
        "the facts that WERE read are still shown"
    );
}

#[test]
fn unreadable_facts_leave_no_columns_and_cannot_link() {
    let mut p = parts();
    p.right = Err("boom".into());
    let v = comparison_view(p, &ChartSet::single(id(1)), &ChartSet::single(id(2)));
    assert!(!v.can_link);
    assert_eq!(v.left_count, 1);
    assert_eq!(
        v.columns.len(),
        1,
        "no invented column for a record that was not read"
    );
}

/// Principle 4: absence is worded — and differently for a chart this node does not hold.
#[test]
fn an_absent_fact_says_not_recorded_or_unknown() {
    let mut unheld = ChartFacts {
        patient_id: id(2),
        held: false,
        trust: "unknown".into(),
        ..ChartFacts::default()
    };
    unheld.names.clear();
    let mut p = parts();
    p.right = Ok(vec![unheld]);
    let v = comparison_view(p, &ChartSet::single(id(1)), &ChartSet::single(id(2)));
    let dob = v.rows.iter().find(|r| r.label == "Date of birth").unwrap();
    assert_eq!(dob.cells[0], "1950-07-01 (document-verified)");
    assert_eq!(dob.cells[1], "unknown — registration not yet received here");
    // Fix round 1, Important 1: an alias pool "none" is a fact only a HELD chart can state.
    // A chart this node does not hold gets the same "unknown" wording as every other absent
    // fact on it — "none" would claim knowledge this node does not have.
    let aliases = v
        .rows
        .iter()
        .find(|r| r.label == "Names struck as false")
        .unwrap();
    assert_eq!(
        aliases.cells[1],
        "unknown — registration not yet received here"
    );
    let mut p = parts();
    p.right = Ok(vec![ChartFacts {
        patient_id: id(2),
        held: true,
        trust: "confirmed".into(),
        ..ChartFacts::default()
    }]);
    let v = comparison_view(p, &ChartSet::single(id(1)), &ChartSet::single(id(2)));
    let dob = v.rows.iter().find(|r| r.label == "Date of birth").unwrap();
    assert_eq!(dob.cells[1], "not recorded");
}

/// Controller ruling (principle 4): a coarse date must never read as a precise day. Day
/// precision (and no precision facet at all, e.g. sex-at-birth) render as before; anything
/// coarser says so, so "1950" is never mistaken for a verified "1950-01-01".
#[test]
fn a_coarse_dob_names_its_precision() {
    let coarse = ChartFacts {
        patient_id: id(2),
        held: true,
        trust: "confirmed".into(),
        dob: Some(FieldFact {
            value: "1950".into(),
            provenance: "patient-stated".into(),
            precision: Some("year".into()),
        }),
        ..ChartFacts::default()
    };
    let mut p = parts();
    p.right = Ok(vec![coarse]);
    let v = comparison_view(p, &ChartSet::single(id(1)), &ChartSet::single(id(2)));
    let dob = v.rows.iter().find(|r| r.label == "Date of birth").unwrap();
    assert_eq!(dob.cells[1], "1950 (year precision, patient-stated)");
}

/// Fix round 1, Important 7: an address with no `use` facet still carries provenance —
/// every other fact row does — so its cell must not go bare.
#[test]
fn an_address_carries_provenance_with_or_without_a_use_facet() {
    use cairn_node::patient::compare::AddressFact;

    let f = ChartFacts {
        patient_id: id(1),
        held: true,
        trust: "confirmed".into(),
        addresses: vec![
            AddressFact {
                use_: Some("home".into()),
                display: "1 Main St".into(),
                provenance: "patient-stated".into(),
            },
            AddressFact {
                use_: None,
                display: "2 Other St".into(),
                provenance: "document-verified".into(),
            },
        ],
        ..ChartFacts::default()
    };
    let rows = fact_rows(&[f]);
    let addresses = rows.iter().find(|r| r.label == "Addresses").unwrap();
    assert_eq!(
        addresses.cells[0],
        "1 Main St (home, patient-stated); 2 Other St (document-verified)"
    );
}

#[test]
fn a_finding_is_a_plain_fact_naming_both_charts() {
    let f = VetoFinding {
        left: id(1),
        right: id(2),
        kind: "dob".into(),
        severity: "hard_veto".into(),
        subject: "dob".into(),
        detail: "verified dob clash (precision day): 'x' vs 'y'".into(),
    };
    let line = finding_line(&f);
    // Fix round 1, Important 2: the message must not echo `line` (patient ids + veto detail
    // are identity-built text) — a static label pins the same fact without the echo.
    assert!(
        line.starts_with("Verified facts differ"),
        "a hard veto is labelled as verified"
    );
    assert!(line.contains(&id(1).to_string()) && line.contains(&id(2).to_string()));
    assert!(
        line.contains("verified dob clash"),
        "db/016's own words, verbatim"
    );
    let hold = finding_line(&VetoFinding {
        severity: "degrade_hold".into(),
        ..f
    });
    assert!(
        hold.starts_with("Facts differ, not verified"),
        "a degrade-hold is labelled as not verified"
    );
}

/// Never "no conflicts": an empty finding list renders nothing at all.
#[test]
fn no_findings_means_no_lines_and_no_clearance_sentence() {
    let v = comparison_view(parts(), &ChartSet::single(id(1)), &ChartSet::single(id(2)));
    assert!(v.findings.is_empty());
}

#[test]
fn only_current_medications_are_listed_and_warnings_carry_over() {
    let (lines, notes) = medication_lines(&meds());
    let current = meds()
        .rows
        .iter()
        .filter(|r| r.status_label == "current")
        .count();
    assert_eq!(
        lines.len(),
        current,
        "ceased drugs are not 'active medications'"
    );
    // Fix round 1, Minor 5/6: the fixture chart carries a withheld AND a missing message
    // (cross-patient row, invisible group); both must survive, each prefixed so they read
    // as about the OTHER record rather than "this chart".
    assert!(
        notes.iter().all(|n| n.starts_with("On the other record: ")),
        "every carried-over note is prefixed: {notes:?}"
    );
    assert_eq!(
        notes.len(),
        2,
        "both the withheld and the missing note carry over: {notes:?}"
    );
}

/// Fix round 1, Minor 4: a note is already present (the list is known incomplete), so no
/// "no current medications" absence claim is added — that would contradict the note.
#[test]
fn a_note_already_present_is_not_joined_by_a_false_absence_claim() {
    let list = MedListView {
        charts: vec![id(2).to_string()],
        rows: vec![],
        sign_off_count: 0,
        sign_off_enabled: false,
        empty_message: None,
        withheld_message: Some("1 line(s) withheld.".into()),
        missing_message: None,
    };
    let (lines, notes) = medication_lines(&list);
    assert!(lines.is_empty());
    assert_eq!(
        notes,
        vec!["On the other record: 1 line(s) withheld.".to_string()]
    );
    assert!(
        !notes.iter().any(|n| n.contains("No current medications")),
        "a note already says the list is incomplete: {notes:?}"
    );
}

#[test]
fn an_empty_list_says_so() {
    let empty = cairn_gui_tab_medications::view::build_view(
        &cairn_medication_view::PatientMedicationList::empty(ChartSet::single(id(2))),
    );
    let (lines, notes) = medication_lines(&empty);
    assert!(lines.is_empty());
    assert!(
        notes.iter().any(|n| n.contains("No current medications")),
        "absence is named"
    );
}

#[test]
fn a_link_that_took_effect_reloads_and_one_outranked_does_not() {
    let set = ChartSet::new([id(1), id(2)]).unwrap();
    let took = link_report(LinkEffect::TookEffect, &set, &set);
    assert!(
        took.reload && took.sentence.starts_with("Linked"),
        "{}",
        took.sentence
    );
    assert!(took.sentence.contains("2 charts"));
    let lost = link_report(LinkEffect::Outranked, &ChartSet::single(id(1)), &set);
    assert!(!lost.reload, "a disagreement is shown, never reloaded away");
    assert!(lost.sentence.contains("NOT in effect"), "{}", lost.sentence);
    // Final review M6: pressing Link again is not a no-op — it records another judgement.
    assert!(
        lost.sentence
            .ends_with("pressing Link again records another judgement but changes nothing."),
        "{}",
        lost.sentence
    );
}

/// Final review O1: the outcome tells the truth about what the link joined. A chart the
/// record now combines that was in NEITHER compared set (a third chart already linked to one
/// side by the time the judgement landed) is named, so it is reviewed rather than assumed.
#[test]
fn a_chart_the_comparison_never_showed_is_named() {
    let compared = ChartSet::new([id(1), id(2)]).unwrap();
    let now = ChartSet::new([id(1), id(2), id(3)]).unwrap();
    let took = link_report(LinkEffect::TookEffect, &now, &compared);
    assert!(took.sentence.starts_with("Linked"), "{}", took.sentence);
    assert!(
        took.sentence.ends_with(&format!(
            "The record now also includes chart(s) {} that were not in the comparison — \
             review them.",
            id(3)
        )),
        "{}",
        took.sentence
    );
    let exact = link_report(LinkEffect::TookEffect, &compared, &compared);
    assert!(
        !exact.sentence.contains("not in the comparison"),
        "{}",
        exact.sentence
    );
}

/// Final review M4: every arm of the funnel's error classification, worded for a link. A
/// verdict (refused / not provisioned) is never retried as-is; only an outage is retryable.
#[test]
fn every_data_error_arm_is_worded_for_a_link() {
    let refused_view = link_error_from(DataError::Refused("floor says no".into()));
    assert_eq!(refused_view.retry, Retry::Never);
    assert_eq!(refused_view.text, "The link was refused: floor says no");

    let unprovisioned = link_error_from(DataError::NotProvisioned("key not held".into()));
    assert_eq!(unprovisioned.retry, Retry::AfterOperator);
    assert_eq!(
        unprovisioned.text,
        "This node cannot record the link yet: key not held"
    );
    assert!(
        !unprovisioned.text.contains("operator"),
        "the not-held arm resolves by sync, not by an operator"
    );

    let outage = link_error_from(DataError::Unavailable("connection reset".into()));
    assert_eq!(outage.retry, Retry::Now);
    assert!(outage.text.contains("not confirmed"), "{}", outage.text);

    let missing = link_error_from(DataError::NotFound);
    assert_eq!(missing.retry, Retry::Never);
    assert_eq!(missing.text, "The link was not recorded.");
}

/// Review Focus 5 (outage half): an outage is "not confirmed", never "nothing changed" — a
/// connection lost mid-commit leaves the outcome unknown.
#[test]
fn an_outage_is_worded_not_confirmed_and_retryable() {
    let outage = anyhow::anyhow!("connection reset");
    assert_eq!(link_error_view(&outage).retry, Retry::Now);
    let text = link_error_view(&outage).text;
    assert!(text.contains("not confirmed"), "{text}");
}
