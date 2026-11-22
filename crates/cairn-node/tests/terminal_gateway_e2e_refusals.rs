//! #909 slice B, Task 7 — the gateway's refusals as the window meets them (plan Review Focus 1 and
//! 4), end to end through the shared rig (`common/gateway_rig.rs`).
//!
//! * RF1: once a terminal is revoked, the window's REOPEN is refused as "not admitted", a
//!   refusal naming the operator, never an outage the window would retry for ever.
//! * RF4: a window login that is over-powered is refused at startup with 28000 and the fixed
//!   text, even from an admitted terminal and with the right credentials.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
mod common;
#[path = "common/gateway_rig.rs"]
mod gateway_rig;

use std::time::{Duration, Instant};

use cairn_node::terminal_service::gateway::admit::REFUSED;
use gateway_rig::{failure, rig, select_one};

/// A window login that can also read the server's files (as Task 4's login-facts test makes).
const OVERPOWERED: &str = "cairn_gw_overpowered_probe";

#[tokio::test]
async fn after_a_revoke_the_windows_reopen_is_refused_as_not_admitted() {
    let Some(r) = rig().await else { return };
    let (t, kid) = r.admitted_terminal().await;
    assert_eq!(select_one(&r.window(&t).await).await, 1);
    r.revoke(&kid).await;
    // The gateway's 1 s tick has to see the revoke first. Until then a reopen may still pass, or
    // (if the sweep lands mid-startup) end as an outage; neither is the answer under test.
    let deadline = Instant::now() + Duration::from_secs(5);
    let (text, refused, sqlstate) = loop {
        let got = r
            .link(&t)
            .connect_with_task(&r.conn("cairn_window_probe"))
            .await;
        let last = got.err().map(failure);
        match last {
            Some(f) if f.1 => break f,
            other => {
                assert!(
                    Instant::now() < deadline,
                    "no not-admitted refusal 5 s after the revoke (last: {other:?})"
                );
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    };
    assert!(text.contains("not admitted"), "{text}");
    assert!(text.contains("operator"), "{text}");
    assert!(
        refused,
        "a refusal, not an outage the window retries: {text}"
    );
    assert_eq!(sqlstate, None);
    r.finish().await;
}

#[tokio::test]
async fn an_overpowered_window_login_is_refused_with_28000() {
    let Some(r) = rig().await else { return };
    r.owner
        .batch_execute(&format!(
            "DROP ROLE IF EXISTS {OVERPOWERED}; CREATE ROLE {OVERPOWERED} LOGIN; \
             GRANT cairn_window TO {OVERPOWERED}; GRANT pg_read_server_files TO {OVERPOWERED}"
        ))
        .await
        .unwrap();
    let (t, _) = r.admitted_terminal().await;
    drop(r.window(&t).await); // admitted: what follows is the startup verdict, not the TLS pin
    let reached = r.link(&t).connect_with_task(&r.conn(OVERPOWERED)).await;
    r.owner
        .batch_execute(&format!("DROP ROLE IF EXISTS {OVERPOWERED}"))
        .await
        .unwrap();
    let Err(err) = reached else {
        panic!("an over-powered login passed the gateway");
    };
    let (text, _, sqlstate) = failure(err);
    assert_eq!(sqlstate.as_deref(), Some("28000"), "{text}");
    assert!(text.contains(REFUSED), "{text}");
    assert!(
        !text.contains("pg_read_server_files"),
        "names no role: {text}"
    );
    r.finish().await;
}
