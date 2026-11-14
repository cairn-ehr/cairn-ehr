//! DB-gated walk of the warning threshold (batch-charting slice 3, ADR-0104; on `CAIRN_TEST_PG`,
//! run locally — #905: CI never runs these), on `supervised_note`'s fixture, whose TRUNCATE
//! clears `medication_warning_policy`, so every walk starts at the default.
//!
//! The chart holds a previous adverse effect to codeine (matched by name: no drug list needed).
//! At the default the line being charted carries a muted note and signs with no reason; once
//! the administrator lowers the threshold (db/081's console door, called here as the owner),
//! the same line asks for a reason, draws the note as a warning, and signs an override naming
//! the adverse-effect record. The charted list draws the line as before (maintainer Q2).
use super::db_tests::{asserted, chart_it, line};
use super::*;
use crate::allergies::{allergy_record_impl, EntryInput};
use crate::commands::med_list_impl;
use crate::notes::review::db_tests::{credential, supervised_note};
use cairn_node::allergy::read::list_patient_allergies;
use tokio_postgres::Client;

const NEEDS_A_REASON: &str = "Charting despite a warning needs a reason";

/// A previous adverse effect to codeine on `chart`, under Dr Bea's key; its record id.
async fn codeine_adverse_effect(owner: &Client, state: &AppState, chart: Uuid) -> Uuid {
    let patient = chart.to_string();
    let entry = EntryInput {
        allergen_kind: Some("drug".into()),
        term: "codeine".into(),
        kind: Some("adverse-effect".into()),
        category: Some("rash".into()),
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
        None,
        credential("bea"),
    )
    .await
    .unwrap();
    list_patient_allergies(owner, chart).await.unwrap().entries[0].event_id
}

fn codeine() -> Vec<PartArg> {
    vec![PartArg {
        term: "codeine".into(),
        coding: None,
    }]
}

async fn check_now(state: &AppState, chart: Uuid) -> LineCheckView {
    let patient = chart.to_string();
    med_line_check_impl(
        state,
        &patient,
        std::slice::from_ref(&patient),
        "codeine".into(),
        codeine(),
    )
    .await
}

async fn set_threshold(owner: &Client, word: Option<&str>) {
    owner
        .execute(
            "SELECT cairn_medication_set_warning_threshold($1)",
            &[&word],
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn med_chart_db_a_lowered_threshold_warns_on_an_adverse_effect() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let record = codeine_adverse_effect(&owner, &state, chart).await;

    // The default: a muted note, no reason asked.
    let v = check_now(&state, chart).await;
    assert!(!v.reason_needed && v.warning_notes.is_empty(), "{v:?}");
    assert_eq!(v.notes.len(), 1, "{v:?}");

    // Lowered at the console: the same line now warns, loud, and asks for a reason.
    set_threshold(&owner, Some("adverse-effect")).await;
    let v = check_now(&state, chart).await;
    assert!(v.reason_needed && v.notes.is_empty(), "{v:?}");
    assert_eq!(v.warning_notes.len(), 1, "{v:?}");
    let bare = line(codeine(), "codeine", "started-here", None);
    let refused = chart_it(&state, chart, bare).await.unwrap_err();
    assert_eq!(refused.text, NEEDS_A_REASON);

    // With a reason it signs, the override naming the adverse-effect record.
    let with_reason = line(
        codeine(),
        "codeine",
        "started-here",
        Some("tolerated in hospital"),
    );
    let done = chart_it(&state, chart, with_reason).await.unwrap();
    let p = &asserted(&owner, chart).await[0];
    assert_eq!(p["warning_override"]["reason"], "tolerated in hospital");
    assert_eq!(
        p["warning_override"]["against"],
        serde_json::json!([{"source": "allergy", "ref": record.to_string()}])
    );

    // Q2: the charted line is drawn as ADR-0098 draws it — the note muted, no Allergy mark.
    let pane = done.pane.expect("the list read again");
    let row = pane
        .list
        .rows
        .iter()
        .find(|r| r.primary == "codeine")
        .unwrap();
    assert!(row.allergy_marks.is_empty(), "{row:?}");
    assert_eq!(row.allergy_notes.len(), 1, "{row:?}");
    let reread = med_list_impl(&state, &chart.to_string(), None)
        .await
        .unwrap();
    let row = reread
        .list
        .rows
        .iter()
        .find(|r| r.primary == "codeine")
        .unwrap();
    assert!(row.allergy_marks.is_empty() && row.allergy_notes.len() == 1);

    set_threshold(&owner, None).await;
}
