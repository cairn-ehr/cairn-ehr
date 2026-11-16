//! DB-gated, #909: the window registers through the node's terminal service.
//!
//! The window holds no node key, so `PatientRegistration::register` writes a request to db/083's
//! queue and waits for the service to settle it (`cairn_node::patient::register_request`). These
//! tests drive the port against a real queue, with and without a service running, and pin what
//! the clerk is told in each case:
//!
//! * the service runs: the chart is made, and the port returns its id;
//! * no service: the request expires, nothing is registered, and the clerk is told the terminal
//!   service did not answer (a `NotProvisioned`: only an operator can mend it);
//! * a chart appeared since the clerk's search: `Stale`, and the search goes back on the form;
//! * the window's connection is lost while it waits: `OutcomeUnknown` (#815), because the
//!   service may still register the request, and the search is spent.
mod common;

use cairn_gui_data::port::{DataError, PatientRegistration};
use cairn_gui_funnel::{bound_for_prompt, AttestedSearch, Restored, TokenStore};
use cairn_gui_live::LiveData;
use cairn_node::patient::register_request::SERVICE_NOT_RUNNING;
use cairn_patient_search::SearchQuery;
use std::time::Duration;
use tokio_postgres::Client;

const ORIGIN: &str = "gui-live-origin";

/// An attested search for `query` that displayed nobody (the funnel's common case).
fn attest(store: &mut TokenStore, query: SearchQuery) -> AttestedSearch {
    let token = store
        .record(query, bound_for_prompt(&common::nothing_found()))
        .expect("a non-empty query mints a token");
    store.take(token).expect("the token is redeemable")
}

async fn chart_count(c: &Client) -> i64 {
    c.query_one("SELECT count(*) FROM patient_chart", &[])
        .await
        .expect("count the charts")
        .get(0)
}

/// The status of the one request in the queue (each test makes exactly one).
async fn only_request_status(c: &Client) -> String {
    c.query_one("SELECT status FROM registration_request", &[])
        .await
        .expect("exactly one request")
        .get(0)
}

#[tokio::test]
async fn registers_when_the_service_runs() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, kid) = common::setup(&reader).await;
    let service = common::serve(&reader, &cs, sk, ORIGIN).await;
    let live = LiveData::new(
        common::connect_for_live(&cs).await,
        &common::identity(ORIGIN, &kid),
    );

    let mut store = TokenStore::new();
    let attested = attest(
        &mut store,
        SearchQuery::new("Queue Ok", Some("1970-02-03"), &[]),
    );
    let chart = live
        .register(attested, Some("Queue Ok"))
        .await
        .map_err(|(e, _)| e)
        .expect("the service registers the request");
    store.commit();
    service.stop().await;

    assert_eq!(chart_count(&reader).await, 1);
    let made: String = reader
        .query_one("SELECT patient_id::text FROM patient_chart", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        made,
        chart.to_string(),
        "the port returns the chart it made"
    );
    assert_eq!(only_request_status(&reader).await, "registered");
}

/// With no service, the request waits out its deadline and is expired: nothing goes in, and the
/// clerk reads what to do (tell the operator), not a retry or a refusal of the form.
#[tokio::test]
async fn says_service_not_running_and_registers_nothing_when_it_does_not() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (_sk, kid) = common::setup(&reader).await;
    let live = LiveData::new(
        common::connect_for_live(&cs).await,
        &common::identity(ORIGIN, &kid),
    );

    let mut store = TokenStore::new();
    let attested = attest(
        &mut store,
        SearchQuery::new("Queue Unserved", Some("1970-02-03"), &[]),
    );
    let (err, returned) = live
        .register(attested, Some("Queue Unserved"))
        .await
        .expect_err("no service, no registration");

    assert_eq!(err, DataError::NotProvisioned(SERVICE_NOT_RUNNING.into()));
    assert_eq!(store.restore(returned), Restored::Kept);
    assert_eq!(chart_count(&reader).await, 0);
    assert_eq!(
        only_request_status(&reader).await,
        "expired",
        "the request is expired, so a service started later can never register it"
    );
}

/// A chart the clerk was never shown appeared after their search: the service refuses with its
/// new-match words, the port calls that `Stale` (search again), and the search goes back.
#[tokio::test]
async fn a_new_match_is_stale_and_keeps_the_attestation() {
    let Some(cs) = common::cs() else { return };
    let (mut reader, _guard) = common::connect(&cs).await;
    let (sk, kid) = common::setup(&reader).await;

    // The clerk's search displayed nobody; then the same person is registered elsewhere.
    let typed = "Queue Newmatch";
    let query = SearchQuery::new(typed, None, &[]);
    let mut store = TokenStore::new();
    let attested = attest(&mut store, query.clone());
    cairn_node::patient::register::register_patient(
        &mut reader,
        &sk,
        &kid,
        ORIGIN,
        Some(typed),
        &query,
        &common::nothing_found(),
    )
    .await
    .expect("the chart that appears after the search");

    let service = common::serve(&reader, &cs, sk, ORIGIN).await;
    let live = LiveData::new(
        common::connect_for_live(&cs).await,
        &common::identity(ORIGIN, &kid),
    );
    let (err, returned) = live
        .register(attested, Some(typed))
        .await
        .expect_err("a chart the clerk never saw refuses the registration");
    service.stop().await;

    assert_eq!(
        err,
        DataError::Stale(cairn_node::terminal_service::registration::NEW_MATCH_REFUSAL.into())
    );
    assert_eq!(store.restore(returned), Restored::Kept);
    assert_eq!(
        chart_count(&reader).await,
        1,
        "only the chart that appeared"
    );
}

/// REVIEW FOCUS 1 (#909). The window's connection goes while it waits on a submitted request. The
/// service may still register it (its deadline has not passed), so the chart MAY exist: the port
/// must say `OutcomeUnknown` and the search must be spent, never `Unavailable` (a retry that
/// mints a second chart) or `NotProvisioned`.
///
/// No service runs, so the request is certainly still pending when the cut lands: the window is
/// between polls. The connection is a fixed one (`LiveData::new`), so the poll cannot reopen it.
#[tokio::test]
async fn a_lost_connection_after_submit_is_outcome_unknown() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (_sk, kid) = common::setup(&reader).await;
    let victim = common::connect_for_live(&cs).await;
    let pid: i32 = victim
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let live = LiveData::new(victim, &common::identity(ORIGIN, &kid));

    let mut store = TokenStore::new();
    let token = store
        .record(
            SearchQuery::new("Queue Lost", Some("1970-02-03"), &[]),
            bound_for_prompt(&common::nothing_found()),
        )
        .expect("a token");
    let attested = store.take(token).expect("redeemable");

    // Cut the window's backend as soon as its request is in the queue.
    let cut = async {
        for _ in 0..250 {
            let n: i64 = reader
                .query_one("SELECT count(*) FROM registration_request", &[])
                .await
                .unwrap()
                .get(0);
            if n == 1 {
                // Inside the window's first `POLL` sleep: after submit answered, before a poll.
                tokio::time::sleep(Duration::from_millis(50)).await;
                reader
                    .execute("SELECT pg_terminate_backend($1)", &[&pid])
                    .await
                    .unwrap();
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the window never submitted its request");
    };
    let (outcome, ()) = tokio::join!(live.register(attested, Some("Queue Lost")), cut);

    match store.settle(outcome) {
        Err((DataError::OutcomeUnknown(text), Restored::Consumed)) => assert!(
            text.contains("may yet register it"),
            "the clerk is told the request went to the service: {text}"
        ),
        Err((error, restored)) => panic!(
            "a request lost in flight MAY be registered, so it must arrive as OutcomeUnknown and \
             spend the search; it arrived as {error:?}, {restored:?}"
        ),
        Ok(id) => panic!("no service ran, yet registration returned {id}"),
    }
    assert!(
        store.take(token).is_err(),
        "the spent search licenses nothing"
    );
    assert_eq!(
        only_request_status(&reader).await,
        "pending",
        "the request is still pending: a service could register it until its deadline"
    );
}
