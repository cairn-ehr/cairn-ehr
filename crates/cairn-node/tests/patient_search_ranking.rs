//! `search_patients` ranks a chart that matched MORE `db/046` passes above older charts that
//! matched fewer — the order the funnel's bounded step-3 prompt shows and a registration signs
//! (funnel UI slice 2c). Real Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via
//! `db::test_serial_guard` like every suite in this directory.
//!
//! Why a suite of its own rather than one more test in `patient_search.rs`: that file is
//! ~1700 lines and pins db/046's candidate SET; this one pins only the ORDER `search_patients`
//! imposes on it.
mod common;

use cairn_event::demographics::{dob_assertion_body, render_dob_twin};
use cairn_node::db;
use cairn_patient_search::SearchQuery;
use common::{chart_named, cs, setup, submit_signed, EventSpec};

/// The projections the charts below write beyond `common::setup`'s default core, cleared so a
/// previous run's charts cannot join the candidate set (#583's shape).
const EXTRA_TABLES: [&str; 2] = ["patient_name", "patient_registration"];

#[tokio::test]
async fn the_chart_sharing_name_and_birth_date_outranks_older_name_only_charts() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // Six OLDER charts sharing only the token "smith" — more than the prompt's cap of five, so
    // in plain id order the real duplicate would be cut from the prompt entirely.
    let mut older = Vec::new();
    for i in 0..6 {
        older.push(chart_named(&c, &sk, &kid, 10 * i, &format!("Smith Other{i}")).await);
    }
    // The duplicate: registered LAST (so it has the newest id), sharing the name AND the dob.
    let dup = chart_named(&c, &sk, &kid, 100, "John Smith").await;
    let dob = "1980-01-01";
    submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: dup,
            event_type: "demographic.field.asserted",
            schema_version: "demographic.field/1",
            payload: dob_assertion_body(dob, "day", None, "patient-stated"),
            plaintext_twin: Some(render_dob_twin(dob, "day", "patient-stated")),
            wall: 102,
        },
    )
    .await
    .expect("dob accepted");

    let query = SearchQuery::new("John Smith", Some(dob), &[]);
    let list = cairn_node::patient::search::search_patients(&c, &query, "2026-09-23")
        .await
        .expect("search succeeds");

    assert_eq!(list.candidates.len(), 7, "the SET is unchanged: {list:?}");
    assert_eq!(
        list.candidates[0].patient_id, dup,
        "the chart matching name AND dob must come first, not the oldest chart: {list:?}"
    );
    let rest: Vec<_> = list.candidates[1..].iter().map(|c| c.patient_id).collect();
    assert_eq!(rest, older, "single-pass ties keep id (chart-age) order");
}

/// Assert a day-precision date of birth for `patient` (file-local helper — deliberately not in
/// `tests/common`, which has a hand-registered helper inventory).
async fn assert_dob(
    c: &tokio_postgres::Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    patient: uuid::Uuid,
    dob: &str,
    wall: i64,
) {
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient,
            event_type: "demographic.field.asserted",
            schema_version: "demographic.field/1",
            payload: dob_assertion_body(dob, "day", None, "patient-stated"),
            plaintext_twin: Some(render_dob_twin(dob, "day", "patient-stated")),
            wall,
        },
    )
    .await
    .expect("dob accepted");
}

/// ADR-0075 / #671: the duplicate typed with a WRONG date of birth. It matches db/046's name
/// pass only, so passes alone tie it with every namesake; the two new keys must lift it —
/// both its name tokens match, and its stored DOB is the typed one with day and month swapped.
#[tokio::test]
async fn a_wrong_dob_duplicate_outranks_namesakes_by_tokens_and_near_miss() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // Six OLDER one-token namesakes — more than the cap, so chart-age order would bury the dup.
    let mut older = Vec::new();
    for i in 0..6 {
        older.push(chart_named(&c, &sk, &kid, 10 * i, &format!("Smith Other{i}")).await);
    }
    // An older full namesake with an unrelated DOB: two tokens, no near-miss.
    let namesake = chart_named(&c, &sk, &kid, 80, "John Smith").await;
    assert_dob(&c, &sk, &kid, namesake, "1950-06-15", 82).await;
    // The duplicate, newest: stored 1980-02-01, typed 1980-01-02 (day/month swapped).
    let dup = chart_named(&c, &sk, &kid, 100, "John Smith").await;
    assert_dob(&c, &sk, &kid, dup, "1980-02-01", 102).await;

    let query = SearchQuery::new("John Smith", Some("1980-01-02"), &[]);
    let list = cairn_node::patient::search::search_patients(&c, &query, "2026-09-26")
        .await
        .expect("search succeeds");

    let ids: Vec<_> = list.candidates.iter().map(|c| c.patient_id).collect();
    assert_eq!(ids.len(), 8, "the SET is unchanged: {list:?}");
    assert_eq!(
        ids[0], dup,
        "two tokens AND a DOB near-miss come first: {list:?}"
    );
    assert_eq!(ids[1], namesake, "two tokens beat one: {list:?}");
    assert_eq!(
        ids[2..].to_vec(),
        older,
        "one-token ties keep chart-age order"
    );
}
