//! DB-gated walks for the list's de-label offers (on `CAIRN_TEST_PG`, run locally — #905), on
//! `med_list_db_tests.rs`'s fixture and helpers.
use super::db_tests::{amoxicillin_allergy, arg, bea_list, pick, read, save_as};
use super::*;
use crate::allergies::{allergy_record_impl, EntryInput};
use crate::allergy_overlays::allergy_challenge_impl;
use crate::notes::review::db_tests::credential;
use cairn_node::allergy::check::GROUP_SYSTEM;

const AMOX: u128 = 0xA40C_7001;

fn the_line<'a>(v: &'a MedListView, id: &str) -> &'a ListLineView {
    v.lines.iter().find(|l| l.line_id == id).unwrap()
}

#[tokio::test]
async fn a_list_line_on_an_allergy_coded_to_its_drug_carries_the_offer() {
    let Some((_g, _o, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    amoxicillin_allergy(&state, chart, AMOX).await;
    let l = arg(
        ("amoxicillin", "500mg tds"),
        pick("amoxicillin", AMOX),
        false,
    );
    let id = l.line_id.clone();
    save_as(&state, chart, l, Some("bea")).await;
    let v = read(&state, chart).await;
    let offers = &the_line(&v, &id).delabel_offers;
    assert_eq!(offers.len(), 1, "{offers:?}");
    assert_eq!(offers[0].tolerated, "amoxicillin");
    assert_eq!(offers[0].coding.code, Uuid::from_u128(AMOX).to_string());
    assert!(offers[0]
        .words
        .starts_with("De-label the amoxicillin allergy"));
}

#[tokio::test]
async fn a_group_allergy_line_carries_no_offer() {
    let Some((_g, _o, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let patient = chart.to_string();
    let entry = EntryInput {
        allergen_kind: Some("drug".into()),
        term: "penicillins".into(),
        kind: Some("allergy".into()),
        category: Some("anaphylaxis".into()),
        reaction_text: None,
        onset: None,
        source: Some("patient-stated".into()),
        corrects: None,
    };
    let mut coding = pick("penicillins", AMOX).remove(0).coding;
    coding.as_mut().unwrap().system = GROUP_SYSTEM.into();
    allergy_record_impl(
        &state,
        &patient,
        vec![patient.clone()],
        entry,
        coding,
        credential("bea"),
    )
    .await
    .unwrap();
    let l = arg(
        ("amoxicillin", "500mg tds"),
        pick("amoxicillin", AMOX),
        false,
    );
    let id = l.line_id.clone();
    save_as(&state, chart, l, Some("bea")).await;
    let v = read(&state, chart).await;
    assert!(the_line(&v, &id).delabel_offers.is_empty());
}

#[tokio::test]
async fn after_the_delabel_signs_the_offer_and_the_mark_are_gone() {
    let Some((_g, _o, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    amoxicillin_allergy(&state, chart, AMOX).await;
    let mut l = arg(
        ("amoxicillin", "500mg tds"),
        pick("amoxicillin", AMOX),
        false,
    );
    let id = l.line_id.clone();
    save_as(&state, chart, l.clone(), Some("bea")).await;
    // The clinician saw the warning, then hand-ticked: the tick is given against it.
    l.hand_ticked = true;
    let v = read(&state, chart).await;
    let drawn = the_line(&v, &id);
    assert!(!drawn.check.marks.is_empty(), "{drawn:?}");
    let target = drawn.delabel_offers[0].target.clone();
    l.seen = drawn
        .against
        .iter()
        .map(|w| WarningRef {
            source: w.source.clone(),
            reference: w.reference.clone(),
        })
        .collect();
    save_as(&state, chart, l, Some("bea")).await;
    let patient = chart.to_string();
    allergy_challenge_impl(
        &state,
        &patient,
        vec![patient.clone()],
        &target,
        "amoxicillin",
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let v = read(&state, chart).await;
    let after = the_line(&v, &id);
    assert!(
        after.delabel_offers.is_empty(),
        "{:?}",
        after.delabel_offers
    );
    assert!(after.check.marks.is_empty(), "{:?}", after.check.marks);
    assert!(after.ticked, "a hand-ticked line stays ticked: {after:?}");
}

#[tokio::test]
async fn a_changed_chart_set_carries_no_offer() {
    let Some((_g, _o, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    amoxicillin_allergy(&state, chart, AMOX).await;
    let l = arg(
        ("amoxicillin", "500mg tds"),
        pick("amoxicillin", AMOX),
        false,
    );
    let id = l.line_id.clone();
    save_as(&state, chart, l, Some("bea")).await;
    // A set on screen that is not the set read now: the allergies are not used.
    let patient = chart.to_string();
    let other = vec![Uuid::now_v7().to_string()];
    let v = med_list_read_impl(&state, &patient, Some(&other))
        .await
        .unwrap();
    assert!(the_line(&v, &id).delabel_offers.is_empty());
}
