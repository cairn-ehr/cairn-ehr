//! #909: `provision-terminal-service-role` and the `terminal-service` verb's startup refusal.
//! The service's login is a member of `cairn_terminal_service` and of nothing a window login
//! would be; an existing window login is never taken over as the service's.
mod common;
use cairn_node::window_access::{provision_login, provision_window_login, LoginGroup};
use std::process::Command;

const ROLE: &str = "cairn_tsvc_test_login";
const WINDOW_ROLE: &str = "cairn_tsvc_test_window";

async fn drop_roles(db: &tokio_postgres::Client) {
    for r in [ROLE, WINDOW_ROLE] {
        db.batch_execute(&format!(
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='{r}') THEN DROP ROLE {r}; END IF; END $$"
        ))
        .await
        .unwrap();
    }
}

async fn is_member(db: &tokio_postgres::Client, role: &str, group: &str) -> bool {
    db.query_one("SELECT pg_has_role($1, $2, 'MEMBER')", &[&role, &group])
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
async fn provisioned_service_login_is_a_member_of_cairn_terminal_service_only() {
    let Some(cs) = common::cs() else {
        eprintln!("skipped: no CAIRN_TEST_PG");
        return;
    };
    let _guard = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    let db = cairn_node::db::connect_and_load_schema(&cs).await.unwrap();
    drop_roles(&db).await;
    let pw = format!("{}-{}", "a long enough", "test phrase");
    provision_login(&db, ROLE, Some(&pw), LoginGroup::TerminalService)
        .await
        .unwrap();
    assert!(is_member(&db, ROLE, "cairn_terminal_service").await);
    // Its only DIRECT membership is the service group (cairn_window arrives through it).
    let direct: Vec<String> = db
        .query(
            "SELECT g.rolname::text FROM pg_auth_members m JOIN pg_roles g ON g.oid = m.roleid \
              WHERE m.member = $1::text::regrole",
            &[&ROLE],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(direct, vec!["cairn_terminal_service".to_string()]);
    // A re-run refreshes it (the posture check must accept its own group's reach).
    provision_login(&db, ROLE, Some(&pw), LoginGroup::TerminalService)
        .await
        .unwrap();
    drop_roles(&db).await;
}

#[tokio::test]
async fn an_existing_window_login_is_not_taken_over_as_the_service_login() {
    let Some(cs) = common::cs() else {
        eprintln!("skipped: no CAIRN_TEST_PG");
        return;
    };
    let _guard = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    let db = cairn_node::db::connect_and_load_schema(&cs).await.unwrap();
    drop_roles(&db).await;
    let pw = format!("{}-{}", "a long enough", "test phrase");
    provision_window_login(&db, WINDOW_ROLE, Some(&pw))
        .await
        .unwrap();
    let why = provision_login(&db, WINDOW_ROLE, Some(&pw), LoginGroup::TerminalService)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        why.contains("is not a terminal-service login")
            && why.contains("other than cairn_terminal_service"),
        "{why}"
    );
    assert!(!is_member(&db, WINDOW_ROLE, "cairn_terminal_service").await);
    drop_roles(&db).await;
}

/// The refusal comes before any connection: a remote host is never dialled.
#[test]
fn terminal_service_refuses_a_remote_database() {
    let out = Command::new(env!("CARGO_BIN_EXE_cairn-node"))
        .args([
            "--conn",
            "host=10.0.0.1 user=x dbname=x",
            "terminal-service",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    let want = cairn_node::conn_posture::require_local("host=10.0.0.1 user=x dbname=x")
        .unwrap_err()
        .to_string();
    assert!(err.contains(&want), "{err}");
}

#[tokio::test]
async fn the_provision_verb_refuses_a_trusting_cluster_and_changes_nothing() {
    let Some(cs) = common::cs() else {
        eprintln!("skipped: no CAIRN_TEST_PG");
        return;
    };
    let db = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    drop_roles(&db).await;
    let out = Command::new(env!("CARGO_BIN_EXE_cairn-node"))
        .args([
            "--conn",
            &cs,
            "provision-terminal-service-role",
            "--role",
            ROLE,
        ])
        .env(
            "CAIRN_TERMINAL_SERVICE_DB_PASSWORD",
            format!("{}-{}", "a long enough", "test phrase"),
        )
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("trust"));
    assert!(!cairn_node::window_access::role_exists(&db, ROLE)
        .await
        .unwrap());
}
