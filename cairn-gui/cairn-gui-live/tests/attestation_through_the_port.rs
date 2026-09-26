//! DB-gated: what the funnel's live ports actually do to a real record.
//!
//! The root tree already proves that `register_patient` stores the candidate list it is
//! handed, in order (`patient_register.rs`'s
//! `the_attestation_round_trips_from_the_displayed_list_to_the_stored_body`). This file proves
//! something that suite CANNOT: that what the PORT hands it is the list the step-3 prompt
//! actually bounded — the `PromptList`, not the node's raw answer. A prompt showing five could
//! otherwise swear it displayed forty, and slice 2a's own end-to-end walk had exactly that bug
//! until `bound_for_prompt` gained a call site.
mod common;

use cairn_gui_data::port::{PatientRegistration, PatientSearch};
use cairn_gui_funnel::{bound_for_prompt, TokenStore, PROMPT_CAP};
use cairn_gui_live::LiveData;
use cairn_patient_search::SearchQuery;

/// A fixed clock, so a displayed age never changes under the suite.
const TODAY: &str = "2026-09-22";

/// The birth date every chart in this file is registered under, and the age it implies on
/// [`TODAY`]. Written as a pair so the two cannot drift: an age assertion computed from a
/// date the test also chose is the only way `today`'s passthrough is observable at all.
const BORN: &str = "1991-03-04";
const AGE_ON_TODAY: u32 = 35;

/// This node's origin id in the fixture.
///
/// DISTINCTIVE on purpose. The root tree's suites use `"n"`, which is exactly the value a port
/// that ignored its `Identity` and hardcoded something would pick — a fixture that cannot tell
/// the right answer from the lazy one proves nothing when it passes.
const ORIGIN: &str = "gui-live-origin";

/// A search that matches nothing must be an EMPTY list, never an error.
///
/// "The search failed" and "nobody matched" are different answers and only one of them is
/// evidence of absence — precisely the distinction that decides whether a clerk creates a
/// duplicate chart (principle 4). The port's own doc requires this; nothing enforced it until
/// here.
#[tokio::test]
async fn nobody_matched_is_an_empty_list_not_a_failure() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;
    let live = LiveData::new(
        common::connect_for_live(&cs).await,
        sk,
        &common::identity(ORIGIN),
    );

    let query = SearchQuery::new("zzzznobodyzzzz", Some("1900-01-01"), &[]);
    let found = live
        .search(&query, TODAY)
        .await
        .expect("a search that matches nothing still SUCCEEDS");

    assert!(found.candidates.is_empty(), "got: {:?}", found.candidates);
    assert!(
        !found.incomplete,
        "an exhaustive search that found nothing is COMPLETE — marking it partial would tell \
         the clerk to keep looking for a chart that does not exist"
    );
}

/// The walk the design is about: browse, nothing fits, register, and the chart is findable
/// afterwards by the very name that was typed.
///
/// The last clause is what makes this more than a smoke test. `register_patient` asserts the
/// typed name and date of birth as demographics precisely so a registered chart is findable
/// (#350); a port that dropped `name` on the floor would still return a `Uuid` and still pass
/// every assertion about the attestation — and would create a chart nobody can ever find
/// again, which is the failure the whole funnel exists to prevent.
#[tokio::test]
async fn a_registration_creates_a_chart_the_next_search_finds() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;
    let live = LiveData::new(
        common::connect_for_live(&cs).await,
        sk,
        &common::identity(ORIGIN),
    );

    // The ONE typed string, feeding both the query and the asserted name — never reassembled
    // from separate given/family boxes, which is the name model ADR-0014 forbids and the
    // drift `register_patient`'s own doc warns it cannot enforce.
    let typed = "Ngaiterangi Waiariki";
    let query = SearchQuery::new(typed, Some(BORN), &[]);

    let before = live.search(&query, TODAY).await.expect("the browse search");
    assert!(
        before.candidates.is_empty(),
        "the fixture starts empty: {:?}",
        before.candidates
    );

    let mut store = TokenStore::new();
    let token = store
        .record(query.clone(), bound_for_prompt(&before))
        .expect("a non-empty query mints a token");
    let attested = store.take(token).expect("the token is redeemable");

    let created = live
        .register(attested, Some(typed))
        .await
        .map_err(|(e, _returned)| e)
        .expect("a well-formed registration is accepted");
    store.commit();

    // BY THE NAME ALONE, with no date of birth in the query — and that is the whole point of
    // this assertion rather than a detail of it. db/046 pass 2 is an exact match on the birth
    // date, which `register_patient` asserts from the QUERY whatever happens to `name`; so a
    // verification search carrying a dob finds the chart even when the port dropped the typed
    // name on the floor. This test passed under exactly that mutation until the dob came out.
    let by_name_only = SearchQuery::new(typed, None, &[]);
    let after = live
        .search(&by_name_only, TODAY)
        .await
        .expect("the browse search");
    let ids: Vec<_> = after.candidates.iter().map(|c| c.patient_id).collect();
    assert!(
        ids.contains(&created),
        "the chart just registered must be findable by the NAME it was registered under — \
         got {ids:?}"
    );

    // `today` REACHES THE DISPLAYED AGE, and until this assertion existed nothing made that
    // observable. `TODAY` was threaded through every call in this file and its value was never
    // read back, so substituting the port's own clock — `Utc::now()`, or `current_date` inside
    // the query — passed the whole suite. Age beside a name is the primary human discriminator
    // between two same-name charts on the step-3 prompt; a drifting one is how the clerk fails
    // to recognise the chart that is already there.
    let found = after
        .candidates
        .iter()
        .find(|c| c.patient_id == created)
        .expect("just asserted it is in the list");
    let age = found
        .age
        .as_ref()
        .expect("a chart registered with a full date of birth displays an age");
    assert_eq!(
        age.years, AGE_ON_TODAY,
        "born {BORN}, asked as at {TODAY} — a different answer means the port substituted its \
         own clock, and nothing on screen would say which one won"
    );

    // An exhaustive search that found everything is COMPLETE, and the signed body must say so.
    // The positive half is `a_search_that_could_not_read_a_chart_still_signs_incomplete`;
    // together they stop `incomplete` being a constant (ADR-0075).
    assert!(
        !common::stored_incomplete(&reader, created).await,
        "the search read every chart it matched, so the attestation must not call it partial"
    );
}

/// THE TEST THIS FILE EXISTS FOR.
///
/// A registration must swear to the list the PROMPT bounded, not to the node's raw answer.
/// With more namesakes than `PROMPT_CAP`, the two are genuinely different values — and a
/// stored body naming all of them would be a signed claim that the clerk saw a screenful they
/// never saw, which is exactly the claim someone would later use to argue they should have
/// spotted the duplicate (design decision 3).
#[tokio::test]
async fn the_stored_attestation_names_what_the_prompt_bounded_and_nothing_more() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;
    let live = LiveData::new(
        common::connect_for_live(&cs).await,
        sk,
        &common::identity(ORIGIN),
    );

    // Register more namesakes than the prompt can show, so that the raw list and the bounded
    // one cannot be the same value by accident.
    let shared = "Kowalczyk";
    for n in 0..(PROMPT_CAP + 3) {
        let typed = format!("{shared} Number{n}");
        let q = SearchQuery::new(&typed, Some("1970-01-01"), &[]);
        let mut store = TokenStore::new();
        let t = store
            .record(q, bound_for_prompt(&common::nothing_found()))
            .expect("a non-empty query mints a token");
        let a = store.take(t).expect("redeemable");
        live.register(a, Some(&typed))
            .await
            .map_err(|(e, _)| e)
            .expect("each namesake registers");
        store.commit();
    }

    let query = SearchQuery::new(shared, None, &[]);
    let raw = live.search(&query, TODAY).await.expect("the step-3 search");
    assert!(
        raw.candidates.len() > PROMPT_CAP,
        "the fixture must OVERFLOW the prompt or this test proves nothing — got {}",
        raw.candidates.len()
    );

    let prompt = bound_for_prompt(&raw);
    let shown: Vec<_> = prompt
        .as_list()
        .candidates
        .iter()
        .map(|c| c.patient_id)
        .collect();

    let mut store = TokenStore::new();
    let token = store.record(query, prompt).expect("a token");
    let attested = store.take(token).expect("redeemable");
    let created = live
        .register(attested, Some("Kowalczyk Newcomer"))
        .await
        .map_err(|(e, _)| e)
        .expect("registration accepted");
    store.commit();

    // BOTH halves of the attested pair, because either one alone can be right while the
    // registration is still a lie. `AttestedSearch` carries them together so they cannot
    // disagree; this is what checks that what reached the signed body is still that pair.
    let (tokens, birth_date) = common::stored_query(&reader, created).await;
    assert_eq!(
        tokens,
        vec![shared.to_lowercase()],
        "the attested query must be the one that produced the list, not the newcomer's name"
    );
    assert_eq!(
        birth_date, None,
        "the step-3 search carried no date of birth, and the attestation must say so rather \
         than borrowing one from the form"
    );

    let stored = common::stored_displayed(&reader, created).await;
    assert_eq!(
        stored, shown,
        "the signed body must name the ids the PROMPT displayed, in display order — not the \
         node's raw answer, and not a reordering of it"
    );
    assert_eq!(
        stored.len(),
        PROMPT_CAP,
        "the bound must actually have bitten, or this test passed for the wrong reason"
    );

    // THE FLAG BESIDE THE ROSTER. ADR-0075 (#671): the prompt was CUT, but the SEARCH was
    // whole — so the signed body must say `incomplete: false`. Truncation is shown on screen,
    // never signed (before ADR-0075 it was, on 92% of registrations, and the flag said
    // nothing). The raw-list bypass the old `true` assertion guarded against is still caught
    // above: a port forwarding the node's raw list would store all `PROMPT_CAP + 3` ids, and
    // `stored.len() == PROMPT_CAP` fails on it.
    assert!(
        !common::stored_incomplete(&reader, created).await,
        "{} candidates were cut from the prompt, but the search read every chart it matched: \
         truncation is not an incompleteness of the search (ADR-0075 decision 3)",
        raw.candidates.len() - PROMPT_CAP
    );
}

/// The positive half of ADR-0075's rule, and what stops `incomplete` becoming a constant
/// `false` now that a CUT prompt signs `false`: a search that could NOT read every chart it
/// matched must still sign `true`.
///
/// The node's one read gap today is a matched chart with no name ever asserted — it displays
/// as "(name unavailable)" and the list reports itself partial. A chart registered with only a
/// date of birth is exactly that, and a later search on the same date finds it.
#[tokio::test]
async fn a_search_that_could_not_read_a_chart_still_signs_incomplete() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;
    let live = LiveData::new(
        common::connect_for_live(&cs).await,
        sk,
        &common::identity(ORIGIN),
    );

    // A nameless chart: registered off a date-of-birth-only search, with no name asserted.
    let born = "1931-07-19";
    let dob_only = SearchQuery::new("", Some(born), &[]);
    let mut store = TokenStore::new();
    let t = store
        .record(dob_only.clone(), bound_for_prompt(&common::nothing_found()))
        .expect("a date of birth alone is a search");
    let a = store.take(t).expect("redeemable");
    live.register(a, None)
        .await
        .map_err(|(e, _)| e)
        .expect("a nameless chart registers");
    store.commit();

    let raw = live
        .search(&dob_only, TODAY)
        .await
        .expect("the step-3 search");
    assert!(
        raw.incomplete,
        "the fixture must make the node's search PARTIAL or this test proves nothing: {raw:?}"
    );

    let prompt = bound_for_prompt(&raw);
    let mut store = TokenStore::new();
    let token = store.record(dob_only, prompt).expect("a token");
    let attested = store.take(token).expect("redeemable");
    let created = live
        .register(attested, Some("Nameless Neighbour"))
        .await
        .map_err(|(e, _)| e)
        .expect("registration accepted");
    store.commit();

    assert!(
        common::stored_incomplete(&reader, created).await,
        "the search could not read a chart it matched, so the signed body MUST say it was \
         partial — that is what makes a 'no match' untrustworthy (ADR-0061, ADR-0075)"
    );
}
