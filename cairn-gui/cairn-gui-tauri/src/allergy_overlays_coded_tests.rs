//! DB-gated walks for the de-label's optional tolerated-drug coding (#1053; on `CAIRN_TEST_PG`,
//! run locally — #905): it lands signed with the coding, an uncoded one still writes `null`, and
//! a de-label of a line already struck is refused before anything is written.
use super::*;
use crate::allergies::{allergy_record_impl, EntryInput};
use crate::allergy_check::CodingArg;
use crate::notes::review::db_tests::{credential, supervised_note};

const AMOX: u128 = 0xA40C_7002;

/// A complete penicillin entry, as the form sends it.
fn penicillin() -> EntryInput {
    EntryInput {
        allergen_kind: Some("drug".into()),
        term: "penicillin".into(),
        kind: Some("allergy".into()),
        category: Some("anaphylaxis".into()),
        reaction_text: None,
        onset: Some("2014".into()),
        source: Some("seen-documented".into()),
        corrects: None,
    }
}

/// The picked tolerated drug, its code spelled in capitals the way a webview may send it.
fn amoxicillin() -> CodingArg {
    CodingArg {
        system: MOIETY_SYSTEM.into(),
        code: Uuid::from_u128(AMOX).to_string().to_uppercase(),
        display: " amoxicillin ".into(),
    }
}

/// The `coding` of every challenge on the chart, as stored (JSON text, or None for SQL/JSON null).
async fn stored_codings(owner: &tokio_postgres::Client, chart: Uuid) -> Vec<Option<String>> {
    owner
        .query(
            "SELECT tolerated_coding::text FROM clinical_allergy_challenge \
             WHERE patient_id = $1::text::uuid",
            &[&chart.to_string()],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect()
}

#[tokio::test]
async fn a_coded_delabel_lands_with_its_coding() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let here = chart.to_string();
    let charts = vec![here.clone()];
    let pane = allergy_record_impl(
        &state,
        &here,
        charts.clone(),
        penicillin(),
        None,
        credential("bea"),
    )
    .await
    .unwrap()
    .pane
    .unwrap();
    let target = pane.drug_allergies[0].event_id.clone();
    let coding = amoxicillin();
    allergy_challenge_impl(
        &state,
        &here,
        charts,
        &target,
        "amoxicillin",
        Some(&coding),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let stored = stored_codings(&owner, chart).await;
    assert_eq!(stored.len(), 1, "{stored:?}");
    let v: serde_json::Value = serde_json::from_str(stored[0].as_ref().unwrap()).unwrap();
    assert_eq!(v["system"], MOIETY_SYSTEM);
    assert_eq!(v["code"], Uuid::from_u128(AMOX).to_string());
    assert_eq!(v["display"], "amoxicillin");
}

#[tokio::test]
async fn an_uncoded_delabel_is_unchanged() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let here = chart.to_string();
    let charts = vec![here.clone()];
    let pane = allergy_record_impl(
        &state,
        &here,
        charts.clone(),
        penicillin(),
        None,
        credential("bea"),
    )
    .await
    .unwrap()
    .pane
    .unwrap();
    let target = pane.drug_allergies[0].event_id.clone();
    allergy_challenge_impl(
        &state,
        &here,
        charts,
        &target,
        "amoxicillin",
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(stored_codings(&owner, chart).await, vec![None]);
}

/// A second de-label of a line already struck is refused with the existing words BEFORE the key:
/// this test's own first act signs a de-label and forgets the held key, so a refusal reached only
/// after the key would be the key's sentence, not GONE_LINE.
#[tokio::test]
async fn a_delabel_of_a_head_struck_since_is_refused_before_the_key() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let here = chart.to_string();
    let charts = vec![here.clone()];
    let pane = allergy_record_impl(
        &state,
        &here,
        charts.clone(),
        penicillin(),
        None,
        credential("bea"),
    )
    .await
    .unwrap()
    .pane
    .unwrap();
    let target = pane.drug_allergies[0].event_id.clone();
    let coding = amoxicillin();
    allergy_challenge_impl(
        &state,
        &here,
        charts.clone(),
        &target,
        "amoxicillin",
        Some(&coding),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    state.forget_held_key().await;
    let err = allergy_challenge_impl(
        &state,
        &here,
        charts,
        &target,
        "amoxicillin",
        Some(&coding),
        None,
        None,
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(err.text, GONE_LINE);
    assert_eq!(err.retry, Retry::AfterReread(Recorded::No));
    assert_eq!(
        stored_codings(&owner, chart).await.len(),
        1,
        "one event written"
    );
}

/// A group pick is refused with its own words ahead of the target lookup and the key (no key is
/// held and none is typed), and nothing is written.
#[tokio::test]
async fn a_group_coded_delabel_is_refused_before_the_key() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let here = chart.to_string();
    let charts = vec![here.clone()];
    let pane = allergy_record_impl(
        &state,
        &here,
        charts.clone(),
        penicillin(),
        None,
        credential("bea"),
    )
    .await
    .unwrap()
    .pane
    .unwrap();
    let target = pane.drug_allergies[0].event_id.clone();
    state.forget_held_key().await;
    let group = CodingArg {
        system: cairn_node::allergy::check::GROUP_SYSTEM.into(),
        ..amoxicillin()
    };
    let err = allergy_challenge_impl(
        &state,
        &here,
        charts,
        &target,
        "amoxicillin",
        Some(&group),
        None,
        None,
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(err.text, crate::allergy_check::NOT_FROM_THE_LIST);
    assert!(
        stored_codings(&owner, chart).await.is_empty(),
        "nothing written"
    );
}

const GONE_LINE: &str = "That allergy line is no longer on this chart — read it again.";
