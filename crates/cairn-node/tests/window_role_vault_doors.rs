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

use cairn_event::keys::Secret32;
use cairn_event::{generate_key, sign_attestation, SigningKey};
use cairn_node::vault::{self, NewMember, Purpose, Release};
use cairn_node::{db, seal};
use common::{assert_not_denied, cs, owner_and_window, setup};
use tokio_postgres::types::ToSql;
use tokio_postgres::Client;
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

/// Release `staff`'s key through the library, as the window does.
async fn released(window: &Client, staff: &str, passphrase: &str, purpose: Purpose) -> SigningKey {
    let r = vault::release_for_act(window, staff, passphrase, purpose).await;
    assert_not_denied("vault::release_for_act", &r);
    match r.unwrap() {
        Release::Released { key, .. } => key,
        other => panic!("{staff}'s key is released, got {other:?}"),
    }
}

/// `sk`'s attestation over the address `sql` computes (run under the window, so the address
/// functions are walked too).
async fn signed(
    window: &Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
    sk: &SigningKey,
) -> Vec<u8> {
    let addr = window
        .query_one(sql, params)
        .await
        .map_err(anyhow::Error::from);
    assert_not_denied(sql, &addr);
    let addr: Vec<u8> = addr.unwrap().get(0);
    let kid = hex::encode(sk.verifying_key().to_bytes());
    sign_attestation(&addr, &kid, "approved", sk).unwrap()
}

/// Run one door under the window and require that it SUCCEEDED (its deep path ran), not merely
/// that it was not refused for want of a privilege.
async fn door(
    window: &Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Vec<tokio_postgres::Row> {
    let r = window.query(sql, params).await.map_err(anyhow::Error::from);
    assert_not_denied(sql, &r);
    r.unwrap_or_else(|e| panic!("{sql} must succeed under the window: {e:#}"))
}

fn sealed(sk: &SigningKey, passphrase: &str) -> Vec<u8> {
    let s = seal::seal(
        &Secret32::from_bytes(sk.to_bytes()),
        passphrase,
        &seal::generate_recovery_code(),
    )
    .unwrap();
    seal::to_cbor(&s)
}

/// Sign-in slice 2's doors (db/063), each DRIVEN to its deep path under the window's login: a grant,
/// an enroller's enrolment, the worklist, a confirmation, a re-key and its notice, a reseal.
#[tokio::test]
async fn the_windows_slice_2_vault_doors_run_under_its_own_login() {
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
    let pass = |who: &str| format!("{who}-{}", Uuid::now_v7());
    let (su_pass, j_pass, a_pass) = (pass("su"), pass("j"), pass("a"));
    let member = |staff_id, role, passphrase| NewMember {
        staff_id,
        display_name: "Test Person",
        role,
        phone: None,
        email: Some("t@example.org"),
        passphrase,
        hint: None,
    };
    vault::bootstrap_superuser(&owner, &member("boss", "superuser", &su_pass))
        .await
        .unwrap();
    let su = released(&window, "boss", &su_pass, Purpose::Enrol).await;
    vault::enrol_member(
        &window,
        &member("jsmith", "clinician", &j_pass),
        "boss",
        &su,
    )
    .await
    .unwrap();

    // A grant, signed over the state it changes.
    let prior: i64 = door(&window, "SELECT cairn_vault_grant_prior('jsmith')", &[]).await[0].get(0);
    let t = signed(
        &window,
        "SELECT cairn_vault_grant_address('jsmith', true, $1)",
        &[&prior],
        &su,
    )
    .await;
    door(
        &window,
        "SELECT cairn_vault_set_enroller('jsmith', true, 'boss', $1)",
        &[&t],
    )
    .await;
    assert!(
        door(&window, "SELECT cairn_vault_is_enroller('jsmith')", &[]).await[0].get::<_, bool>(0)
    );
    assert_eq!(
        door(&window, "SELECT * FROM cairn_vault_enrollers()", &[])
            .await
            .len(),
        1
    );

    // The enroller enrols; the worklist shows it; the superuser confirms it.
    let jk = released(&window, "jsmith", &j_pass, Purpose::Enrol).await;
    let a_kid = vault::enrol_member(&window, &member("anurse", "nurse", &a_pass), "jsmith", &jk)
        .await
        .unwrap();
    assert_eq!(
        door(&window, "SELECT * FROM cairn_vault_unconfirmed()", &[])
            .await
            .len(),
        1
    );
    let t = signed(
        &window,
        "SELECT cairn_vault_judge_address($1, true)",
        &[&a_kid],
        &su,
    )
    .await;
    door(
        &window,
        "SELECT cairn_vault_judge_key($1, true, 'boss', $2)",
        &[&a_kid, &t],
    )
    .await;

    // A re-key, and the notice the old passphrase now earns.
    let (new_sk, new_kid) = generate_key().unwrap();
    let t = signed(
        &window,
        "SELECT cairn_vault_rekey_address('anurse', 'nurse', $1)",
        &[&new_kid],
        &su,
    )
    .await;
    door(
        &window,
        "SELECT cairn_vault_rekey('anurse', 'nurse', NULL, NULL, NULL, $1, $2, NULL, 'boss', $3)",
        &[&new_kid, &sealed(&new_sk, &pass("a2")), &t],
    )
    .await;
    assert_eq!(
        door(
            &window,
            "SELECT * FROM cairn_vault_replaced_notice('anurse')",
            &[]
        )
        .await
        .len(),
        1
    );
    // The expiry warning's date (§5.5): no limit is set, so there is none.
    door(
        &window,
        "SELECT cairn_vault_unconfirmed_expires_at('anurse')",
        &[],
    )
    .await;

    // A reseal, proved by the member's own released key.
    let blob = sealed(&jk, &pass("j2"));
    let proof = signed(
        &window,
        "SELECT cairn_vault_reseal_address('jsmith', $1)",
        &[&blob],
        &jk,
    )
    .await;
    door(
        &window,
        "SELECT cairn_vault_reseal('jsmith', $1, NULL, $2)",
        &[&blob, &proof],
    )
    .await;
}
