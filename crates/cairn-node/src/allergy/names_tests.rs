//! Charting slice 3 (ADR-0106, ruling R3): an allergy entry coded to a DRUG reads generic-first,
//! the clinician's own word in brackets — "lidocaine (lignocaine)" — on every allergy line (the
//! strip, the line, a de-labelled line). A group-coded or uncoded entry keeps its words as typed.
use super::allergen_name;
use crate::allergy::read::PatientAllergies;
use crate::allergy::test_fixtures::{bne, challenge, coded, entry};
use crate::allergy::view::{build_view, entry_line_text};

/// "lignocaine" coded under `system` to a drug list entry displayed "lidocaine".
fn lignocaine(n: u128, system: &str) -> crate::allergy::read::AllergyRow {
    let mut row = coded(
        entry(n, "drug", "lignocaine", "allergy", "rash", 8),
        system,
        77,
    );
    row.coding.as_mut().unwrap().display = "lidocaine".into();
    row
}

fn pa(entries: Vec<crate::allergy::read::AllergyRow>) -> PatientAllergies {
    PatientAllergies {
        entries,
        none: vec![],
        challenges: vec![],
        retractions: vec![],
        doubted_charts: vec![],
        unreadable: 0,
    }
}

#[test]
fn a_moiety_coded_line_reads_generic_first() {
    let row = lignocaine(1, "drugref-moiety");
    assert_eq!(allergen_name(&row), "lidocaine (lignocaine)");
    assert!(
        entry_line_text(&row).starts_with("Allergy: lidocaine (lignocaine)"),
        "{}",
        entry_line_text(&row)
    );
    let v = build_view(&pa(vec![row]), bne());
    assert!(v.strip.contains("lidocaine (lignocaine)"), "{}", v.strip);
}

/// Review Focus 5: no bracket for a group code, for no code, or for the same word in another case.
#[test]
fn only_a_drug_code_with_another_name_is_bracketed() {
    assert_eq!(allergen_name(&lignocaine(1, "drugref-class")), "lignocaine");
    let uncoded = entry(2, "drug", "lignocaine", "allergy", "rash", 8);
    assert_eq!(allergen_name(&uncoded), "lignocaine");
    let mut same = lignocaine(3, "drugref-moiety");
    same.term = "Lidocaine".into();
    assert_eq!(allergen_name(&same), "lidocaine");
}

#[test]
fn a_delabelled_line_reads_generic_first() {
    let mut a = pa(vec![lignocaine(1, "drugref-moiety")]);
    a.challenges = vec![challenge(2, 1, 9)];
    let v = build_view(&a, bne());
    let line = v
        .historical
        .iter()
        .map(|h| h.text.clone())
        .find(|t| t.starts_with("Historically reported:"))
        .unwrap_or_else(|| panic!("no historical line in {:?}", v.historical));
    assert!(
        line.starts_with("Historically reported: lidocaine (lignocaine) allergy"),
        "{line}"
    );
}
