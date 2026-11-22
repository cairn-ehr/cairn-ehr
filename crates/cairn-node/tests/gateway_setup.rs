//! #909 slice B: `gateway_setup` records the gateway's route and prints the node's fingerprint,
//! and refuses (writing nothing) when it may not. Postgres is never reconfigured.
//!
//! The local test cluster uses `trust` auth, which `hba_verdict` rightly refuses, so the success
//! path is exercised through `record_gateway` (the write + report half `gateway_setup` calls once
//! every check has passed), and `gateway_setup` itself is shown to refuse here and write nothing.
mod common;
use cairn_node::terminal_service::gateway::upstream::{gateway_setup, parse, record_gateway};

const ROLE: &str = "cairn_gw_setup_probe";

async fn rows(db: &tokio_postgres::Client) -> Vec<(String, String)> {
    db.query("SELECT upstream, checked_by FROM terminal_gateway", &[])
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect()
}

#[tokio::test]
async fn record_writes_one_row_and_reports_the_fingerprint_and_restores() {
    let Some(cs) = common::cs() else {
        eprintln!("skipped: no CAIRN_TEST_PG");
        return;
    };
    let db = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    // All inside one transaction that is rolled back: the shared test DB is left as found.
    db.batch_execute("BEGIN").await.unwrap();
    let key: String = (0..32)
        .map(|i| format!("{:02x}", (i * 7 + 3) as u8))
        .collect();
    db.batch_execute(&format!(
        "DELETE FROM local_node; DELETE FROM terminal_gateway; \
         INSERT INTO local_node (node_id, signer_key_id) VALUES ('\\x00'::bytea, '{key}')"
    ))
    .await
    .unwrap();

    let up = parse("127.0.0.1:5432").unwrap();
    let report = record_gateway(&db, &up, &["warn one".to_string()])
        .await
        .unwrap();
    let again = record_gateway(&db, &up, &[]).await.unwrap();
    let after = rows(&db).await;
    db.batch_execute("ROLLBACK").await.unwrap();

    assert_eq!(after.len(), 1, "run twice, still one row");
    assert_eq!(after[0].0, "127.0.0.1:5432");
    assert!(report.starts_with("warn one\n"), "{report}");
    let last = report.lines().last().unwrap();
    let fp = last.strip_prefix("node fingerprint: ").expect(last);
    assert_eq!(fp.split('-').count(), 5, "{fp}");
    assert_eq!(again.lines().last().unwrap(), last);
}

#[tokio::test]
async fn a_trusting_cluster_and_a_non_superuser_are_refused_and_nothing_is_written() {
    let Some(cs) = common::cs() else {
        eprintln!("skipped: no CAIRN_TEST_PG");
        return;
    };
    let db = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    let before = rows(&db).await;
    let up = parse("127.0.0.1:5432").unwrap();

    // The test cluster trusts local logins: refused by the access-rules check.
    let e = gateway_setup(&db, &up)
        .await
        .expect_err("a trusting cluster must be refused");
    assert!(format!("{e:#}").contains("trust"), "{e:#}");
    assert_eq!(rows(&db).await, before);

    // A login with no power cannot read the access rules at all.
    db.batch_execute(&format!(
        "DROP ROLE IF EXISTS {ROLE}; CREATE ROLE {ROLE} NOLOGIN; GRANT {ROLE} TO CURRENT_USER;"
    ))
    .await
    .unwrap();
    db.batch_execute(&format!("SET ROLE {ROLE}")).await.unwrap();
    assert_eq!(
        db.query_one("SELECT current_user::text", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        ROLE
    );
    let denied = gateway_setup(&db, &up).await;
    db.batch_execute("RESET ROLE").await.unwrap();
    db.batch_execute(&format!("DROP ROLE IF EXISTS {ROLE}"))
        .await
        .unwrap();
    let msg = format!("{:#}", denied.unwrap_err());
    assert!(msg.contains("superuser"), "{msg}");
    assert_eq!(rows(&db).await, before);
}
