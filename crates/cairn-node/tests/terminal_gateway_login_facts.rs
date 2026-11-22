//! #909 slice B, Task 4 — `read_login_facts`: what the gateway learns about the login a startup
//! names. DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
mod common;

use cairn_node::db;
use cairn_node::terminal_service::gateway::admit::read_login_facts;
use common::cs;

#[tokio::test]
async fn login_facts_for_window_service_and_unknown_roles() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let owner = db::connect_and_load_schema(&base).await.unwrap();
    common::window_login(&owner, &base).await;
    common::service_login(&owner, &base).await;

    let w = read_login_facts(&owner, "cairn_window_probe")
        .await
        .unwrap();
    assert!(w.can_login && w.window_member && !w.service_member);
    assert_eq!(cairn_node::conn_posture::over_powered(&w.posture), None);

    // cairn_terminal_service inherits cairn_window (db/083), so the service is both.
    let s = read_login_facts(&owner, "cairn_terminal_service_probe")
        .await
        .unwrap();
    assert!(s.can_login && s.window_member && s.service_member);

    // A group role is not a login.
    let g = read_login_facts(&owner, "cairn_window").await.unwrap();
    assert!(!g.can_login);

    // Unknown, and an injection-shaped name, are both plain errors.
    assert!(read_login_facts(&owner, "no_such_role_909").await.is_err());
    assert!(read_login_facts(&owner, "x'; DROP TABLE event_log; --")
        .await
        .is_err());
}
