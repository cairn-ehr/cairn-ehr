//! The offer builder, pure: which offers a line's parts draw from the live allergy heads.
use super::*;
use cairn_node::allergy::read::{AllergyRow, Coding, KeyState, PatientAllergies};
use cairn_node::allergy::thread::resolve;
use chrono::{TimeZone, Utc};

/// A live drug allergy `n` coded to moiety `code`, signed by Dr Ada.
fn allergy(n: u128, term: &str, code: u128) -> AllergyRow {
    let at = Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap();
    AllergyRow {
        event_id: Uuid::from_u128(n),
        chart: Uuid::from_u128(1000),
        allergen_kind: "drug".into(),
        term: term.into(),
        kind: "allergy".into(),
        category: "rash".into(),
        reaction_text: None,
        onset_precision: "not-known".into(),
        onset_value: None,
        source: "seen-documented".into(),
        recorded_at: at,
        hlc: (at.timestamp_millis(), 0),
        signer: Some("Dr Ada".into()),
        corrects: None,
        key: KeyState::Standing,
        coding: Some(Coding {
            system: MOIETY_SYSTEM.into(),
            code: Uuid::from_u128(code).to_string(),
            display: term.into(),
        }),
    }
}

fn part(term: &str, code: Option<u128>) -> PartArg {
    PartArg {
        term: term.into(),
        coding: code.map(|c| CodingArg {
            system: MOIETY_SYSTEM.into(),
            code: Uuid::from_u128(c).to_string(),
            display: term.into(),
        }),
    }
}

fn offers(rows: Vec<AllergyRow>, parts: &[PartArg]) -> Vec<DelabelOffer> {
    let a = PatientAllergies {
        entries: rows,
        none: vec![],
        doubted_charts: vec![],
        challenges: vec![],
        retractions: vec![],
        unreadable: 0,
    };
    offers_for(&resolve(&a), &[], parts)
}

#[test]
fn a_combination_offers_only_its_matching_part() {
    let parts = [part("clavulanate", Some(2)), part("amoxicillin", Some(1))];
    let got = offers(vec![allergy(10, "Amoxicillin", 1)], &parts);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].target, Uuid::from_u128(10).to_string());
    assert_eq!(got[0].tolerated, "amoxicillin");
    assert_eq!(got[0].coding.code, Uuid::from_u128(1).to_string());
    assert_eq!(
        got[0].words,
        "De-label the Amoxicillin allergy: tolerates amoxicillin (recorded by Dr Ada)"
    );
}

#[test]
fn an_uncoded_part_offers_nothing() {
    assert!(offers(
        vec![allergy(10, "Amoxicillin", 1)],
        &[part("amoxicillin", None)]
    )
    .is_empty());
}

#[test]
fn one_head_met_by_two_parts_is_offered_once() {
    let parts = [part("amoxicillin", Some(1)), part("amox", Some(1))];
    let got = offers(vec![allergy(10, "Amoxicillin", 1)], &parts);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].tolerated, "amoxicillin", "first occurrence wins");
}
