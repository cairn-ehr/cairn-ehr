//! DB-gated walks for `med_chart.rs` (on `CAIRN_TEST_PG`, run locally — #905: CI never runs
//! these), on `supervised_note`'s fixture: the window's own login, a John Doe chart, and Dr Bea
//! in the vault. There is no drug list on the test database, so every allergy mark here is by
//! code or by name (no drugref needed), and an uncoded line reads "not checked".
use super::*;
use crate::allergies::{allergy_record_impl, EntryInput};
use crate::notes::review::db_tests::{credential, supervised_note};
use cairn_node::allergy::read::list_patient_allergies;
use serde_json::Value;
use tokio_postgres::Client;

/// The refusal a Start line with an allergy mark and no reason gets (Global Constraints).
const NEEDS_A_REASON: &str = "Charting despite a listed allergy needs a reason";

/// Sign an amoxicillin allergy, coded to `moiety`, on `chart` under Dr Bea's key; its record id.
async fn amoxicillin_allergy(owner: &Client, state: &AppState, chart: Uuid, moiety: Uuid) -> Uuid {
    let patient = chart.to_string();
    let entry = EntryInput {
        allergen_kind: Some("drug".into()),
        term: "amoxicillin".into(),
        kind: Some("allergy".into()),
        category: Some("anaphylaxis".into()),
        reaction_text: None,
        onset: None,
        source: Some("patient-stated".into()),
        corrects: None,
    };
    let coding = Some(CodingArg {
        system: MOIETY_SYSTEM.into(),
        code: moiety.to_string(),
        display: "amoxicillin".into(),
    });
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

fn amox_part(moiety: Uuid) -> PartArg {
    PartArg {
        term: "amox".into(),
        coding: Some(CodingArg {
            system: MOIETY_SYSTEM.into(),
            code: moiety.to_string(),
            display: "amoxicillin".into(),
        }),
    }
}

fn line(parts: Vec<PartArg>, typed: &str, charted_as: &str, reason: Option<&str>) -> ChartArg {
    ChartArg {
        parts,
        typed_drug: typed.into(),
        directions: "500mg tds".into(),
        charted_as: charted_as.into(),
        reason: reason.map(str::to_owned),
    }
}

async fn chart_it(state: &AppState, chart: Uuid, l: ChartArg) -> Result<MedChartWrite, ErrorView> {
    let patient = chart.to_string();
    med_chart_impl(state, &patient, vec![patient.clone()], l, credential("bea")).await
}

/// The clear payloads of every medication assertion on `chart`.
async fn asserted(owner: &Client, chart: Uuid) -> Vec<Value> {
    owner
        .query(
            "SELECT cairn_clear_payload(e)::text FROM event_log e \
              WHERE e.event_type = 'clinical.medication.asserted' \
                AND e.patient_id = $1::text::uuid",
            &[&chart.to_string()],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| serde_json::from_str(r.get::<_, &str>(0)).unwrap())
        .collect()
}

async fn vault_attempts(owner: &Client) -> i64 {
    owner
        .query_one("SELECT count(*) FROM vault_attempt", &[])
        .await
        .unwrap()
        .get(0)
}

/// Review Focus 4: a whitespace-only reason on a Start line with a mark is refused with the exact
/// copy BEFORE the key — Bea's (correct) passphrase is sent, yet the vault is never asked and
/// nothing is signed.
#[tokio::test]
async fn med_chart_db_a_start_line_with_an_allergy_mark_needs_a_reason() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let amox = Uuid::from_u128(0xA40C_5001);
    amoxicillin_allergy(&owner, &state, chart, amox).await;
    let before = vault_attempts(&owner).await;
    let l = line(vec![amox_part(amox)], "amox", "started-here", Some("   "));
    let err = chart_it(&state, chart, l).await.unwrap_err();
    assert_eq!(err.text, NEEDS_A_REASON);
    assert_eq!(
        vault_attempts(&owner).await,
        before,
        "the vault was not asked"
    );
    assert!(
        asserted(&owner, chart).await.is_empty(),
        "nothing was signed"
    );
}

#[tokio::test]
async fn med_chart_db_an_already_taking_line_never_needs_a_reason() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let amox = Uuid::from_u128(0xA40C_5002);
    amoxicillin_allergy(&owner, &state, chart, amox).await;
    let l = line(vec![amox_part(amox)], "amox", "pre-existing", None);
    let done = chart_it(&state, chart, l).await.unwrap();
    assert_eq!(done.outcome, "Charted.");
    let p = &asserted(&owner, chart).await[0];
    assert_eq!(p["charted_as"], "pre-existing");
    assert_eq!(p["info_source"], "patient-reported");
    assert!(p.get("allergy_override").is_none() && p.get("started").is_none());
    assert_eq!(p["substance"]["term"], "amoxicillin");
    assert_eq!(p["substance"]["coding"]["code"], amox.to_string().as_str());
    assert_eq!(p["sig"], "500mg tds");
    // The pane is the medication list read again, the line marked.
    let pane = done.pane.expect("the list read again");
    let row = pane
        .list
        .rows
        .iter()
        .find(|r| r.primary == "amoxicillin")
        .unwrap();
    assert_eq!(row.allergy_marks.len(), 1);
}

/// The window's earlier answer is never trusted: the line was checked clean, then an allergy
/// was recorded, and Sign re-reads it and asks for the reason.
#[tokio::test]
async fn med_chart_db_the_check_is_reread_at_sign() {
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
        vec![amox_part(amox)],
    )
    .await;
    assert!(first.marks.is_empty() && !first.reason_needed, "{first:?}");
    amoxicillin_allergy(&owner, &state, chart, amox).await;
    let now = med_line_check_impl(
        &state,
        &patient,
        std::slice::from_ref(&patient),
        vec![amox_part(amox)],
    )
    .await;
    assert!(now.reason_needed, "{now:?}");
    let l = line(vec![amox_part(amox)], "amox", "started-here", None);
    assert_eq!(
        chart_it(&state, chart, l).await.unwrap_err().text,
        NEEDS_A_REASON
    );
}

/// Review Focus 5: no drug list. The search says so (never an error), the line charts uncoded,
/// and against a coded allergy it reads "not checked", never clean.
#[tokio::test]
async fn med_chart_db_no_drug_list_charts_uncoded() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let found = med_drug_search_impl(&state, "metformin").await;
    assert!(!found.available && found.options.is_empty());
    amoxicillin_allergy(&owner, &state, chart, Uuid::from_u128(0xA40C_5004)).await;
    let metformin = PartArg {
        term: "metformin".into(),
        coding: None,
    };
    let done = chart_it(
        &state,
        chart,
        line(vec![metformin], "metformin", "started-here", None),
    )
    .await
    .unwrap();
    let p = &asserted(&owner, chart).await[0];
    assert!(p["substance"].get("coding").is_none());
    assert_eq!(p["started"]["precision"], "day");
    let pane = done.pane.unwrap();
    let row = pane
        .list
        .rows
        .iter()
        .find(|r| r.primary == "metformin")
        .unwrap();
    assert!(
        row.allergy_marks.is_empty() && row.allergy_unverified,
        "not checked"
    );
}

#[tokio::test]
async fn med_chart_db_the_signed_event_carries_the_override() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let amox = Uuid::from_u128(0xA40C_5005);
    let head = amoxicillin_allergy(&owner, &state, chart, amox).await;
    let l = line(
        vec![amox_part(amox)],
        "amox",
        "started-here",
        Some(" no alternative "),
    );
    chart_it(&state, chart, l).await.unwrap();
    let p = &asserted(&owner, chart).await[0];
    assert_eq!(p["charted_as"], "started-here");
    assert_eq!(p["allergy_override"]["reason"], "no alternative");
    assert_eq!(
        p["allergy_override"]["against"],
        serde_json::json!([head.to_string()])
    );
}

/// A combination signs its parts as components, never a single coding, and the list read
/// again checks each part: the amoxicillin in it meets the amoxicillin allergy.
#[tokio::test]
async fn med_chart_db_a_combination_signs_components() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let amox = Uuid::from_u128(0xA40C_5006);
    amoxicillin_allergy(&owner, &state, chart, amox).await;
    let clav = PartArg {
        term: "clav".into(),
        coding: None,
    };
    let l = line(
        vec![amox_part(amox), clav],
        "amox + clav",
        "started-here",
        Some("reason"),
    );
    let done = chart_it(&state, chart, l).await.unwrap();
    let p = &asserted(&owner, chart).await[0];
    let s = &p["substance"];
    assert_eq!(s["term"], "amox + clav");
    assert!(s.get("coding").is_none());
    assert_eq!(s["components"][0]["term"], "amoxicillin");
    assert_eq!(
        s["components"][0]["coding"]["code"],
        amox.to_string().as_str()
    );
    assert_eq!(
        s["components"][1],
        serde_json::json!({"term": "clav", "coding": null})
    );
    let pane = done.pane.unwrap();
    let row = pane
        .list
        .rows
        .iter()
        .find(|r| r.primary == "amox + clav")
        .unwrap();
    assert_eq!(row.allergy_marks.len(), 1, "{:?}", row.allergy_marks);
}
