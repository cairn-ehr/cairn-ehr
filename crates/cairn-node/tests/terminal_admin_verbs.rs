//! #909 slice B, Task 8 — the owner's console verbs over the terminal trust set.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`. Pending rows are made
//! through the real `cairn_terminal_attempt` door with keys computed at runtime (house rule 6);
//! each test starts from an empty `terminal_trust`, as `terminal_gateway_doors.rs` does.
mod common;

use cairn_event::short_fingerprint;
use cairn_node::db;
use cairn_node::terminal_service::terminals::{admit, list, render, revoke};
use common::cs;
use tokio_postgres::Client;

/// Connected owner client with an empty trust set; the guard keeps other suites out.
async fn rig() -> Option<(Client, Client)> {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = db::test_serial_guard(&base).await.unwrap();
    let owner = db::connect(&base).await.unwrap();
    owner
        .batch_execute("TRUNCATE terminal_trust")
        .await
        .unwrap();
    Some((guard, owner))
}

/// Make a pending terminal; returns its stored fingerprint.
async fn pending(owner: &Client, variant: u8) -> String {
    let bytes: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(variant));
    let key = hex::encode(bytes);
    owner
        .execute("SELECT cairn_terminal_attempt($1)", &[&key])
        .await
        .unwrap();
    short_fingerprint(&key).unwrap()
}

#[tokio::test]
async fn admit_then_list_shows_active_with_label_and_revoke_shows_revoked() {
    let Some((_g, owner)) = rig().await else {
        return;
    };
    let fp = pending(&owner, 1).await;

    // Lower-case, no dashes: the operator's typing, not the stored form.
    let typed = fp.replace('-', "").to_lowercase();
    let said = admit(&owner, &typed, "front desk").await.unwrap();
    assert!(said.contains(&fp) && said.contains("front desk"), "{said}");
    let rows = list(&owner).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].status.as_str(), rows[0].label.as_deref()),
        ("active", Some("front desk"))
    );
    assert!(render(&rows).contains("active"));

    let said = revoke(&owner, &fp).await.unwrap();
    assert!(said.contains("2 s") && said.contains("cancelled"), "{said}");
    assert_eq!(list(&owner).await.unwrap()[0].status, "revoked");
}

/// What the operator sees: the error's own Display (main prints it), not a downcast.
fn shown(e: &anyhow::Error) -> String {
    e.to_string()
}

#[tokio::test]
async fn refusals_carry_the_doors_own_text() {
    let Some((_g, owner)) = rig().await else {
        return;
    };
    let fp = pending(&owner, 2).await;

    let e = admit(&owner, "0000-0000-0000-0000-0000", "x")
        .await
        .unwrap_err();
    assert!(shown(&e).contains("no terminal has the fingerprint"), "{e}");
    let e = revoke(&owner, "0000-0000-0000-0000-0000")
        .await
        .unwrap_err();
    assert!(shown(&e).contains("no terminal has the fingerprint"), "{e}");

    let e = admit(&owner, &fp, "  ").await.unwrap_err();
    assert!(shown(&e).contains("needs a label"), "{e}");

    let e = revoke(&owner, "nonsense").await.unwrap_err();
    assert!(shown(&e).contains("20 hex digits"), "{e}");
}

#[tokio::test]
async fn revoke_is_idempotent_and_only_a_pending_terminal_can_be_admitted() {
    let Some((_g, owner)) = rig().await else {
        return;
    };
    let fp = pending(&owner, 3).await;
    admit(&owner, &fp, "a").await.unwrap();

    let e = admit(&owner, &fp, "b").await.unwrap_err(); // active
    assert!(shown(&e).contains("only a pending one can"), "{e}");

    revoke(&owner, &fp).await.unwrap();
    revoke(&owner, &fp).await.unwrap(); // already revoked: a no-op, not an error
    let e = admit(&owner, &fp, "b").await.unwrap_err(); // revoked
    assert!(shown(&e).contains("only a pending one can"), "{e}");
}

#[tokio::test]
async fn list_is_newest_seen_first() {
    let Some((_g, owner)) = rig().await else {
        return;
    };
    let first = pending(&owner, 4).await;
    let second = pending(&owner, 5).await;
    let order = |rows: &[cairn_node::terminal_service::terminals::TerminalRow]| {
        rows.iter()
            .map(|r| r.fingerprint.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        order(&list(&owner).await.unwrap()),
        vec![second.clone(), first.clone()]
    );

    // The older one is seen again: it is now the newest.
    let key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(4));
    owner
        .execute("SELECT cairn_terminal_attempt($1)", &[&hex::encode(key)])
        .await
        .unwrap();
    assert_eq!(order(&list(&owner).await.unwrap()), vec![first, second]);
}

#[tokio::test]
async fn list_failure_reads_as_the_database_message() {
    let Some((_g, owner)) = rig().await else {
        return;
    };
    // A table this login cannot read: SET ROLE to a role without SELECT (42501).
    owner
        .batch_execute("CREATE ROLE t8_noread NOLOGIN")
        .await
        .ok();
    owner.batch_execute("SET ROLE t8_noread").await.unwrap();
    let r = list(&owner).await;
    owner.batch_execute("RESET ROLE").await.unwrap();
    let e = r.unwrap_err();
    assert!(shown(&e).contains("permission denied"), "{e}");
}
