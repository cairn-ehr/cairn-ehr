//! The front door's search → register walk, end to end against `AppState::mock`
//! (moved out of `funnel/commands.rs`, #793): a token is minted only for a searched
//! form, redeemed once, invalidated by an edit or a close, and the registration attests
//! exactly the rows the prompt showed. Also the window's open/close chart state.
use super::tests::f;
use super::*;
use crate::funnel::view::{Recorded, Retry};
use cairn_gui_data::port::DataError;
use cairn_gui_funnel::TokenError;

#[tokio::test]
async fn the_whole_register_walk_opens_the_new_chart_and_the_next_browse_finds_it() {
    let state = AppState::mock(None);
    let p = prompt_search_impl(&state, f(1, "Zebedee Quixote", "1990-05-05"), false)
        .await
        .unwrap();
    assert!(p.waiting.is_none(), "{p:?}");
    let token = p.token.expect("a searchable form mints a token");
    let header = register_impl(&state, token).await.unwrap();
    assert_eq!(header.name, "Zebedee Quixote");
    assert_eq!(header.born, "born 1990-05-05");
    assert_eq!(
        state.open_patient().await.unwrap().to_string(),
        header.patient_id
    );
    close_chart_impl(&state).await;
    let found = browse_impl(&state, f(2, "quixote", "")).await.unwrap();
    assert!(found
        .people
        .iter()
        .flat_map(|r| &r.members)
        .any(|c| c.patient_id == header.patient_id));
}

/// Review Focus 2: a mononymous patient never trips the trigger, and must still be
/// registrable — the forced search runs on whatever is typed.
#[tokio::test]
async fn a_forced_search_runs_even_when_the_trigger_is_waiting() {
    let state = AppState::mock(None);
    let waiting = prompt_search_impl(&state, f(1, "Cher", ""), false)
        .await
        .unwrap();
    assert!(waiting.waiting.is_some() && waiting.token.is_none());
    let forced = prompt_search_impl(&state, f(1, "Cher", ""), true)
        .await
        .unwrap();
    let token = forced.token.expect("Register must still be reachable");
    register_impl(&state, token).await.unwrap();
}

/// An empty form is refused with the token store's own sentence, not searched.
#[tokio::test]
async fn an_empty_form_mints_no_token_and_says_why() {
    let state = AppState::mock(None);
    let p = prompt_search_impl(&state, f(1, " ", ""), true)
        .await
        .unwrap();
    assert!(p.token.is_none());
    assert!(p.waiting.unwrap().contains("nothing to search on"));
}

#[tokio::test]
async fn a_stale_prompt_search_is_reported_stale_and_mints_nothing() {
    let state = AppState::mock(None);
    form_edited_impl(&state, 5).await;
    let p = prompt_search_impl(&state, f(4, "Jon Smith", "1980"), true)
        .await
        .unwrap();
    assert!(p.stale && p.token.is_none(), "{p:?}");
}

#[tokio::test]
async fn a_second_register_with_the_same_token_is_refused() {
    let state = AppState::mock(None);
    let t = prompt_search_impl(&state, f(1, "Ada Byron", "1815-12-10"), false)
        .await
        .unwrap()
        .token
        .unwrap();
    register_impl(&state, t).await.unwrap();
    close_chart_impl(&state).await;
    let second = register_impl(&state, t).await.unwrap_err();
    // THE token refusal — not a provisioning or mock failure that would pass for the wrong
    // reason — and no second chart behind it.
    assert_eq!(
        second,
        token_error_view(TokenError::Absent),
        "one search, one chart"
    );
    let found = browse_impl(&state, f(2, "byron", "")).await.unwrap();
    assert_eq!(found.people.len(), 1, "{found:?}");
}

#[tokio::test]
async fn an_edit_after_the_prompt_makes_its_token_unredeemable() {
    let state = AppState::mock(None);
    let t = prompt_search_impl(&state, f(1, "Jon Smith", "1980"), false)
        .await
        .unwrap()
        .token
        .unwrap();
    form_edited_impl(&state, 2).await;
    assert_eq!(
        register_impl(&state, t).await.unwrap_err(),
        token_error_view(TokenError::Absent)
    );
}

#[tokio::test]
async fn a_failed_registration_keeps_the_search_and_the_retry_works() {
    let state = AppState::mock(None);
    let t = prompt_search_impl(&state, f(1, "Ada Byron", "1815"), false)
        .await
        .unwrap()
        .token
        .unwrap();
    state
        .mock_data()
        .unwrap()
        .fail_next(DataError::Unavailable("disk full".into()));
    let err = register_impl(&state, t).await.unwrap_err();
    assert_eq!(err.retry, Retry::Now);
    assert!(state.open_patient().await.is_err(), "nothing was opened");
    register_impl(&state, t)
        .await
        .expect("the Kept search registers on retry");
}

/// #815: a registration whose commit got no ordinary answer may have created the chart. The
/// clerk is told it is NOT known (never "nothing was saved") and the same search cannot register
/// again: a second press would mint a second chart for the same person.
#[tokio::test]
async fn an_unknown_registration_outcome_withholds_the_retry_and_spends_the_search() {
    let state = AppState::mock(None);
    let t = prompt_search_impl(&state, f(1, "Ada Byron", "1815"), false)
        .await
        .unwrap()
        .token
        .unwrap();
    state
        .mock_data()
        .unwrap()
        .fail_next(DataError::OutcomeUnknown("connection closed".into()));
    let err = register_impl(&state, t).await.unwrap_err();
    // Not a live retry; and the signal funnel.js reads to forget every token it holds.
    assert_eq!(err.retry, Retry::AfterReread(Recorded::Unknown));
    assert!(err.text.contains("NOT known"), "{}", err.text);
    assert!(!err.text.contains("Nothing was saved"), "{}", err.text);
    assert_eq!(
        register_impl(&state, t).await.unwrap_err(),
        token_error_view(TokenError::Absent)
    );
}

/// #815 review: after an unknown outcome funnel.js forgets the token it holds even when a newer
/// search replaced the one sent — `settle` spent that one too. Otherwise the next press is
/// refused `Absent`, and that refusal overwrites the "NOT known" sentence. The webview has no
/// test rig of its own (no npm: main.js's header), so this reads the source.
#[test]
fn funnel_js_forgets_every_token_after_an_unknown_registration() {
    let register = crate::source_guard::js_fn(include_str!("../../src-ui/funnel.js"), "onRegister");
    assert!(
        register.contains(r#"const spent = behindTheRecord(failure) === "unknown";"#),
        "{register}"
    );
    assert!(register.contains("if (spent || "), "{register}");
}

#[tokio::test]
async fn a_refused_registration_withholds_the_retry() {
    let state = AppState::mock(None);
    let t = prompt_search_impl(&state, f(1, "Ada Byron", "1815"), false)
        .await
        .unwrap()
        .token
        .unwrap();
    state
        .mock_data()
        .unwrap()
        .fail_next(DataError::Refused("bad date".into()));
    let err = register_impl(&state, t).await.unwrap_err();
    assert_eq!(err.retry, Retry::Never);
    assert!(err.text.contains("bad date"));
}

/// A failed browse must never reach the clerk as an empty list.
#[tokio::test]
async fn a_failed_browse_is_an_error_not_an_empty_list() {
    let state = AppState::mock(None);
    state
        .mock_data()
        .unwrap()
        .fail_next(DataError::Unavailable("gone".into()));
    let err = browse_impl(&state, f(1, "mich", "")).await.unwrap_err();
    assert!(err.text.contains("NOT"), "{}", err.text);
}

/// Review Focus 5.
#[tokio::test]
async fn chart_commands_refuse_when_no_chart_is_open() {
    let state = AppState::mock(None);
    let err = state.open_patient().await.unwrap_err();
    assert!(err.contains("no chart is open"), "{err}");
}

#[tokio::test]
async fn a_patient_given_at_launch_opens_straight_on_the_chart() {
    let id = uuid::Uuid::from_u128(7);
    let state = AppState::mock(Some(id));
    assert_eq!(state.open_patient().await.unwrap(), id);
    let status = funnel_status_impl(&state).await;
    assert!(status.mock);
    assert_eq!(status.chart.unwrap().patient_id, id.to_string());
}

/// PR #674 review #3: a reloaded webview restarts its revision counter at 0 while the backend's
/// floor survives, so every search would come back stale and Register would go dead with no
/// word. The status reports the floor so the webview can resume above it.
#[tokio::test]
async fn the_status_reports_the_revision_floor_so_a_reload_can_resume() {
    let state = AppState::mock(None);
    form_edited_impl(&state, 7).await;
    let status = funnel_status_impl(&state).await;
    assert_eq!(status.revision, 7);
    let p = prompt_search_impl(&state, f(status.revision + 1, "Ada Byron", "1815"), false)
        .await
        .unwrap();
    assert!(!p.stale && p.token.is_some(), "{p:?}");
}

/// PR #674 review #6, the command half: a minted prompt carries its announcement.
#[tokio::test]
async fn a_minted_prompt_carries_its_summary() {
    let state = AppState::mock(None);
    let p = prompt_search_impl(&state, f(1, "Samantha Michaelowski", "1975"), false)
        .await
        .unwrap();
    assert!(p.summary.unwrap().contains("none of these"));
}

#[tokio::test]
async fn closing_the_chart_returns_to_the_front_door() {
    let state = AppState::mock(Some(uuid::Uuid::from_u128(7)));
    close_chart_impl(&state).await;
    assert!(state.open_patient().await.is_err());
    assert!(funnel_status_impl(&state).await.chart.is_none());
}

/// Closing the chart resets the form, so a search held for it must not survive to be
/// attested by the next registration.
#[tokio::test]
async fn closing_the_chart_discards_a_held_search() {
    let state = AppState::mock(None);
    let t = prompt_search_impl(&state, f(1, "Ada Byron", "1815"), false)
        .await
        .unwrap()
        .token
        .unwrap();
    close_chart_impl(&state).await;
    assert_eq!(
        register_impl(&state, t).await.unwrap_err(),
        token_error_view(TokenError::Absent)
    );
}

/// Register one fixture patient through the whole walk and return to the front door.
async fn register_through_the_walk(state: &AppState, rev: u64, name: &str) -> uuid::Uuid {
    let p = prompt_search_impl(state, f(rev, name, ""), true)
        .await
        .unwrap();
    let header = register_impl(state, p.token.expect("a token"))
        .await
        .unwrap();
    close_chart_impl(state).await;
    header.patient_id.parse().unwrap()
}

/// WHAT IS SHOWN IS WHAT IS SIGNED, end to end (PR #674 review). More candidates than the
/// prompt may show: the prompt carries exactly `PROMPT_CAP` rows and says how many it cut,
/// and the registration attests exactly those rows, in that order — not the unbounded list.
#[tokio::test]
async fn the_prompt_is_bounded_and_the_registration_attests_exactly_its_rows() {
    let state = AppState::mock(None);
    let mut rev = 0;
    for i in 0..=cairn_gui_funnel::PROMPT_CAP {
        rev += 1;
        register_through_the_walk(&state, rev, &format!("Zanzibar Kin{i}")).await;
    }
    rev += 1;
    let p = prompt_search_impl(&state, f(rev, "Zanzibar Newcomer", ""), true)
        .await
        .unwrap();
    assert_eq!(p.people.len(), cairn_gui_funnel::PROMPT_CAP, "{p:?}");
    // ADR-0075: a cut prompt says how many it did not show, and does NOT call the search
    // partial — that word is kept for a search that could not read a chart.
    assert!(
        p.incomplete_reason.is_none(),
        "a cut prompt is not a partial search: {p:?}"
    );
    // The COUNT, not just the phrase: PROMPT_CAP + 1 charts matched, so exactly one was cut
    // (a wrong `withheld`, or the raw list length passed for it, would still say "closest of").
    let expected = format!(
        "{} closest of {}",
        cairn_gui_funnel::PROMPT_CAP,
        cairn_gui_funnel::PROMPT_CAP + 1
    );
    assert!(
        p.summary.as_deref().unwrap_or("").contains(&expected),
        "a cut prompt must say how many it cut: {p:?}"
    );
    let shown: Vec<uuid::Uuid> = p
        .people
        .iter()
        .flat_map(|r| &r.members)
        .map(|c| c.patient_id.parse().unwrap())
        .collect();
    let header = register_impl(&state, p.token.unwrap()).await.unwrap();
    let attested = state
        .mock_data()
        .unwrap()
        .attested_display(header.patient_id.parse().unwrap());
    assert_eq!(attested, Some(shown));
}

/// A candidate the cap withheld was never on screen, so it can never be opened.
#[tokio::test]
async fn a_candidate_the_cap_withheld_cannot_be_opened() {
    let state = AppState::mock(None);
    let mut registered = Vec::new();
    let mut rev = 0;
    for i in 0..=cairn_gui_funnel::PROMPT_CAP {
        rev += 1;
        registered.push(register_through_the_walk(&state, rev, &format!("Zanzibar Kin{i}")).await);
    }
    rev += 1;
    let p = prompt_search_impl(&state, f(rev, "Zanzibar Newcomer", ""), true)
        .await
        .unwrap();
    let withheld = registered
        .iter()
        .find(|id| {
            !p.people
                .iter()
                .flat_map(|r| &r.members)
                .any(|c| c.patient_id == id.to_string())
        })
        .expect("one more than the cap was registered");
    assert!(open_chart_impl(&state, &withheld.to_string())
        .await
        .is_err());
}

/// The header over a chart just registered must agree with what the next search says of the
/// same chart — two trust states on screen for one chart is a precise untruth on the very
/// affordance that prevents wrong-chart acts (PR #674 review).
#[tokio::test]
async fn a_registered_charts_header_agrees_with_the_next_browse() {
    let state = AppState::mock(None);
    let p = prompt_search_impl(&state, f(1, "Zebedee Quixote", "1990-05-05"), false)
        .await
        .unwrap();
    let header = register_impl(&state, p.token.unwrap()).await.unwrap();
    close_chart_impl(&state).await;
    let found = browse_impl(&state, f(2, "quixote", "")).await.unwrap();
    let row = found
        .people
        .iter()
        .flat_map(|r| &r.members)
        .find(|c| c.patient_id == header.patient_id)
        .unwrap();
    assert_eq!(header.trust, row.trust);
}

/// "This is them" was clicked while a Register was still on its way: the chart the clerk
/// RECOGNISED is open, so the registration must not go ahead behind it (PR #674 review).
#[tokio::test]
async fn a_registration_arriving_after_a_chart_was_opened_is_refused_unsaved() {
    let state = AppState::mock(None);
    let p = prompt_search_impl(&state, f(1, "Samantha Michaelowski", "1975"), true)
        .await
        .unwrap();
    let recognised = p.people[0].members[0].patient_id.clone();
    open_chart_impl(&state, &recognised).await.unwrap();
    let err = register_impl(&state, p.token.unwrap()).await.unwrap_err();
    assert_eq!(err.retry, Retry::Never);
    assert!(err.text.contains("Nothing was registered"), "{}", err.text);
    assert_eq!(state.open_patient().await.unwrap().to_string(), recognised);
    assert_eq!(state.mock_data().unwrap().registered_count(), 0);
}

/// The same race, one step later: the registration was already saving when the clerk
/// opened a chart, and it commits. The window must stay on the chart the clerk chose — and
/// SAY that a new chart now exists, because it may be a duplicate of the one on screen.
#[tokio::test]
async fn a_registration_committing_after_a_chart_was_opened_leaves_the_clerk_on_it() {
    let a = uuid::Uuid::from_u128(1);
    let state = AppState::mock(Some(a));
    let new = uuid::Uuid::from_u128(2);
    let err = open_after_registering(
        &state,
        new,
        header_from_registration(new, "Ada Byron", Some("1815")),
    )
    .await
    .unwrap_err();
    assert_eq!(err.retry, Retry::Never);
    assert!(err.text.contains(&new.to_string()), "{}", err.text);
    assert!(err.text.contains("duplicate"), "{}", err.text);
    assert_eq!(
        state.open_patient().await.unwrap(),
        a,
        "still on the chart the clerk chose"
    );
}

/// Review Focus: the provisioning check runs BEFORE the search is taken, so a node that may
/// not write refuses with the search intact — and the same search registers once an
/// operator has acted.
#[tokio::test]
async fn an_unprovisioned_node_refuses_with_the_search_intact() {
    let state = AppState::mock(None);
    let t = prompt_search_impl(&state, f(1, "Ada Byron", "1815"), false)
        .await
        .unwrap()
        .token
        .unwrap();
    state
        .mock_data()
        .unwrap()
        .fail_next(DataError::NotProvisioned("enrol this node".into()));
    let err = register_impl(&state, t).await.unwrap_err();
    assert_eq!(err.retry, Retry::AfterOperator);
    register_impl(&state, t)
        .await
        .expect("the search was never taken, so it still registers");
}

/// An id the webview could not have sent from a list says so, rather than blaming the list.
#[tokio::test]
async fn an_unreadable_chart_id_is_named_as_unreadable() {
    let state = AppState::mock(None);
    let err = open_chart_impl(&state, "not-a-uuid").await.unwrap_err();
    assert!(err.contains("could not read"), "{err}");
}

/// Browse carries the node's bare `incomplete` flag through even with no prose, and its
/// announcement comes from Rust.
#[test]
fn a_bare_incomplete_flag_reaches_the_browse_view() {
    let list = cairn_patient_search::CandidateList {
        people: vec![],
        incomplete: true,
        incomplete_reason: None,
    };
    let v = browse_view(1, &list);
    assert!(v.incomplete_reason.is_some(), "{v:?}");
    assert!(
        !v.summary.contains("No existing chart matched"),
        "{}",
        v.summary
    );
}
