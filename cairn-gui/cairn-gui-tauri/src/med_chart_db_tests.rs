//! DB-gated walks for `med_chart.rs`'s check and pick (on `CAIRN_TEST_PG`, run locally — #905: CI
//! never runs these), on `supervised_note`'s fixture: the window's own login, a John Doe chart,
//! and Dr Bea in the vault. There is no drug list on the test database, so every allergy mark here
//! is by code or by name (no drugref needed). What the list's Sign signs is walked in
//! `med_list_signed_db_tests.rs` (moved there when the lone-line Sign was deleted, #1139).
use super::*;
use crate::allergies::{allergy_record_impl, EntryInput};
use crate::notes::review::db_tests::{credential, supervised_note};
use cairn_node::allergy::read::list_patient_allergies;
use tokio_postgres::Client;

/// Sign a drug allergy to `term` on `chart` under Dr Bea's key, coded when `coding` is given;
/// its record id.
async fn drug_allergy(
    owner: &Client,
    state: &AppState,
    chart: Uuid,
    term: &str,
    coding: Option<CodingArg>,
) -> Uuid {
    let patient = chart.to_string();
    let entry = EntryInput {
        allergen_kind: Some("drug".into()),
        term: term.into(),
        kind: Some("allergy".into()),
        category: Some("anaphylaxis".into()),
        reaction_text: None,
        onset: None,
        source: Some("patient-stated".into()),
        corrects: None,
    };
    allergy_record_impl(
        state,
        &patient,
        vec![patient.clone()],
        entry,
        coding,
        credential("bea"),
    )
    .await
    .unwrap();
    list_patient_allergies(owner, chart).await.unwrap().entries[0].event_id
}

/// An amoxicillin allergy coded to `moiety`; its record id.
async fn amoxicillin_allergy(owner: &Client, state: &AppState, chart: Uuid, moiety: Uuid) -> Uuid {
    let coding = Some(CodingArg {
        matched: None,
        system: MOIETY_SYSTEM.into(),
        code: moiety.to_string(),
        display: "amoxicillin".into(),
    });
    drug_allergy(owner, state, chart, "amoxicillin", coding).await
}

fn amox_part(moiety: Uuid) -> PartArg {
    PartArg {
        term: "amox".into(),
        coding: Some(CodingArg {
            matched: None,
            system: MOIETY_SYSTEM.into(),
            code: moiety.to_string(),
            display: "amoxicillin".into(),
        }),
    }
}

/// The check reads the allergies live: the line was checked clean, then an allergy was recorded,
/// and the next check marks it. (That the list's Sign re-reads too is its R1 walks'.)
#[tokio::test]
async fn med_chart_db_the_check_reads_the_allergies_live() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let amox = Uuid::from_u128(0xA40C_5003);
    let patient = chart.to_string();
    let first = med_line_check_impl(
        &state,
        &patient,
        std::slice::from_ref(&patient),
        "amox".into(),
        vec![amox_part(amox)],
    )
    .await;
    assert!(first.marks.is_empty() && !first.reason_needed, "{first:?}");
    amoxicillin_allergy(&owner, &state, chart, amox).await;
    let now = med_line_check_impl(
        &state,
        &patient,
        std::slice::from_ref(&patient),
        "amox".into(),
        vec![amox_part(amox)],
    )
    .await;
    assert!(now.reason_needed, "{now:?}");
}

/// Review Focus 5: no drug list. The search says so, never an error (charting the line uncoded is
/// walked in `med_list_signed_db_tests.rs`).
#[tokio::test]
async fn med_chart_db_no_drug_list_is_said_not_failed() {
    let Some((_guard, _owner, state, _chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let found = med_drug_search_impl(&state, "metformin").await;
    assert!(!found.available && found.options.is_empty());
}

/// Ruling R16: the check on screen reads the line term as Sign does (a combination's: the drug
/// as typed, R22), so an allergy recorded by name on the brand marks a combination typed by it
/// while typing too.
#[tokio::test]
async fn med_chart_db_a_by_name_allergy_on_the_typed_drug_marks_the_check() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    drug_allergy(&owner, &state, chart, "Augmentin", None).await;
    let pick = |term: &str, code: u128| PartArg {
        term: term.into(),
        coding: Some(CodingArg {
            matched: None,
            system: MOIETY_SYSTEM.into(),
            code: Uuid::from_u128(code).to_string(),
            display: term.into(),
        }),
    };
    let parts = vec![
        pick("amoxicillin", 0xA40C_5007),
        pick("clavulanic acid", 0xC1A7_5007),
    ];
    let patient = chart.to_string();
    let check = med_line_check_impl(
        &state,
        &patient,
        std::slice::from_ref(&patient),
        "Augmentin".into(),
        parts,
    )
    .await;
    assert!(check.reason_needed, "marked by the typed name: {check:?}");
}
