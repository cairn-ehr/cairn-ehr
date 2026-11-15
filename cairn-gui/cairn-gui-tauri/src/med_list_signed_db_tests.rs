//! DB-gated walks of what the list's Sign SIGNS (on `CAIRN_TEST_PG`, run locally — #905: CI never
//! runs these), on `med_list_db_tests.rs`'s fixture (Dr Bea named, a John Doe chart, no drug list
//! installed, so a mark is by code or by name and an uncoded line reads "not checked").
//!
//! Moved here from slice 1a's lone-line `med_chart` Sign when that command was deleted (#1139):
//! these are the walks the list's own tests lacked — the signed event's fields, its override and
//! its combination components, the warning threshold at Sign, and a Sign that could chart nothing
//! never releasing the key. Each line is ticked as the webview ticks it: by hand, having seen
//! what the pane draws on it (ruling R1: a tick vouches only for the warnings drawn).
use super::db_tests::{amoxicillin_allergy, arg, assertions, bea_list, done, pick, read, releases};
use super::db_tests::{save_as, sign};
use super::review_db_tests::drawn;
use super::stat_db_tests::payloads;
use super::*;
use crate::allergies::{allergy_record_impl, EntryInput};
use crate::commands::med_list_impl;
use crate::med_chart::{med_line_check_impl, LineCheckView, NEEDS_A_REASON};
use crate::med_list_sign::MedListSigned;
use crate::notes::review::db_tests::credential;
use cairn_node::allergy::read::list_patient_allergies;
use tokio_postgres::Client;

/// Save `line` as Bea's, then tick it by hand having seen what the pane draws on it, with
/// `reason` (as the webview saves a tick); its id.
async fn tick_seen(
    state: &AppState,
    chart: Uuid,
    mut line: LineArg,
    reason: Option<&str>,
) -> String {
    let id = line.line_id.clone();
    assert_eq!(
        save_as(state, chart, line.clone(), Some("bea")).await,
        "saved"
    );
    line.seen = drawn(&read(state, chart).await, &id);
    line.hand_ticked = true;
    line.reason = reason.map(str::to_owned);
    assert_eq!(save_as(state, chart, line, Some("bea")).await, "saved");
    id
}

/// An Already-taking line (`arg`'s, pressed "Already taking").
fn already_taking(typed: (&str, &str), parts: Vec<PartArg>) -> LineArg {
    LineArg {
        charted_as: "pre-existing".into(),
        ..arg(typed, parts, false)
    }
}

/// The medication-list row whose drug reads `primary`, from a Sign's re-read pane.
fn row<'a>(
    pane: &'a Option<crate::chart_set::ChartPane>,
    primary: &str,
) -> &'a cairn_gui_tab_medications::MedListRowView {
    let pane = pane.as_ref().expect("the list read again");
    pane.list
        .rows
        .iter()
        .find(|r| r.primary == primary)
        .unwrap()
}

/// Sign `ids` under Bea's key; the `Done` answer's pane and the ids not charted (with why).
async fn sign_bea(
    state: &AppState,
    chart: Uuid,
    ids: &[String],
) -> (Option<crate::chart_set::ChartPane>, Vec<(String, String)>) {
    match sign(state, chart, ids, credential("bea")).await.unwrap() {
        MedListSigned::Done {
            pane, not_charted, ..
        } => (pane, not_charted),
        other => panic!("expected Done, got {other:?}"),
    }
}

/// A Start line with a seen mark and a whitespace-only reason is refused with the exact copy, and
/// a Sign that could chart nothing never releases the key (ADR-0100 decision 16): nothing is
/// signed. The same measure then moves on a Sign that does chart, so the pin is not vacuous.
#[tokio::test]
async fn a_sign_that_could_chart_nothing_never_releases_the_key() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    amoxicillin_allergy(&state, chart, 0xA40C_9001).await;
    let amox = arg(
        ("amox", "500mg tds"),
        pick("amoxicillin", 0xA40C_9001),
        false,
    );
    let id = tick_seen(&state, chart, amox, Some("   ")).await;
    let before = releases(&owner, "bea").await;
    let (outcome, charted, of, not_charted, _) = done(
        sign(&state, chart, std::slice::from_ref(&id), credential("bea"))
            .await
            .unwrap(),
    );
    assert_eq!((charted, of), (0, 1), "{outcome}");
    assert_eq!(not_charted, [(id, NEEDS_A_REASON.to_string())]);
    assert_eq!(
        releases(&owner, "bea").await,
        before,
        "the key was not released"
    );
    assert_eq!(assertions(&owner, chart).await, 0, "nothing was signed");
    let met = arg(("metformin", "500mg bd"), vec![], true);
    let met = tick_seen(&state, chart, met, None).await;
    sign_bea(&state, chart, &[met]).await;
    assert_eq!(
        releases(&owner, "bea").await,
        before + 1,
        "the measure moves"
    );
}

/// An Already-taking line needs no reason, even marked, and signs what was typed: the patient's
/// account, no start, no override, the pick's code, the directions as the `sig`. The pane read
/// again marks the line.
#[tokio::test]
async fn an_already_taking_line_signs_what_was_typed() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let amox = Uuid::from_u128(0xA40C_9002);
    amoxicillin_allergy(&state, chart, 0xA40C_9002).await;
    let line = already_taking(("amox", "500mg tds"), pick("amoxicillin", 0xA40C_9002));
    let id = tick_seen(&state, chart, line, None).await;
    let (pane, not_charted) = sign_bea(&state, chart, &[id]).await;
    assert!(not_charted.is_empty(), "{not_charted:?}");
    let p = &payloads(&owner, chart).await[0];
    assert_eq!(p["charted_as"], "pre-existing");
    assert_eq!(p["info_source"], "patient-reported");
    assert!(p.get("warning_override").is_none() && p.get("started").is_none());
    assert_eq!(p["substance"]["term"], "amoxicillin");
    assert_eq!(p["substance"]["coding"]["code"], amox.to_string().as_str());
    assert_eq!(p["sig"], "500mg tds");
    assert_eq!(row(&pane, "amoxicillin").allergy_marks.len(), 1);
}

/// No drug list: an uncoded Start line charts, started today, and against a coded allergy it
/// reads "not checked", never clean.
#[tokio::test]
async fn an_uncoded_start_line_charts_and_reads_not_checked() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    amoxicillin_allergy(&state, chart, 0xA40C_9003).await;
    let met = arg(("metformin", "500mg bd"), vec![], false);
    let id = tick_seen(&state, chart, met, None).await;
    let (pane, _) = sign_bea(&state, chart, &[id]).await;
    let p = &payloads(&owner, chart).await[0];
    assert!(p["substance"].get("coding").is_none());
    assert_eq!(p["started"]["precision"], "day");
    let met = row(&pane, "metformin");
    assert!(
        met.allergy_marks.is_empty() && met.allergy_unverified,
        "not checked"
    );
}

/// A Start line over a seen mark signs the reason (trimmed) as the override, naming the allergy
/// record it was given against.
#[tokio::test]
async fn the_signed_event_carries_the_override() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    amoxicillin_allergy(&state, chart, 0xA40C_9004).await;
    let head = list_patient_allergies(&owner, chart).await.unwrap().entries[0].event_id;
    let amox = arg(
        ("amox", "500mg tds"),
        pick("amoxicillin", 0xA40C_9004),
        false,
    );
    let id = tick_seen(&state, chart, amox, Some(" no alternative ")).await;
    sign_bea(&state, chart, &[id]).await;
    let p = &payloads(&owner, chart).await[0];
    assert_eq!(p["charted_as"], "started-here");
    assert_eq!(p["warning_override"]["reason"], "no alternative");
    assert_eq!(
        p["warning_override"]["against"],
        serde_json::json!([{"source": "allergy", "ref": head.to_string()}])
    );
}

/// A combination signs its parts as components, never a single coding (R22: the parts' signed
/// terms joined, an unpicked part keeping its typed words), and the pane read again checks each
/// part: the amoxicillin in it meets the amoxicillin allergy.
#[tokio::test]
async fn a_combination_signs_components() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let amox = Uuid::from_u128(0xA40C_9005);
    amoxicillin_allergy(&state, chart, 0xA40C_9005).await;
    let mut parts = pick("amoxicillin", 0xA40C_9005);
    parts.push(PartArg {
        term: "clav".into(),
        coding: None,
    });
    let line = arg(("amox + clav", "500mg tds"), parts, false);
    let id = tick_seen(&state, chart, line, Some("reason")).await;
    let (pane, not_charted) = sign_bea(&state, chart, &[id]).await;
    assert!(not_charted.is_empty(), "{not_charted:?}");
    let s = &payloads(&owner, chart).await[0]["substance"];
    assert_eq!(s["term"], "amoxicillin + clav");
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
    let combo = row(&pane, "amoxicillin + clav");
    assert_eq!(combo.allergy_marks.len(), 1, "{:?}", combo.allergy_marks);
}

/// A previous adverse effect to codeine on `chart` (matched by name), under Dr Bea's key; its
/// record id.
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
    let patients = vec![patient.clone()];
    allergy_record_impl(state, &patient, patients, entry, None, credential("bea"))
        .await
        .unwrap();
    list_patient_allergies(owner, chart).await.unwrap().entries[0].event_id
}

/// The codeine line, checked as the webview checks it while typing.
async fn check_codeine(state: &AppState, chart: Uuid) -> LineCheckView {
    let patient = chart.to_string();
    let charts = std::slice::from_ref(&patient);
    let parts = vec![PartArg {
        term: "codeine".into(),
        coding: None,
    }];
    med_line_check_impl(state, &patient, charts, "codeine".into(), parts).await
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

/// The warning threshold (batch-charting slice 3, ADR-0104). At the default, a previous adverse
/// effect is a muted note and asks no reason. Once the administrator lowers the threshold
/// (db/081's console door, called here as the owner) the same line warns loud and asks a reason;
/// a blank one is refused, and a reason signs an override naming the adverse-effect record. The
/// charted line is drawn as ADR-0098 draws it: the note muted, no Allergy mark (maintainer Q2).
/// The fixture's TRUNCATE clears `medication_warning_policy`, so the walk starts at the default.
#[tokio::test]
async fn a_lowered_threshold_warns_on_an_adverse_effect() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let record = codeine_adverse_effect(&owner, &state, chart).await;
    let v = check_codeine(&state, chart).await;
    assert!(!v.reason_needed && v.warning_notes.is_empty(), "{v:?}");
    assert_eq!(v.notes.len(), 1, "{v:?}");

    set_threshold(&owner, Some("adverse-effect")).await;
    let v = check_codeine(&state, chart).await;
    assert!(v.reason_needed && v.notes.is_empty(), "{v:?}");
    assert_eq!(v.warning_notes.len(), 1, "{v:?}");
    let bare = tick_seen(
        &state,
        chart,
        arg(("codeine", "30mg qid"), vec![], false),
        None,
    )
    .await;
    let (_, not_charted) = sign_bea(&state, chart, std::slice::from_ref(&bare)).await;
    assert_eq!(not_charted, [(bare.clone(), NEEDS_A_REASON.to_string())]);

    let line = arg(("codeine", "30mg qid"), vec![], false);
    let line = LineArg {
        line_id: bare,
        ..line
    };
    let id = tick_seen(&state, chart, line, Some("tolerated in hospital")).await;
    let (pane, not_charted) = sign_bea(&state, chart, &[id]).await;
    assert!(not_charted.is_empty(), "{not_charted:?}");
    let p = &payloads(&owner, chart).await[0];
    assert_eq!(p["warning_override"]["reason"], "tolerated in hospital");
    assert_eq!(
        p["warning_override"]["against"],
        serde_json::json!([{"source": "allergy", "ref": record.to_string()}])
    );
    let codeine = row(&pane, "codeine");
    assert!(codeine.allergy_marks.is_empty(), "{codeine:?}");
    assert_eq!(codeine.allergy_notes.len(), 1, "{codeine:?}");
    let reread = med_list_impl(&state, &chart.to_string(), None)
        .await
        .unwrap();
    let codeine = reread
        .list
        .rows
        .iter()
        .find(|r| r.primary == "codeine")
        .unwrap();
    assert!(codeine.allergy_marks.is_empty() && codeine.allergy_notes.len() == 1);

    set_threshold(&owner, None).await;
}
