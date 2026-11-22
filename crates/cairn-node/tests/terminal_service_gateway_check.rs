//! #909 slice B, Task 7 — `startup::check_gateway`, the DB half of `terminal-service
//! --gateway-listen`'s startup check: the service's own login with a recorded route passes; a
//! login missing any grant the gateway uses at run time, and a node with no route, are refused
//! naming the owner's step. A grant missing at run time would otherwise surface only as every
//! window's startup being refused, over and over, as if transient.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
mod common;

use cairn_node::db;
use cairn_node::terminal_service::startup::check_gateway;
use common::{cs, service_login, window_login};

/// A login with SELECT on both gateway tables but nothing else the gateway needs.
const READER: &str = "cairn_gw_check_probe";

async fn refusal(conn: &str) -> String {
    let c = db::connect(conn).await.unwrap();
    format!("{:#}", check_gateway(&c).await.expect_err("refused"))
}

#[tokio::test]
async fn check_gateway_passes_the_service_and_refuses_each_missing_piece() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let owner = db::connect_and_load_schema(&base).await.unwrap();
    let service = service_login(&owner, &base).await;
    let window = window_login(&owner, &base).await;
    let drop_reader = format!(
        "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{READER}') THEN \
           EXECUTE 'DROP OWNED BY {READER}'; EXECUTE 'DROP ROLE {READER}'; END IF; END $$"
    );
    owner
        .batch_execute(&format!(
            "{drop_reader}; CREATE ROLE {READER} LOGIN; \
             GRANT SELECT ON terminal_trust, terminal_gateway TO {READER}; \
             DELETE FROM terminal_gateway"
        ))
        .await
        .unwrap();

    // No route recorded yet: the owner's gateway-setup is named.
    let e = refusal(&service).await;
    assert!(e.contains("gateway-setup"), "{e}");

    owner
        .batch_execute(
            "INSERT INTO terminal_gateway (upstream, checked_at, checked_by) \
             VALUES ('127.0.0.1:5432', now(), 'check-test')",
        )
        .await
        .unwrap();
    let ok = check_gateway(&db::connect(&service).await.unwrap()).await;

    // The window's login cannot read the trust set at all.
    let no_select = refusal(&window).await;
    // This one reads both tables but cannot record an attempt (no EXECUTE on
    // cairn_terminal_attempt): the gateway would refuse every new terminal without a trace.
    let no_execute = refusal(&format!("{base} user={READER}")).await;

    owner
        .batch_execute(&format!("DELETE FROM terminal_gateway; {drop_reader}"))
        .await
        .unwrap();
    ok.unwrap();
    assert!(
        no_select.contains("provision-terminal-service-role"),
        "{no_select}"
    );
    assert!(
        no_execute.contains("provision-terminal-service-role"),
        "{no_execute}"
    );
    assert!(
        no_execute.contains("cairn_terminal_attempt"),
        "{no_execute}"
    );
}
