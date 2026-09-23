//! DB-gated: the three node reads the window makes beside the two funnel ports — where this
//! node's signing key stands (the launch probe, #654 option 2), the remedy-naming refusal a
//! registration meets before it takes its attestation (#665), and the DATABASE's date — plus
//! the one connection the funnel shares with the chart commands.
//!
//! Skips (green) without `$CAIRN_TEST_PG`; `db_gate_ran.rs` is what fails closed when nobody
//! declared that skip.
mod common;

use cairn_gui_data::port::DataError;
use cairn_gui_live::LiveData;
use cairn_node::actor_enrolment::ActorStanding;

/// A distinctive origin, for the same reason `attestation_through_the_port.rs` gives.
const ORIGIN: &str = "node-reads-origin";

/// A node whose signing key nothing enrolled: the window must be able to SAY so at launch and
/// must refuse a registration with the remedy, not with db/005's bare key id.
#[tokio::test]
async fn an_unenrolled_node_key_is_not_provisioned_and_names_the_remedy() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    // `setup` enrols ITS key; the port below signs with a DIFFERENT one nothing enrolled.
    let (_enrolled, _kid) = common::setup(&reader).await;
    let (unenrolled, _) = cairn_event::generate_key().expect("a fresh key");
    let live = LiveData::new(
        common::connect_for_live(&cs).await,
        unenrolled,
        &common::identity(ORIGIN),
    );

    assert_eq!(live.standing().await.unwrap(), ActorStanding::NeverEnrolled);
    let err = live.require_provisioned().await.unwrap_err();
    assert!(
        matches!(&err, DataError::NotProvisioned(t) if t.contains("enroll-device-actor")),
        "an unprovisioned node is a NodeState verdict carrying its remedy, got {err:?}"
    );
}

#[tokio::test]
async fn an_enrolled_node_key_is_provisioned() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;
    let live = LiveData::new(
        common::connect_for_live(&cs).await,
        sk,
        &common::identity(ORIGIN),
    );
    assert_eq!(live.standing().await.unwrap(), ActorStanding::Enrolled);
    live.require_provisioned()
        .await
        .expect("an enrolled key may write");
}

/// `today` is the DATABASE's date — the one `cairn-node`'s own CLI uses — never this machine's
/// wall clock. Compared with a date read on a second connection rather than with the local
/// clock, so the test says which clock it means.
#[tokio::test]
async fn today_is_the_databases_current_date() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;
    let expected: String = reader
        .query_one("SELECT current_date::text", &[])
        .await
        .unwrap()
        .get(0);
    let live = LiveData::new(
        common::connect_for_live(&cs).await,
        sk,
        &common::identity(ORIGIN),
    );
    assert_eq!(live.today().await.unwrap(), expected);
}

/// The chart commands and the funnel share ONE connection, so the window answers "which node
/// am I" once (see `LiveData::new`'s doc).
#[tokio::test]
async fn the_connection_is_shared_not_copied() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;
    let shared = std::sync::Arc::new(tokio::sync::Mutex::new(common::connect_for_live(&cs).await));
    let live = LiveData::sharing(shared.clone(), sk, &common::identity(ORIGIN));
    assert!(std::sync::Arc::ptr_eq(&shared, &live.connection()));
}
