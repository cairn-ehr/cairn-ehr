//! #898/#899: `provision-window-role` — the login it makes holds only a SCRAM verifier, a re-run
//! keeps it, and the verb refuses a cluster that trusts anyone (the local test cluster does).
mod common;
use cairn_node::window_access::provision_window_login;
use std::process::Command;

const ROLE: &str = "cairn_window_test_login";

async fn drop_role(db: &tokio_postgres::Client) {
    db.batch_execute(&format!(
        "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='{ROLE}') THEN DROP ROLE {ROLE}; END IF; END $$"
    ))
    .await
    .unwrap();
}

async fn verifier(db: &tokio_postgres::Client) -> Option<String> {
    db.query_one(
        "SELECT rolpassword FROM pg_authid WHERE rolname = $1",
        &[&ROLE],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn creates_scram_login_in_cairn_window_and_rerun_keeps_it() {
    let Some(cs) = common::cs() else {
        eprintln!("skipped: no CAIRN_TEST_PG");
        return;
    };
    let _guard = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    let db = cairn_node::db::connect_and_load_schema(&cs).await.unwrap();
    drop_role(&db).await;

    // A new role with no password is refused and creates nothing.
    assert!(provision_window_login(&db, ROLE, None).await.is_err());
    assert!(!cairn_node::window_access::role_exists(&db, ROLE)
        .await
        .unwrap());

    let pw = format!("{}-{}", "a long enough", "test phrase");
    provision_window_login(&db, ROLE, Some(&pw)).await.unwrap();
    let v1 = verifier(&db).await.unwrap();
    assert!(v1.starts_with("SCRAM-SHA-256$"));
    let member: bool = db
        .query_one("SELECT pg_has_role($1, 'cairn_window', 'MEMBER')", &[&ROLE])
        .await
        .unwrap()
        .get(0);
    assert!(member);

    // Re-run with no password: the verifier is kept (not cleared).
    provision_window_login(&db, ROLE, None).await.unwrap();
    assert_eq!(verifier(&db).await.unwrap(), v1);

    // A bad identifier is refused before any DDL.
    assert!(provision_window_login(&db, "bad;role", Some(&pw))
        .await
        .is_err());
    drop_role(&db).await;
}

#[tokio::test]
async fn the_verb_refuses_a_trusting_cluster_and_changes_nothing() {
    let Some(cs) = common::cs() else {
        eprintln!("skipped: no CAIRN_TEST_PG");
        return;
    };
    // Held while the binary runs: it replays/touches the same database.
    let db = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    drop_role(&db).await;
    let out = Command::new(env!("CARGO_BIN_EXE_cairn-node"))
        .args(["--conn", &cs, "provision-window-role", "--role", ROLE])
        .env(
            "CAIRN_WINDOW_DB_PASSWORD",
            format!("{}-{}", "a long enough", "test phrase"),
        )
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("trust"));
    // Refused before anything was mutated: the role was not made.
    assert!(!cairn_node::window_access::role_exists(&db, ROLE)
        .await
        .unwrap());
}
