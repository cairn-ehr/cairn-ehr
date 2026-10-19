//! #822: `cairn-node event <id>` — the check an unknown-outcome commit asks the operator to make.
//!
//! Every marked commit failure says "check whether it was recorded before retrying". Before
//! #822 the only way to check was raw psql against `event_log`. These pin the read the command
//! is built on: an event this node holds is found, with its type and chart; one it does not hold
//! is reported absent, not as an error.
//!
//! DB-gated on $CAIRN_TEST_PG, guard-before-connect.
mod common;

use cairn_node::db;
use cairn_node::event_lookup::recorded_event;
use common::cs;
use uuid::Uuid;

/// A registration this node holds is found, naming its type and the chart it is filed in.
#[tokio::test]
async fn an_event_this_node_holds_is_found_with_its_type_and_chart() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &[]).await;
    let patient = Uuid::now_v7();
    let event = common::submit_registration(&c, &sk, &kid, patient, 1).await;

    let found = recorded_event(&c, event)
        .await
        .unwrap()
        .expect("the registration is recorded");
    assert_eq!(found.event_type, "identity.registration.asserted");
    assert_eq!(found.chart, patient);
    assert!(!found.recorded_at.is_empty(), "says when it was recorded");
}

/// An event this node does not hold is absent — `Ok(None)`, the answer the operator needs to
/// run the command again — never an error.
#[tokio::test]
async fn an_event_this_node_does_not_hold_is_absent_not_an_error() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert!(recorded_event(&c, Uuid::now_v7()).await.unwrap().is_none());
}
