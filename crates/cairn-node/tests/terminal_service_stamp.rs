//! #909 slice C1, task 5: a running terminal service notices that the node's schema stamp no
//! longer names its build (an upgrade replayed the schema), so it can exit for its supervisor.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` (this test
//! changes `node_schema`, shared by every DB test).
use cairn_node::db;
use cairn_node::terminal_service::run::stamp_still_ours;

#[tokio::test]
async fn a_changed_stamp_is_noticed() {
    let Ok(base) = std::env::var("CAIRN_TEST_PG") else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert!(
        stamp_still_ours(&c).await.is_ok(),
        "precondition: a freshly loaded schema carries this build's stamp"
    );

    // Another build replayed the schema: the stamp no longer names this one.
    c.batch_execute("UPDATE node_schema SET schema_hash = NULL")
        .await
        .unwrap();
    let err = stamp_still_ours(&c)
        .await
        .expect_err("a stamp without this build's hash must be refused");
    assert!(!err.0.is_empty(), "the refusal says why");

    // Restore: force a replay and reload, so later tests inherit a sound stamp.
    db::force_schema_replay(&c).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert!(stamp_still_ours(&c).await.is_ok(), "reloaded: ours again");
}
