//! #909 Task 3: `register_patient_in_tx` runs every precondition, tick and submit on a
//! transaction the CALLER owns and never commits it. The terminal service needs this so one
//! transaction can claim a request, register, and settle it. Real Postgres, gated on
//! `$CAIRN_TEST_PG`.
mod common;

use cairn_node::db;
use cairn_node::patient::register::register_patient_returning_event;
use cairn_node::patient::register_tx::register_patient_in_tx;
use cairn_patient_search::{CandidateList, SearchQuery};
use common::{cs, setup};
use tokio_postgres::Client;
use uuid::Uuid;

const EXTRA_TABLES: [&str; 1] = ["patient_registration"];

fn empty_list() -> CandidateList {
    CandidateList {
        people: vec![],
        incomplete: false,
        incomplete_reason: None,
    }
}

/// Events on a chart, as `(event_type, contributors-as-text)` in log order.
async fn events_of(c: &Client, p: Uuid) -> Vec<(String, String)> {
    c.query(
        "SELECT event_type, COALESCE(contributors::text, '') FROM event_log \
         WHERE patient_id::text = $1 ORDER BY event_type",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1)))
    .collect()
}

async fn registrations(c: &Client, p: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM patient_registration WHERE patient_id::text = $1",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn a_rolled_back_registration_leaves_no_chart_and_no_event() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;
    let query = SearchQuery::new("smith", Some("1980-01-01"), &[]);

    let hlc_before = hlc_state(&c).await;
    let tx = c.transaction().await.unwrap();
    let done = register_patient_in_tx(&tx, &sk, &kid, "n", Some("smith"), &query, &empty_list())
        .await
        .unwrap();
    drop(tx); // never committed: the drop rolls it back.

    assert!(events_of(&c, done.value).await.is_empty());
    assert_eq!(registrations(&c, done.value).await, 0);
    assert_eq!(hlc_state(&c).await, hlc_before, "the ticks rolled back too");
}

#[tokio::test]
async fn register_in_tx_then_commit_equals_register_patient() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;
    let query = SearchQuery::new("smith", Some("1980-01-01"), &[("mrn".into(), "A1".into())]);

    let tx = c.transaction().await.unwrap();
    let a = register_patient_in_tx(&tx, &sk, &kid, "n", Some("smith"), &query, &empty_list())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let b = register_patient_returning_event(
        &mut c,
        &sk,
        &kid,
        "n",
        Some("smith"),
        &query,
        &empty_list(),
    )
    .await
    .unwrap();

    assert_ne!(a.value, b.value);
    assert_eq!(events_of(&c, a.value).await.len(), 4);
    assert_eq!(events_of(&c, a.value).await, events_of(&c, b.value).await);
}

#[tokio::test]
async fn a_savepoint_rollback_leaves_the_outer_transaction_usable() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;
    let query = SearchQuery::new("smith", None, &[]);
    let hlc_before = hlc_state(&c).await;

    let mut outer = c.transaction().await.unwrap();
    let sp = outer.savepoint("reg").await.unwrap();
    let done = register_patient_in_tx(&sp, &sk, &kid, "n", Some("smith"), &query, &empty_list())
        .await
        .unwrap();
    sp.rollback().await.unwrap();
    // The outer transaction still works, and can even register again.
    outer.query_one("SELECT 1", &[]).await.unwrap();
    let kept = register_patient_in_tx(&outer, &sk, &kid, "n", Some("smith"), &query, &empty_list())
        .await
        .unwrap();
    outer.commit().await.unwrap();

    assert!(events_of(&c, done.value).await.is_empty());
    assert_eq!(registrations(&c, done.value).await, 0);
    assert_eq!(registrations(&c, kept.value).await, 1);
    // `kept`'s ticks survive; the rolled-back savepoint's did not add extra ones.
    assert!(hlc_state(&c).await > hlc_before);
}

#[tokio::test]
async fn a_refused_registration_on_a_savepoint_leaves_the_outer_transaction_usable() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;
    // A NUL in the name is accepted by the builders but refused by Postgres (jsonb cannot hold
    // it) on the SECOND submit, i.e. after the registration event was already accepted.
    let bad_name = "smi\u{0}th";
    let query = SearchQuery::new(bad_name, None, &[]);
    let good = SearchQuery::new("smith", None, &[]);

    let mut outer = c.transaction().await.unwrap();
    let sp = outer.savepoint("reg").await.unwrap();
    let refused =
        register_patient_in_tx(&sp, &sk, &kid, "n", Some(bad_name), &query, &empty_list()).await;
    assert!(refused.is_err(), "the floor must refuse the NUL name");
    // The savepoint is now aborted (25P02): it must be rolled back before any further use.
    sp.rollback().await.unwrap();
    outer.query_one("SELECT 1", &[]).await.unwrap();
    let kept = register_patient_in_tx(&outer, &sk, &kid, "n", Some("smith"), &good, &empty_list())
        .await
        .unwrap();
    outer.commit().await.unwrap();

    let total: i64 = c
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(total, 2, "only the good chart's registration + name remain");
    assert_eq!(registrations(&c, kept.value).await, 1);
}

/// The node's HLC high-water mark, as `(wall, counter)`.
async fn hlc_state(c: &Client) -> (i64, i32) {
    let r = c
        .query_one("SELECT hlc_wall, hlc_counter FROM hlc_state", &[])
        .await
        .unwrap();
    (r.get(0), r.get(1))
}
