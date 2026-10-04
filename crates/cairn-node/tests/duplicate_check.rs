//! Repair path R4: the node reads db/056's status, and the CLI prints it.
mod common;
use cairn_node::db;
use cairn_node::duplicate_check::{
    chart_check_pending, classify, read_snapshot, CheckState, STALLED_AFTER_SECS,
};
use common::cs;
use std::process::Command;
use uuid::Uuid;

#[tokio::test]
async fn a_node_with_no_worker_reads_never_run_and_every_chart_pending() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE match_pending, match_worker_state")
        .await
        .unwrap();
    let s = read_snapshot(&c).await.unwrap();
    assert_eq!(
        classify(&s, STALLED_AFTER_SECS),
        CheckState::NeverRun { waiting: 0 }
    );
    assert!(chart_check_pending(&c, Uuid::now_v7()).await.unwrap());
}

#[tokio::test]
async fn a_stale_newest_notice_reads_stalled() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute(
        "TRUNCATE match_pending, match_worker_state; \
         INSERT INTO match_worker_state (matcher_version, last_drained_at) VALUES ('v', now()); \
         INSERT INTO match_pending (patient_id, reason, queued_at) \
           VALUES (gen_random_uuid(), 'change', now() - interval '6 minutes');",
    )
    .await
    .unwrap();
    let s = read_snapshot(&c).await.unwrap();
    assert!(matches!(
        classify(&s, STALLED_AFTER_SECS),
        CheckState::Stalled { waiting: 1, .. }
    ));
}

#[tokio::test]
async fn the_cli_prints_the_status_and_the_charts_line() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE match_pending, match_worker_state")
        .await
        .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_cairn-node"))
        .args([
            "--conn",
            &base,
            "duplicate-check",
            "--patient",
            &Uuid::now_v7().to_string(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("Duplicate check has never run on this node."),
        "{text}"
    );
    assert!(
        text.contains("This chart: duplicate check not yet run"),
        "{text}"
    );
}
