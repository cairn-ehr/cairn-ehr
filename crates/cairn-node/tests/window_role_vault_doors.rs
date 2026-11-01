//! #898 — the vault doors the window calls, run under the window's OWN login.
//!
//! Split out of `window_role_grants.rs` (which walks the rest of the window's day) to keep each
//! file under 500 lines; read that file's module doc for why these walks exist and what they
//! assert ("this call was not refused for want of a privilege"). The shared checks,
//! [`common::assert_not_denied`] and [`common::owner_and_window`], live in `tests/common`.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`. Passphrases are
//! minted at runtime.
mod common;

use cairn_node::db;
use cairn_node::vault::{self, NewMember, Purpose, Release};
use common::{assert_not_denied, cs, owner_and_window, setup};
use uuid::Uuid;

/// The vault the window talks to: the terminal lock's stored hash, a staff member's hint, the
/// duplicate-ID check, an enrolment, and the release of a key for one act.
#[tokio::test]
async fn the_windows_vault_doors_run_under_its_own_login() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, window) = owner_and_window(&base).await;
    setup(&owner, &[]).await;
    owner
        .batch_execute("TRUNCATE vault_attempt, vault_member, vault_terminal CASCADE")
        .await
        .unwrap();
    let su_pass = format!("su-{}", Uuid::now_v7());
    let member = |staff_id, role, passphrase| NewMember {
        staff_id,
        display_name: "Test Person",
        role,
        phone: None,
        email: Some("t@example.org"),
        passphrase,
        hint: Some("a hint"),
    };
    // The first superuser is bootstrapped by the owner's console verb, never by the window.
    vault::bootstrap_superuser(&owner, &member("boss", "superuser", &su_pass))
        .await
        .unwrap();

    let terminal = window
        .query_one("SELECT cairn_vault_terminal_phc()", &[])
        .await
        .map_err(anyhow::Error::from);
    assert_not_denied("terminal lock: cairn_vault_terminal_phc", &terminal);
    let hint = window
        .query_one("SELECT cairn_vault_hint($1)", &[&"boss"])
        .await
        .map_err(anyhow::Error::from);
    assert_not_denied("sign-in hint: cairn_vault_hint", &hint);
    let r = vault::staff_id_taken(&window, "jsmith").await;
    assert_not_denied("vault::staff_id_taken", &r);

    let r = vault::release_for_act(&window, "boss", &su_pass, Purpose::Enrol).await;
    assert_not_denied("vault::release_for_act", &r);
    let Release::Released { key: su, .. } = r.unwrap() else {
        panic!("the superuser's key is released for an enrolment")
    };
    let clin_pass = format!("clin-{}", Uuid::now_v7());
    let r = vault::enrol_member(
        &window,
        &member("jsmith", "clinician", &clin_pass),
        "boss",
        &su,
    )
    .await;
    assert_not_denied("vault::enrol_member", &r);
    r.expect("the enrolment is recorded");

    // Pure, but the window calls it: hashing the terminal passphrase touches no table.
    assert!(vault::terminal_phc(&clin_pass).is_ok());
}
