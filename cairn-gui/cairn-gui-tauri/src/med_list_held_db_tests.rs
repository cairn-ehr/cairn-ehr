//! DB-gated walks for the Task 6 review's fix round (on `CAIRN_TEST_PG`, run locally — #905),
//! beside `med_list_db_tests.rs`, whose helpers these share.
//!
//! - C1: unticking a line the DEFAULT tick gives is the clinician's veto (`held`), stored: a
//!   re-read never brings the line back ticked; changed words clear it; a held id sent to Sign
//!   is a changed picture.
//! - I2: a duplicate line says which line it duplicates, in Rust's words — another line of the
//!   list, or a current line of the chart.
use super::db_tests::{arg, bea_list, done, pick, read, save_as, sign};
use super::*;
use crate::allergies::allergy_none_impl;
use crate::med_list_sign::MedListSigned;
use crate::notes::review::db_tests::credential;

/// "No known allergies", so a complete coded line gets the default tick.
async fn none_known(state: &AppState, chart: Uuid) {
    let patient = chart.to_string();
    allergy_none_impl(
        state,
        &patient,
        vec![patient.clone()],
        "all",
        credential("bea"),
    )
    .await
    .unwrap();
}

/// A complete, coded Already-taking line: default-ticked on a chart with an allergy list.
fn complete(directions: &str, moiety: u128) -> LineArg {
    let mut line = arg(
        ("amoxicillin", directions),
        pick("amoxicillin", moiety),
        false,
    );
    line.charted_as = "pre-existing".into();
    line
}

fn line<'a>(view: &'a MedListView, id: &str) -> &'a ListLineView {
    view.lines.iter().find(|l| l.line_id == id).unwrap()
}

/// C1: the untick is stored, so the line reads unticked on every later read; changing its words
/// clears the veto, and the default tick is given again.
#[tokio::test]
async fn an_untick_of_a_default_tick_survives_a_re_read_until_the_words_change() {
    let Some((_guard, _owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    none_known(&state, chart).await;
    let bea = Some("bea");
    let mut l = complete("500mg cap tds", 0xA40C_9001);
    let id = l.line_id.clone();
    assert_eq!(save_as(&state, chart, l.clone(), bea).await, "saved");
    assert!(
        line(&read(&state, chart).await, &id).ticked,
        "default-ticked"
    );
    l.held = true;
    assert_eq!(save_as(&state, chart, l.clone(), bea).await, "saved");
    let view = read(&state, chart).await;
    let drawn = line(&view, &id);
    assert!(
        drawn.held && drawn.default_ticked && !drawn.ticked,
        "{drawn:?}"
    );
    // The words change (the window clears `held` too; the door does whatever it is sent).
    l.directions = "250mg cap tds".into();
    l.line_text = "amoxicillin 250mg cap tds".into();
    assert_eq!(save_as(&state, chart, l, bea).await, "saved");
    let view = read(&state, chart).await;
    let drawn = line(&view, &id);
    assert!(!drawn.held && drawn.ticked, "{drawn:?}");
}

/// C1: a held line's id sent to Sign (a stale or lying screen) is a changed picture: nothing is
/// signed.
#[tokio::test]
async fn a_held_line_sent_ticked_is_changed() {
    let Some((_guard, _owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    none_known(&state, chart).await;
    let mut l = complete("500mg cap tds", 0xA40C_9002);
    l.held = true;
    let id = l.line_id.clone();
    assert_eq!(save_as(&state, chart, l, Some("bea")).await, "saved");
    match sign(&state, chart, std::slice::from_ref(&id), credential("bea"))
        .await
        .unwrap()
    {
        MedListSigned::Changed { lines, .. } => assert_eq!(lines, [id]),
        other => panic!("expected Changed, got {other:?}"),
    }
}

/// I2: two lines of the list on one moiety each name the other.
#[tokio::test]
async fn a_draft_duplicate_names_the_other_line() {
    let Some((_guard, _owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let bea = Some("bea");
    let (a, b) = (
        complete("500mg cap tds", 0xA40C_9003),
        complete("1g tab bd", 0xA40C_9003),
    );
    assert_eq!(save_as(&state, chart, a.clone(), bea).await, "saved");
    assert_eq!(save_as(&state, chart, b.clone(), bea).await, "saved");
    let view = read(&state, chart).await;
    assert_eq!(
        line(&view, &a.line_id).duplicate_of.as_deref(),
        Some("already on the list: amoxicillin 1g tab bd")
    );
    assert_eq!(
        line(&view, &b.line_id).duplicate_of.as_deref(),
        Some("already on the list: amoxicillin 500mg cap tds")
    );
}

/// I2: a line on a moiety the chart already holds as a current medication names that line.
#[tokio::test]
async fn a_chart_duplicate_names_the_chart_line() {
    let Some((_guard, _owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    none_known(&state, chart).await;
    let bea = Some("bea");
    let first = complete("500mg cap tds", 0xA40C_9004);
    assert_eq!(save_as(&state, chart, first.clone(), bea).await, "saved");
    let (outcome, charted, ..) = done(
        sign(
            &state,
            chart,
            std::slice::from_ref(&first.line_id),
            credential("bea"),
        )
        .await
        .unwrap(),
    );
    assert_eq!(charted, 1, "{outcome}");
    let again = complete("1g tab bd", 0xA40C_9004);
    assert_eq!(save_as(&state, chart, again.clone(), bea).await, "saved");
    let view = read(&state, chart).await;
    let drawn = line(&view, &again.line_id);
    assert!(drawn.duplicate, "{drawn:?}");
    let words = drawn.duplicate_of.as_deref().unwrap_or_default();
    assert!(
        words.starts_with("already on the list: ") && words.contains("amoxicillin"),
        "{words}"
    );
}
