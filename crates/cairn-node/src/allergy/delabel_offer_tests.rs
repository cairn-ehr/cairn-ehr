//! The same-drug rule: code equality only, live allergies only.
use super::*;
use crate::allergy::check::MOIETY_SYSTEM;
use crate::allergy::read::{AllergyRow, Coding};
use crate::allergy::test_fixtures::{challenge, coded, entry, id, pa};
use crate::allergy::thread::resolve;

const AMOX: u128 = 500;
const PEN_GROUP: u128 = 600;

fn amox(n: u128) -> AllergyRow {
    coded(
        entry(n, "drug", "amoxicillin", "allergy", "rash", 1),
        MOIETY_SYSTEM,
        AMOX,
    )
}

fn offered(rows: Vec<AllergyRow>, doubted: &[Uuid]) -> Vec<Uuid> {
    let a = pa(rows, vec![], vec![], vec![]);
    same_drug_heads(&resolve(&a), doubted, id(AMOX))
        .iter()
        .map(|r| r.event_id)
        .collect()
}

#[test]
fn an_allergy_coded_to_the_same_moiety_is_offered() {
    assert_eq!(offered(vec![amox(1)], &[]), vec![id(1)]);
}

#[test]
fn a_group_coded_allergy_is_never_offered() {
    let g = coded(
        entry(1, "drug", "penicillin", "allergy", "rash", 1),
        "drugref-group",
        PEN_GROUP,
    );
    assert!(offered(vec![g], &[]).is_empty());
    // Even a group row whose code equals the moiety's: the system must be the moiety system.
    let g2 = coded(
        entry(2, "drug", "penicillin", "allergy", "rash", 1),
        "drugref-group",
        AMOX,
    );
    assert!(offered(vec![g2], &[]).is_empty());
}

#[test]
fn an_uncoded_allergy_of_the_same_name_is_never_offered() {
    let e = entry(1, "drug", "amoxicillin", "allergy", "rash", 1);
    assert!(offered(vec![e], &[]).is_empty());
}

#[test]
fn an_adverse_effect_is_never_offered() {
    let e = coded(
        entry(1, "drug", "amoxicillin", "adverse-effect", "rash", 1),
        MOIETY_SYSTEM,
        AMOX,
    );
    assert!(offered(vec![e], &[]).is_empty());
}

#[test]
fn an_unknown_kind_is_never_offered() {
    let e = coded(
        entry(1, "drug", "amoxicillin", "intolerance-v2", "rash", 1),
        MOIETY_SYSTEM,
        AMOX,
    );
    assert!(offered(vec![e], &[]).is_empty());
}

#[test]
fn a_non_drug_allergen_is_never_offered() {
    let e = coded(
        entry(1, "other", "amoxicillin", "allergy", "rash", 1),
        MOIETY_SYSTEM,
        AMOX,
    );
    assert!(offered(vec![e], &[]).is_empty());
}

#[test]
fn a_delabelled_or_withdrawn_head_is_never_offered() {
    // Control: the entry alone is offered.
    assert_eq!(offered(vec![amox(1)], &[]), vec![id(1)]);
    // De-labelled: the head moves to `historical`.
    let a = pa(vec![amox(1)], vec![], vec![challenge(10, 1, 2)], vec![]);
    let r = resolve(&a);
    assert!(r.live.is_empty(), "fixture must really de-label the head");
    assert!(same_drug_heads(&r, &[], id(AMOX)).is_empty());
    // Withdrawn: the head moves to `withdrawn`.
    let a = pa(
        vec![amox(1)],
        vec![],
        vec![],
        vec![crate::allergy::test_fixtures::retraction(11, 1, 2)],
    );
    let r = resolve(&a);
    assert!(r.live.is_empty(), "fixture must really withdraw the head");
    assert!(same_drug_heads(&r, &[], id(AMOX)).is_empty());
}

#[test]
fn a_head_on_a_doubted_chart_is_never_offered() {
    let doubted = [id(crate::allergy::test_fixtures::CHART)];
    assert!(offered(vec![amox(1)], &doubted).is_empty());
    assert_eq!(offered(vec![amox(1)], &[id(7)]), vec![id(1)]);
}

#[test]
fn two_entries_on_one_moiety_are_both_offered() {
    let mut got = offered(vec![amox(1), amox(2)], &[]);
    got.sort();
    assert_eq!(got, vec![id(1), id(2)]);
}

#[test]
fn a_code_spelled_in_capitals_still_matches() {
    let mut e = amox(1);
    e.coding = Some(Coding {
        system: MOIETY_SYSTEM.into(),
        code: id(AMOX).to_string().to_uppercase(),
        display: "amoxicillin".into(),
    });
    assert_eq!(offered(vec![e], &[]), vec![id(1)]);
    let mut bad = amox(2);
    bad.coding.as_mut().unwrap().code = "not-a-uuid".into();
    assert!(offered(vec![bad], &[]).is_empty());
}

#[test]
fn offer_words_name_the_record_and_its_signer() {
    assert_eq!(
        offer_words("Amoxicillin", "amoxicillin", Some("Dr A")),
        "De-label the Amoxicillin allergy: tolerates amoxicillin (recorded by Dr A)"
    );
    assert_eq!(
        offer_words("Amoxicillin", "amoxicillin", None),
        "De-label the Amoxicillin allergy: tolerates amoxicillin"
    );
}
