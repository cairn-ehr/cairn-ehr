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

/// Seed one worker-state row and one waiting notice, then classify. `queued_ago` and
/// `progress_ago` are SQL intervals ("6 minutes") for the notice's `queued_at` and the worker's
/// last progress stamp (`last_drained_at`). A private helper, so each test below reads as the
/// one situation it pins.
async fn classify_seeded(
    c: &tokio_postgres::Client,
    queued_ago: &str,
    progress_ago: &str,
) -> CheckState {
    c.batch_execute(&format!(
        "TRUNCATE match_pending, match_worker_state; \
         INSERT INTO match_worker_state (matcher_version, last_drained_at) \
           VALUES ('v', now() - interval '{progress_ago}'); \
         INSERT INTO match_pending (patient_id, reason, queued_at) \
           VALUES (gen_random_uuid(), 'change', now() - interval '{queued_ago}');"
    ))
    .await
    .unwrap();
    classify(&read_snapshot(c).await.unwrap(), STALLED_AFTER_SECS)
}

/// Behind = a chart waits AND nothing has happened for longer than the threshold: no new notice
/// (6 min ago) and no worker progress (7 min ago).
#[tokio::test]
async fn a_stale_newest_notice_reads_stalled() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert!(matches!(
        classify_seeded(&c, "6 minutes", "7 minutes").await,
        CheckState::Stalled { waiting: 1, .. }
    ));
}

/// The case the newest-notice rule got wrong (ruling R13): a restore or a `reproject --rebuild`
/// queues every chart AT ONCE, so after five minutes even the newest notice is old — but a
/// healthy worker is still making progress through the backlog. That is "running", not "behind".
#[tokio::test]
async fn a_backlog_queued_at_once_reads_running_while_the_worker_progresses() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert_eq!(
        classify_seeded(&c, "10 minutes", "1 minute").await,
        CheckState::CatchingUp {
            waiting: 1,
            config_recheck: false
        }
    );
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
