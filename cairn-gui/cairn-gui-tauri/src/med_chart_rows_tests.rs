//! `med_chart`'s tests, continued: the medication list's lines with combinations
//! (`allergy_check::charted_lines`), a brand allergy on a combination, and the allergy read at
//! Sign over a changed chart set. Split from `med_chart_tests.rs` to keep each under 500 lines;
//! it shares that file's helpers (`before_key`, `line`, `part`, `coded`).
use super::*;
use crate::allergy_check::charted_lines;
use cairn_medication_view::fixtures::sample_chart;
use cairn_medication_view::{MedicationRow, MedicationStatus, MemberVouch};
use cairn_node::allergy::check::{check_medications, MedLine};
use cairn_node::allergy::drug_facts::DrugFacts;
use cairn_node::allergy::read::{AllergyRow, KeyState, PatientAllergies};
use cairn_node::medication::ChartingRow;
use std::collections::HashMap;

fn row(group: u128, term: &str, members: &[u128]) -> MedicationRow {
    let mut r = sample_chart().rows[0].clone();
    r.group_id = Uuid::from_u128(group);
    r.term = term.into();
    r.status = MedicationStatus::Active;
    r.coding_system = None;
    r.coding_code = None;
    r.coding_display = None;
    let vouch = r.members[0].clone();
    r.members = members
        .iter()
        .map(|m| MemberVouch {
            medication_id: Uuid::from_u128(*m),
            ..vouch.clone()
        })
        .collect();
    r
}

fn combo(parts: &[(&str, Option<Uuid>)]) -> ChartingRow {
    ChartingRow {
        charted_as: Some("started-here".into()),
        route: None,
        strength: None,
        components: parts
            .iter()
            .map(|(t, c)| {
                let coding = c.map(|c| (MOIETY_SYSTEM.to_string(), c.to_string(), t.to_string()));
                (t.to_string(), coding)
            })
            .collect(),
        override_reason: None,
        source_detail: None,
        why: None,
    }
}

#[test]
fn an_uncharted_list_reads_as_before() {
    let rows = [row(1, "Amoxil", &[11]), row(2, "Diabex", &[21])];
    let mut charting = HashMap::new();
    // A charted single drug (no components) is still one line.
    charting.insert(Uuid::from_u128(21), combo(&[]));
    let lines = charted_lines(&rows, &charting);
    let old = crate::allergy_check::med_lines(&rows);
    assert_eq!(lines.len(), 2);
    for (a, b) in lines.iter().zip(&old) {
        assert_eq!(
            (a.group_id, a.term, a.moiety, a.current),
            (b.group_id, b.term, b.moiety, b.current)
        );
    }
}

#[test]
fn a_combination_is_one_line_per_component_under_its_group() {
    let amox = Uuid::from_u128(0xA40C);
    let rows = [row(1, "Augmentin", &[11])];
    let charting = HashMap::from([(
        Uuid::from_u128(11),
        combo(&[("amoxicillin", Some(amox)), ("clavulanic acid", None)]),
    )]);
    let lines = charted_lines(&rows, &charting);
    assert_eq!(lines.len(), 2, "the components, not the row");
    assert!(lines
        .iter()
        .all(|l| l.group_id == Uuid::from_u128(1) && l.current));
    // R12: every part carries the LINE's term, so a by-name allergy on the brand still meets it.
    assert!(lines.iter().all(|l| l.term == "Augmentin"));
    assert_eq!(
        (lines[0].name, lines[0].moiety),
        ("amoxicillin", Some(amox))
    );
    assert_eq!(lines[0].coding_display, Some("amoxicillin"));
    assert_eq!((lines[1].name, lines[1].moiety), ("clavulanic acid", None));
}

/// A group reconciling a combination with a single drug keeps the row's own line too.
#[test]
fn a_mixed_group_keeps_its_row_line() {
    let rows = [row(1, "Augmentin", &[11, 12])];
    let charting = HashMap::from([(
        Uuid::from_u128(11),
        combo(&[("amoxicillin", None), ("clavulanic acid", None)]),
    )]);
    let lines: Vec<MedLine> = charted_lines(&rows, &charting);
    assert_eq!(lines.len(), 3);
}

fn uncoded_allergy(term: &str) -> PatientAllergies {
    let at = chrono::DateTime::from_timestamp(1_762_600_000, 0).unwrap();
    let a = AllergyRow {
        event_id: Uuid::from_u128(9),
        chart: Uuid::from_u128(1),
        allergen_kind: "drug".into(),
        term: term.into(),
        kind: "allergy".into(),
        category: "anaphylaxis".into(),
        reaction_text: None,
        onset_precision: "unknown".into(),
        onset_value: None,
        source: "patient-stated".into(),
        recorded_at: at,
        hlc: (at.timestamp_millis(), 0),
        signer: None,
        corrects: None,
        key: KeyState::Standing,
        coding: None,
    };
    PatientAllergies {
        entries: vec![a],
        none: vec![],
        challenges: vec![],
        retractions: vec![],
        doubted_charts: vec![],
        unreadable: 0,
    }
}

/// R12: an allergy recorded by name on the brand ("Augmentin", uncoded) marks a line typed
/// "Augmentin" and charted as two coded components — on the list and on the line being charted.
#[tokio::test]
async fn a_brand_allergy_marks_a_combination_typed_by_the_brand() {
    let (amox, clav) = (Uuid::from_u128(0xA40C), Uuid::from_u128(0xC1A7));
    let a = uncoded_allergy("Augmentin");
    let r = cairn_node::allergy::thread::resolve(&a);

    let rows = [row(1, "Augmentin", &[11])];
    let charting = HashMap::from([(
        Uuid::from_u128(11),
        combo(&[("amoxicillin", Some(amox)), ("clavulanic acid", Some(clav))]),
    )]);
    let checks = check_medications(
        &r,
        &charted_lines(&rows, &charting),
        &DrugFacts::Unavailable,
        false,
    );
    let on_list = &checks[&Uuid::from_u128(1)];
    assert_eq!(
        on_list.marks.len(),
        1,
        "a mark, not just \"not checked\": {on_list:?}"
    );
    assert!(!on_list.unverified);

    let (term, parts) = before_key(&line(
        vec![
            part("amoxicillin", coded(amox, "amoxicillin")),
            part("clavulanic acid", coded(clav, "clavulanic acid")),
        ],
        "Augmentin",
        "started-here",
    ))
    .unwrap();
    let charting_now = check_lines(
        None::<&tokio_postgres::Client>,
        &a,
        &the_line("Augmentin", &term, &parts),
    )
    .await;
    let now = charting_now.get(&Uuid::nil()).cloned().unwrap_or_default();
    assert_eq!(now.against, [Uuid::from_u128(9)]);
}

/// Ruling R15: allergies read over a set that is no longer the one on screen refuse Sign as
/// `ChartSetError::Changed` does everywhere (a re-read), never "not checked, sign anyway". A
/// failed read still proceeds, "not checked" (Global Constraints).
#[test]
fn a_changed_set_refuses_and_a_failed_read_proceeds() {
    use cairn_medication_view::ChartSet;
    let shown = Uuid::from_u128(1);
    let charts = [shown.to_string()];
    let none = || PatientAllergies {
        entries: vec![],
        none: vec![],
        challenges: vec![],
        retractions: vec![],
        doubted_charts: vec![],
        unreadable: 0,
    };
    let linked = ChartSet::new(vec![shown, Uuid::from_u128(2)]).unwrap();
    let err = fresh_allergies(Ok((linked, none(), HashMap::new())), &charts).unwrap_err();
    assert_eq!(err, crate::chart_set::ChartSetError::Changed.into_view());
    let same = fresh_allergies(
        Ok((ChartSet::single(shown), none(), HashMap::new())),
        &charts,
    );
    assert!(matches!(same, Ok(Some(_))));
    let failed = fresh_allergies(Err(refused("the read failed")), &charts);
    assert!(matches!(failed, Ok(None)));
}

/// Ruling R17: a row that carries a coding keeps its own line beside its components, even when
/// every member is a charted combination: a coding added later (db/042) is still checked.
#[test]
fn a_coded_combination_row_keeps_its_own_line() {
    let (amox, clav) = (Uuid::from_u128(0xA40C), Uuid::from_u128(0xC1A7));
    let mut r = row(1, "Augmentin", &[11]);
    r.coding_system = Some(MOIETY_SYSTEM.into());
    r.coding_code = Some(clav.to_string());
    r.coding_display = Some("clavulanic acid".into());
    let charting = HashMap::from([(
        Uuid::from_u128(11),
        combo(&[("amoxicillin", Some(amox)), ("clavulanic acid", None)]),
    )]);
    let rows = [r];
    let lines = charted_lines(&rows, &charting);
    assert_eq!(lines.len(), 3, "the row's own line and both components");
    assert_eq!(lines[0].moiety, Some(clav));
}

/// Ruling R24: Sign signs a combination as its parts' names joined ("amoxicillin + clavulanic
/// acid", R22) and the list re-checks the row under that term, so the check at Sign must read it
/// too: an allergy recorded by name on the joined generic term marks the line being charted, and
/// a Start line then needs a reason — never "not checked" at Sign and marked once drawn.
#[tokio::test]
async fn a_by_name_allergy_on_the_signed_combination_term_marks_the_line() {
    let (amox, clav) = (Uuid::from_u128(0xA40C), Uuid::from_u128(0xC1A7));
    let a = uncoded_allergy("amoxicillin + clavulanic acid");
    let (term, parts) = before_key(&line(
        vec![
            part("amox", coded(amox, "amoxicillin")),
            part("clav", coded(clav, "clavulanic acid")),
        ],
        "amox + clav",
        "started-here",
    ))
    .unwrap();
    assert_eq!(term, "amoxicillin + clavulanic acid");
    let checks = check_lines(
        None::<&tokio_postgres::Client>,
        &a,
        &the_line("amox + clav", &term, &parts),
    )
    .await;
    let now = checks.get(&Uuid::nil()).cloned().unwrap_or_default();
    assert_eq!(now.against, [Uuid::from_u128(9)], "{now:?}");
    assert_eq!(
        override_before_the_key(ChartedAs::StartedHere, &now, None)
            .unwrap_err()
            .text,
        NEEDS_A_REASON
    );
}
