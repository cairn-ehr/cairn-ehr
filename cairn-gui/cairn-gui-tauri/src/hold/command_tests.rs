//! Tests for the withhold and release commands (`hold/mod.rs`), against `AppState::mock` as
//! `commands_tests.rs` tests `cease`: each gesture acts only on the chart on SCREEN, and with the
//! right chart fixture mode's own refusal is what the clinician reads. The writes themselves need
//! a node; what is pinned here without one is the refusal order, `parse_until`, and (by source)
//! that an interrupted commit is sorted from an ordinary failure.
use super::*;
use crate::funnel::view::Retry;

fn open_on_the_fixture_chart() -> (AppState, String) {
    let fixture: Uuid = cairn_gui_data::mock::fixtures::FIXTURE_UUID
        .parse()
        .unwrap();
    (AppState::mock(Some(fixture)), fixture.to_string())
}

fn another_chart() -> String {
    Uuid::from_u128(424_242).to_string()
}

/// Checked FIRST: whatever else would refuse (here a group id that is no line at all), a withhold
/// naming a chart that is not on screen is refused for THAT reason.
#[tokio::test]
async fn withhold_refuses_a_chart_that_is_not_on_screen() {
    let (state, _) = open_on_the_fixture_chart();
    let err = withhold_impl(
        &state,
        &another_chart(),
        vec![another_chart()],
        &another_chart(),
        "nil by mouth",
        "reviewed",
    )
    .await
    .unwrap_err();
    assert!(err.text.contains("not the chart"), "{}", err.text);
}

#[tokio::test]
async fn release_refuses_a_chart_that_is_not_on_screen() {
    let (state, _) = open_on_the_fixture_chart();
    let err = release_impl(
        &state,
        &another_chart(),
        vec![another_chart()],
        &another_chart(),
        &[another_chart()],
        "",
    )
    .await
    .unwrap_err();
    assert!(err.text.contains("not the chart"), "{}", err.text);
}

/// With the right chart, fixture mode refuses before the reason or the time is even read — a
/// mock window writes nothing, whatever it is handed.
#[tokio::test]
async fn a_withhold_on_the_chart_on_screen_meets_the_fixture_refusal() {
    let (state, on_screen) = open_on_the_fixture_chart();
    let err = withhold_impl(
        &state,
        &on_screen,
        vec![on_screen.clone()],
        &another_chart(),
        "",
        "",
    )
    .await
    .unwrap_err();
    assert_eq!(err.retry, Retry::Never);
    assert!(err.text.contains("fixture mode"), "{}", err.text);

    let err = release_impl(
        &state,
        &on_screen,
        vec![on_screen.clone()],
        &another_chart(),
        &["not an id".to_string()],
        "",
    )
    .await
    .unwrap_err();
    assert!(err.text.contains("fixture mode"), "{}", err.text);
}

/// The end of a hold is a clinical fact the clinician must choose (ADR-0081): a time, or "until
/// reviewed". Nothing is ever defaulted, and a text that is not an instant is refused by name.
#[test]
fn parse_until_reads_reviewed_or_an_instant_and_nothing_else() {
    assert_eq!(parse_until("reviewed"), Ok(UntilChoice::Reviewed));
    assert_eq!(
        parse_until(""),
        Err("choose when the hold ends: a time, or until reviewed".to_string())
    );
    for instant in ["2026-10-21T20:00:00.000Z", "2026-10-21T20:00:00+10:00"] {
        assert_eq!(
            parse_until(instant),
            Ok(UntilChoice::At(instant.to_string())),
            "the instant passes through as written"
        );
    }
    assert_eq!(
        parse_until("tomorrow"),
        Err("not a time: tomorrow".to_string())
    );
}

/// #814 for both gestures: each sorts a failed write by the node's marker — an interrupted commit
/// to the not-known sentence (and the report marks the list behind the record), anything else to
/// an ordinary failure. Pinned by source, as `cease_sorts_an_interrupted_commit_from_a_failure`
/// is, because only a lost connection mid-commit reaches that arm.
#[test]
fn both_gestures_sort_an_interrupted_commit_from_a_failure() {
    let src = include_str!("mod.rs");
    let flat = |name| -> String {
        crate::source_guard::rust_fn(src, name)
            .split_whitespace()
            .collect()
    };
    let withhold = flat("withhold_impl");
    for shape in [
        "matchcairn_node::db_diagnosis::commit_outcome_unknown_event(&e){",
        "Some(event)=>Some(UnknownThread{thread:medication_id,event,cause:e.root_cause().to_string(),}),",
        "None=>returnErr(node_failure(&e)),",
        "withhold_outcome(unknown.as_ref())",
    ] {
        assert!(withhold.contains(shape), "{shape}\n{withhold}");
    }
    let release = flat("release_impl");
    for shape in [
        "Err(e)=>matchcairn_node::db_diagnosis::commit_outcome_unknown_event(&e){",
        "Some(event)=>unknown.push(UnknownThread{thread:medication_id,event,cause:e.root_cause().to_string(),}),",
        "(!unknown.is_empty()).then_some(Recorded::Unknown)",
    ] {
        assert!(release.contains(shape), "{shape}\n{release}");
    }
}
