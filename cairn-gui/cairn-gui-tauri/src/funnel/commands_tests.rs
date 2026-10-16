//! Tests for `funnel/commands.rs`: which ids may be opened, and `funnel.js`'s drift
//! guards (moved out of line, #793). The search → register walk is in
//! `commands_register_tests.rs`, which borrows this module's `f` form builder.
use super::*;
use crate::funnel::view::Retry;

use crate::commands::tests::fields_read_in;
use crate::funnel::view::tests::sample_candidate;
use crate::funnel::view::{candidate_view, header_opened_by_id};
use std::collections::BTreeSet;

/// A linked person row of two charts, ids 1 and 2.
fn two_chart_row() -> cairn_patient_search::PersonRow {
    let chart = |n: u128| {
        let mut c = sample_candidate();
        c.patient_id = uuid::Uuid::from_u128(n);
        c
    };
    cairn_patient_search::PersonRow::new(vec![chart(1), chart(2)]).unwrap()
}

/// The maintainer's decision: every member line on screen is its own open target, including
/// a member the search itself did not match (it only rides along in the row).
#[tokio::test]
async fn a_member_the_search_did_not_match_can_be_opened() {
    let state = AppState::mock(None);
    let row = two_chart_row();
    let b = row.members()[1].patient_id;
    let list = CandidateList {
        people: vec![row],
        incomplete: false,
        incomplete_reason: None,
    };
    remember_shown(&state, &list).await;
    let header = open_chart_impl(&state, &b.to_string()).await.unwrap();
    assert_eq!(header.patient_id, b.to_string());
}

#[tokio::test]
async fn an_id_on_no_row_is_still_refused() {
    let state = AppState::mock(None);
    let list = CandidateList {
        people: vec![two_chart_row()],
        incomplete: false,
        incomplete_reason: None,
    };
    remember_shown(&state, &list).await;
    let stranger = uuid::Uuid::from_u128(99).to_string();
    let refused = open_chart_impl(&state, &stranger)
        .await
        .expect_err("an id on no row is refused");
    assert!(refused.contains("not in a list on screen"), "{refused}");
}

/// THE FRONT DOOR'S DRIFT GUARD — the same guard `commands.rs` keeps for `main.js`.
///
/// `funnel.js` is untyped, so a Rust field rename does not break the build: it renders
/// `undefined`. On this screen that is not cosmetic. `incomplete_reason` undefined is a
/// partial list shown as whole; `retry` undefined is a Register button left live on a
/// verdict. So every field the JS reads must exist in the payload the Rust sends.
#[test]
fn funnel_js_reads_no_field_the_backend_does_not_send() {
    let js = include_str!("../../src-ui/funnel.js");
    let header = header_opened_by_id(uuid::Uuid::nil());
    let cand = candidate_view(&sample_candidate());
    // A LINKED sample, so `label` is present and `members` has more than one entry.
    let linked = person_row_view(&two_chart_row());
    let payloads: [(&str, serde_json::Value); 7] = [
        (
            "status",
            serde_json::to_value(FunnelStatus {
                mock: true,
                provisioning: None,
                chart: Some(header.clone()),
                revision: 0,
            })
            .unwrap(),
        ),
        ("header", serde_json::to_value(&header).unwrap()),
        ("cand", serde_json::to_value(&cand).unwrap()),
        ("row", serde_json::to_value(&linked).unwrap()),
        (
            "browseView",
            serde_json::to_value(browse_view(
                0,
                &CandidateList {
                    people: vec![two_chart_row()],
                    incomplete: false,
                    incomplete_reason: None,
                },
            ))
            .unwrap(),
        ),
        (
            "prompt",
            serde_json::to_value(PromptView::saying(0, String::new())).unwrap(),
        ),
        (
            "failure",
            serde_json::to_value(ErrorView {
                text: String::new(),
                retry: Retry::Now,
            })
            .unwrap(),
        ),
    ];
    for (binding, value) in payloads {
        let available: BTreeSet<String> = value.as_object().unwrap().keys().cloned().collect();
        let read = fields_read_in(js, binding);
        assert!(!read.is_empty(), "funnel.js no longer reads `{binding}` at all — rename the binding in this guard, don't delete it");
        for field in read {
            assert!(
                available.contains(&field),
                "funnel.js reads `{binding}.{field}`, which the backend does not send. \
                 Available: {available:?}"
            );
        }
    }
}

/// The other direction, for the fields whose SILENCE is the dangerous failure.
#[test]
fn funnel_js_reads_both_incompleteness_reports_and_the_retry_advice() {
    let js = include_str!("../../src-ui/funnel.js");
    assert!(fields_read_in(js, "browseView").contains("incomplete_reason"));
    assert!(fields_read_in(js, "browseView").contains("summary"));
    assert!(fields_read_in(js, "prompt").contains("incomplete_reason"));
    assert!(fields_read_in(js, "prompt").contains("stale"));
    // A linked row is drawn from its Rust-worded label and its members; losing either read
    // would draw an unlabelled or empty person.
    assert!(fields_read_in(js, "row").contains("members"));
    assert!(fields_read_in(js, "row").contains("label"));
    assert!(fields_read_in(js, "failure").contains("retry"));
    assert!(fields_read_in(js, "status").contains("provisioning"));
}

/// A form snapshot at revision `rev` (shared with `register_tests`).
pub(super) fn f(rev: u64, name: &str, dob: &str) -> FormSnapshot {
    FormSnapshot {
        revision: rev,
        raw_name: name.into(),
        birth_date: dob.into(),
    }
}

#[tokio::test]
async fn browse_then_open_a_displayed_candidate_opens_that_chart() {
    let state = AppState::mock(None);
    let list = browse_impl(&state, f(1, "mich", "")).await.unwrap();
    let id = list.people[0].members[0].patient_id.clone();
    let header = open_chart_impl(&state, &id).await.unwrap();
    assert_eq!(header.patient_id, id);
    assert_eq!(state.open_patient().await.unwrap().to_string(), id);
}

/// Review Focus 3: only a candidate from a list the backend itself returned may be opened.
#[tokio::test]
async fn only_a_displayed_candidate_can_be_opened() {
    let state = AppState::mock(None);
    let never_shown = uuid::Uuid::from_u128(424_242).to_string();
    assert!(open_chart_impl(&state, &never_shown).await.is_err());
    assert!(state.open_patient().await.is_err());
}

/// A prompt candidate ("this is them") stays openable after a later browse.
#[tokio::test]
async fn a_prompt_candidate_survives_a_later_browse() {
    let state = AppState::mock(None);
    let p = prompt_search_impl(&state, f(1, "Samantha Michaelowski", "1975"), true)
        .await
        .unwrap();
    let id = p.people[0].members[0].patient_id.clone();
    browse_impl(&state, f(1, "zzzz-nobody", "")).await.unwrap();
    open_chart_impl(&state, &id).await.expect("still on screen");
}
