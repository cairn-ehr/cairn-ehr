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
