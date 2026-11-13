//! Pure tests for `med_list_versions.rs`: which versions a Sign takes with which line, the
//! outcome's words for them, what "From:" stamps on a line, and which charted lines a version may
//! be set aside beside. No database; the DB-gated walks are `med_list_versions_db_tests.rs`.
use super::*;
use cairn_medication_view::fixtures::sample_chart;
use cairn_medication_view::{MedicationRow, MedicationStatus, MemberVouch, VouchState};
use cairn_node::allergy::check::MOIETY_SYSTEM;

/// A finished draft line coded to moiety `code` (one part), set aside of `mark` when given.
fn line(id: u128, code: u128, mark: Option<String>) -> DraftLine {
    DraftLine {
        line_id: Uuid::from_u128(id),
        terminal_id: "pc-1".into(),
        seq: id as i32,
        charted_as: "pre-existing".into(),
        line_text: format!("line {id}"),
        parts: serde_json::json!([{
            "term": "metoprolol",
            "coding": {
                "system": MOIETY_SYSTEM,
                "code": Uuid::from_u128(code).to_string(),
                "display": "metoprolol",
            },
        }]),
        reason: None,
        hand_ticked: false,
        ticked_against: vec![],
        finished: true,
        saved_at: "2026-11-13 10:00:00+00".into(),
        typed_drug: "metoprolol".into(),
        directions: "50 mg bd".into(),
        held: false,
        info_source: None,
        source_detail: None,
        why: None,
        set_aside_of: mark,
        restores: None,
    }
}

fn of_line(id: u128) -> Option<String> {
    Some(format!("line:{}", Uuid::from_u128(id)))
}

#[test]
fn the_outcome_counts_versions_and_names_each_kept() {
    assert_eq!(
        outcome_words(3, 3, 0, &[]),
        "Charted 3 of 3",
        "slice 1's words"
    );
    assert_eq!(
        outcome_words(2, 3, 1, &[]),
        "Charted 2 of 3; 1 version(s) set aside"
    );
    assert_eq!(
        outcome_words(
            1,
            2,
            2,
            &[
                ("Lipitor 25 mg".into(), VERSION_KEPT),
                ("atenolol 50 mg".into(), "the line it was set aside beside is no longer the same drug"),
            ]
        ),
        "Charted 1 of 2; 2 version(s) set aside\n\
         Lipitor 25 mg — version kept: its line was not charted\n\
         atenolol 50 mg — version kept: the line it was set aside beside is no longer the same drug"
    );
    assert_eq!(VERSION_KEPT, "its line was not charted");
}

/// A version whose companion is not ticked (or not signed) stays in the list; one whose mark no
/// longer holds stays with its reason; a ticked companion takes the versions naming it.
#[test]
fn a_versions_companion_unticked_keeps_it() {
    let lines = [
        line(1, 0xA, None),
        line(2, 0xA, of_line(1)),
        line(3, 0xB, None),
        line(4, 0xB, of_line(3)),
        // Marked beside line 1, but it is now another drug (Review Focus 2).
        line(5, 0xC, of_line(1)),
    ];
    let ticked = BTreeSet::from([Uuid::from_u128(1)]);
    let (with, alone, kept) = group_versions(&lines, &ticked, &[]);
    assert_eq!(
        with,
        BTreeMap::from([(Uuid::from_u128(1), vec![Uuid::from_u128(2)])])
    );
    assert!(alone.is_empty());
    assert_eq!(
        kept,
        vec![
            (Uuid::from_u128(4), VERSION_KEPT),
            (
                Uuid::from_u128(5),
                "the line it was set aside beside is no longer the same drug"
            ),
        ]
    );
}

#[test]
fn group_versions_sends_a_charted_companions_versions_alone() {
    let med = Uuid::from_u128(0x99);
    let code = Uuid::from_u128(0xA).to_string();
    let lines = [line(2, 0xA, Some(format!("med:{med}")))];
    let (with, alone, kept) = group_versions(&lines, &BTreeSet::new(), &[(med, code.as_str())]);
    assert!(with.is_empty());
    assert_eq!(alone, vec![(Uuid::from_u128(2), med)]);
    assert!(kept.is_empty());
    // The charted line stopped since: no longer a candidate, so the version stays.
    let (_, alone, kept) = group_versions(&lines, &BTreeSet::new(), &[]);
    assert!(alone.is_empty());
    assert_eq!(kept.len(), 1);
}

/// A version still being typed (never finished) signs neither with its ticked companion nor
/// alone: it would freeze a half-typed figure no source gave, and a version has no tick to guard
/// it the way an unfinished line's default tick does.
#[test]
fn an_unfinished_version_is_kept_not_signed() {
    let med = Uuid::from_u128(0x99);
    let code = Uuid::from_u128(0xA).to_string();
    let mut beside_draft = line(2, 0xA, of_line(1));
    beside_draft.finished = false;
    let mut beside_charted = line(3, 0xA, Some(format!("med:{med}")));
    beside_charted.finished = false;
    let lines = [line(1, 0xA, None), beside_draft, beside_charted];
    let ticked = BTreeSet::from([Uuid::from_u128(1)]);
    let (with, alone, kept) = group_versions(&lines, &ticked, &[(med, code.as_str())]);
    assert!(with.is_empty(), "{with:?}");
    assert!(alone.is_empty(), "{alone:?}");
    assert_eq!(
        kept,
        vec![
            (Uuid::from_u128(2), VERSION_UNFINISHED),
            (Uuid::from_u128(3), VERSION_UNFINISHED),
        ]
    );
}

/// "From:" stamps its category and names the document; a blank detail names the preset; unset
/// (or a label not in the list) stamps nothing, so the press's default stands.
#[test]
fn from_stamps_the_category_and_names_the_preset_when_blank() {
    let from = |p: &str, d: Option<&str>| FromView {
        preset: Some(p.into()),
        detail: d.map(Into::into),
    };
    assert_eq!(
        stamp(&from("GP letter", Some("dated Aug 2025"))),
        Some(("external-record", "dated Aug 2025".into()))
    );
    assert_eq!(
        stamp(&from("Packets / Webster pack seen", Some("  "))),
        Some(("clinician-observed", "Packets / Webster pack seen".into()))
    );
    assert_eq!(
        stamp(&from("Other", None)),
        Some(("unknown", "Other".into()))
    );
    let unset = FromView {
        preset: None,
        detail: Some("GP letter".into()),
    };
    assert_eq!(
        stamp(&unset),
        None,
        "the category is never guessed from the text"
    );
}

/// A row as the list reads it: active, coded to `code`, one member thread on `chart`.
fn row(group: u128, code: u128, chart: Uuid) -> MedicationRow {
    let mut r = sample_chart().rows[0].clone();
    r.group_id = Uuid::from_u128(group);
    r.status = MedicationStatus::Active;
    r.holds = vec![];
    r.coding_system = Some(MOIETY_SYSTEM.into());
    r.coding_code = Some(Uuid::from_u128(code).to_string());
    r.members = vec![MemberVouch {
        medication_id: Uuid::from_u128(group),
        vouch: VouchState::Absent,
        patient_id: chart,
    }];
    r
}

/// Spec §2: a version is set aside only beside a line of the OPENED chart, never a linked one's.
#[test]
fn charted_companions_are_the_opened_charts_current_lines_only() {
    let (opened, linked) = (Uuid::from_u128(0x0E), Uuid::from_u128(0x1E));
    let mut stopped = row(3, 0xA, opened);
    stopped.status = MedicationStatus::Ceased;
    let rows = [row(1, 0xA, opened), row(2, 0xA, linked), stopped];
    assert_eq!(
        charted_companions(&rows, opened),
        vec![(Uuid::from_u128(1), Uuid::from_u128(0xA).to_string())]
    );
}

/// A line whose coding conflicts, a wrong-chart hazard, and one coded under another system (or
/// to a code that is not a uuid) are never offered: Sign would sign a version into a thread the
/// floor checks only for being on this chart.
#[test]
fn a_conflicted_hazard_or_non_moiety_line_is_never_a_companion() {
    let opened = Uuid::from_u128(0x0E);
    let mut conflict = row(1, 0xA, opened);
    conflict.coding_conflict = true;
    let mut hazard = row(2, 0xA, opened);
    hazard.cross_patient = true;
    let mut other_system = row(3, 0xA, opened);
    other_system.coding_system = Some("drugref-class".into());
    let mut not_a_uuid = row(4, 0xA, opened);
    not_a_uuid.coding_code = Some("not-a-uuid".into());
    let rows = [
        conflict,
        hazard,
        other_system,
        not_a_uuid,
        row(5, 0xA, opened),
    ];
    assert_eq!(
        charted_companions(&rows, opened),
        vec![(Uuid::from_u128(5), Uuid::from_u128(0xA).to_string())]
    );
}

/// The "Version of which line?" pick names each candidate as the clinician knows it: a list line
/// by its words, a charted line by its drug and directions — so regular and PRN oxycodone can be
/// told apart (design §3.5).
#[test]
fn the_pick_names_each_candidate_line() {
    let opened = Uuid::from_u128(0x0E);
    let mut regular = row(1, 0xA, opened);
    regular.coding_display = Some("oxycodone".into());
    regular.sig = Some("10 mg bd".into());
    let mut prn = row(2, 0xA, opened);
    prn.coding_display = Some("oxycodone".into());
    prn.sig = Some("5 mg q4h prn".into());
    let words = charted_words(&[regular, prn], opened);
    assert_eq!(
        words,
        [
            (Uuid::from_u128(1), "oxycodone 10 mg bd".to_string()),
            (Uuid::from_u128(2), "oxycodone 5 mg q4h prn".to_string()),
        ]
    );
    let lines = [line(7, 0xA, None)];
    let marks = [
        format!("med:{}", Uuid::from_u128(2)),
        format!("line:{}", Uuid::from_u128(7)),
        format!("med:{}", Uuid::from_u128(9)),
    ];
    assert_eq!(
        companion_words(&marks, &lines, &words),
        [
            "charted: oxycodone 5 mg q4h prn".to_string(),
            "on this list: line 7".to_string(),
            marks[2].clone(),
        ]
    );
}

/// A version judged from `line` as Sign rebuilds it (signable as typed, nothing read).
fn judged_version(line: DraftLine) -> Judged {
    Judged {
        signable: Ok((line.typed_drug.clone(), vec![])),
        line,
        parts: vec![],
        read: cairn_node::medication::line::Directions::default(),
        check: cairn_node::allergy::check::LineCheck::default(),
        duplicate: false,
        duplicate_of: None,
        default_ticked: false,
        offers: vec![],
    }
}

/// A version nobody named a source for signs `unknown`, never the press's default: a version was
/// never charted, so "Start" (clinician-observed) or "Already taking" (patient-reported) says
/// nothing about who reported the figure the clinician set aside (principle 4, ADR-0102 item 4).
/// A source the line carries ("From:" stamped, or its own) still signs as given.
#[test]
fn a_version_with_no_source_signs_unknown_not_the_press_default() {
    for press in ["started-here", "pre-existing"] {
        let mut l = line(2, 0xA, of_line(1));
        l.charted_as = press.into();
        let j = judged_version(l);
        let v = version_to_sign(&j).expect("signable");
        assert_eq!(v.drug.info_source, "unknown", "{press}");
    }
    let mut l = line(2, 0xA, of_line(1));
    l.info_source = Some("external-record".into());
    let j = judged_version(l);
    assert_eq!(
        version_to_sign(&j).unwrap().drug.info_source,
        "external-record"
    );
}
