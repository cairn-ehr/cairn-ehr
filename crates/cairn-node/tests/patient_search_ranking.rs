//! `search_patients` ranks a chart that matched MORE `db/046` passes above older charts that
//! matched fewer — the order the funnel's bounded step-3 prompt shows and a registration signs
//! (funnel UI slice 2c) — and, within equal passes, by ADR-0075's keys: an identifier match,
//! then name tokens matched (exactly or as a typed prefix), then a DOB near-miss. Real Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via
//! `db::test_serial_guard` like every suite in this directory.
//!
//! Why a suite of its own rather than one more test in `patient_search.rs`: that file is
//! ~1700 lines and pins db/046's candidate SET; this one pins only the ORDER `search_patients`
//! imposes on it.
mod common;

use cairn_event::demographics::{
    dob_assertion_body, identifier_assertion_body, render_dob_twin, render_identifier_twin,
    IdentifierAssertion,
};
use cairn_node::{db, john_doe};
use cairn_patient_search::SearchQuery;
use common::{chart_named, cs, setup, submit_signed, EventSpec};

/// The projections the charts below write beyond `common::setup`'s default core, cleared so a
/// previous run's charts cannot join the candidate set (#583's shape). `chart_identity_state`
/// is the overlay `register_john_doe` writes (the callsign test).
const EXTRA_TABLES: [&str; 3] = [
    "patient_name",
    "patient_registration",
    "chart_identity_state",
];

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

/// Six OLDER charts sharing one name token with the query — one more than the prompt's cap of
/// five, so a chart that ranks BEHIND them all is cut from the prompt.
async fn six_older_namesakes(
    c: &tokio_postgres::Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    surname: &str,
) -> Vec<uuid::Uuid> {
    let mut older = Vec::new();
    for i in 0..6 {
        older.push(chart_named(c, sk, kid, 10 * i, &format!("{surname} Other{i}")).await);
    }
    older
}

/// Review of #678: the chart found ONLY by the identifier the clerk typed — "Peggy Jones" for a
/// "Margaret Smith" query, a nickname and a married surname. It shares no name token, so a
/// ranking by tokens alone put it below every one-token namesake and out of the five shown.
#[tokio::test]
async fn an_identifier_only_match_outranks_one_token_namesakes() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let older = six_older_namesakes(&c, &sk, &kid, "Smith").await;
    let dup = chart_named(&c, &sk, &kid, 100, "Peggy Jones").await;
    let mrn = IdentifierAssertion {
        value: "77001",
        system: "MRN",
        provenance: "document-verified",
        normalized: None,
        profile: None,
        use_: None,
    };
    submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: dup,
            event_type: "demographic.identifier.asserted",
            schema_version: "demographic.identifier/1",
            payload: identifier_assertion_body(&mrn),
            plaintext_twin: Some(render_identifier_twin(&mrn)),
            wall: 102,
        },
    )
    .await
    .expect("identifier accepted");

    let query = SearchQuery::new(
        "Margaret Smith",
        None,
        &[("MRN".to_string(), "77001".to_string())],
    );
    let list = cairn_node::patient::search::search_patients(&c, &query, "2026-09-26")
        .await
        .expect("search succeeds");

    let ids: Vec<_> = list.candidates.iter().map(|c| c.patient_id).collect();
    assert_eq!(ids.len(), 7, "the SET is unchanged: {list:?}");
    assert_eq!(ids[0], dup, "the identifier match comes first: {list:?}");
    assert_eq!(ids[1..].to_vec(), older, "namesakes keep chart-age order");
}

/// Review of #678: db/046 finds "Alexander Nguyen" when the clerk types "Alex" (#636's prefix
/// arm), so that match must count toward the order too — otherwise the duplicate a clerk found
/// by typing a short first name ties every other Nguyen and falls to chart age.
#[tokio::test]
async fn a_duplicate_found_by_a_typed_prefix_outranks_one_token_namesakes() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let older = six_older_namesakes(&c, &sk, &kid, "Nguyen").await;
    let dup = chart_named(&c, &sk, &kid, 100, "Alexander Nguyen").await;

    let query = SearchQuery::new("Alex Nguyen", None, &[]);
    let list = cairn_node::patient::search::search_patients(&c, &query, "2026-09-26")
        .await
        .expect("search succeeds");

    let ids: Vec<_> = list.candidates.iter().map(|c| c.patient_id).collect();
    assert_eq!(ids.len(), 7, "the SET is unchanged: {list:?}");
    assert_eq!(
        ids[0], dup,
        "two tokens (one by prefix) come first: {list:?}"
    );
    assert_eq!(ids[1..].to_vec(), older, "namesakes keep chart-age order");
}

/// db/046 never splits a §5.4 callsign ("unknown-n-ed-site1-…") into parts, so a clerk typing
/// "Ed" does not match every John Doe. The ranking must not either: a John Doe that joined the
/// set through its DOB is NOT a name match for "Ed Smith", so an older John Doe must rank
/// BEHIND a newer one-token namesake, not tie it and win on chart age.
#[tokio::test]
async fn a_callsign_is_not_split_into_name_tokens_for_ranking() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let (jd, _call, _ord) = john_doe::register_john_doe(
        &mut c,
        &sk,
        &kid,
        "n",
        "ED",
        "site1",
        "2026-09-26",
        "unconscious ED arrival, no ID",
    )
    .await
    .expect("john doe registration accepted by the floor");
    let dob = "1980-01-01";
    assert_dob(&c, &sk, &kid, jd, dob, 50).await;
    // Newer, and both of its tokens are plain: "smith" matches, "other" does not.
    let namesake = chart_named(&c, &sk, &kid, 100, "Smith Other").await;

    let query = SearchQuery::new("Ed Smith", Some(dob), &[]);
    let list = cairn_node::patient::search::search_patients(&c, &query, "2026-09-26")
        .await
        .expect("search succeeds");

    let ids: Vec<_> = list.candidates.iter().map(|c| c.patient_id).collect();
    assert_eq!(
        ids,
        vec![namesake, jd],
        "a callsign's parts are not name tokens: {list:?}"
    );
}
