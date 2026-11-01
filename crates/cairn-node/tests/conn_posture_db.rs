//! #898 — no signing key is released over a connection with more power than a window may have.
//!
//! # Why this file exists
//!
//! The vault (db/061) counts every guess and checks every purpose, but those protections hold
//! only against a connection that cannot read the sealed keys or write around the floor. The
//! schema owner, a superuser, a member of the owner role, or the daemon's own `cairn_node` login
//! can do one of those, so `cairn_node::vault::release_for_act` refuses over any of them BEFORE
//! the passphrase is sent (`Release::OverPowered`). The pure judgement is unit-tested in
//! `src/conn_posture_tests.rs`; this file pins the FACTS as the database reports them for real
//! logins, and the three review-focus promises: a member of the owner role is caught (focus 3),
//! a refused guess is never counted (focus 4), and a fixed login is judged again on the same
//! connection, never from a cached verdict (focus 1).
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`. The throwaway logins
//! are cluster-wide, so each is dropped at the end and any leftover of a crashed run is dropped
//! before it is made again. Passphrases are built at runtime.
mod common;

use cairn_node::conn_posture::{over_powered, read_posture, PostureFacts};
use cairn_node::db;
use cairn_node::vault::{bootstrap_superuser, release_for_act, NewMember, Purpose, Release};
use common::{cs, window_login};
use tokio_postgres::Client;

/// A login granted the window's role AND `cairn_node`, the daemon's peer-door role.
const NODE_PROBE: &str = "cairn_window_probe_node";
/// A login granted the window's role AND membership of the schema owner's role (focus 3).
const OWNER_MEMBER_PROBE: &str = "cairn_window_probe_owner_member";

/// Serialised, freshly-loaded owner connection with an empty vault.
async fn fresh(base: &str) -> (Client, Client) {
    let guard = db::test_serial_guard(base).await.unwrap();
    let owner = db::connect_and_load_schema(base).await.unwrap();
    owner
        .batch_execute(
            "TRUNCATE vault_attempt, vault_member, vault_terminal, event_log, actor_event, \
             patient_chart, node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
        )
        .await
        .unwrap();
    (guard, owner)
}

/// Make the throwaway LOGIN `name` (granted `cairn_window`, plus whatever `extra` grants), first
/// dropping a leftover of a crashed run; return its connection string. `name` is one of this
/// file's constants, never input. No password: the test clusters use `trust`.
async fn throwaway_login(owner: &Client, base: &str, name: &str, extra: &str) -> String {
    owner
        .batch_execute(&format!(
            "DROP ROLE IF EXISTS {name}; CREATE ROLE {name} LOGIN; GRANT cairn_window TO {name}; \
             {extra}"
        ))
        .await
        .expect("the throwaway login");
    format!("{base} user={name}")
}

/// Undo [`throwaway_login`]: revoke what it was given, then drop it.
async fn drop_login(owner: &Client, name: &str) {
    owner
        .batch_execute(&format!(
            "REVOKE cairn_window FROM {name}; DROP ROLE {name};"
        ))
        .await
        .expect("dropping the throwaway login");
}

/// The sequences' view of `staff_id`'s guesses: (failures since the last success, epoch-ms of
/// the last checked guess). Both live in sequences, which no rollback undoes (db/061).
async fn attempts(owner: &Client, staff_id: &str) -> (i64, Option<i64>) {
    let row = owner
        .query_one(
            "SELECT failures, last_attempt_ms FROM cairn_vault_attempt_state($1)",
            &[&staff_id],
        )
        .await
        .unwrap();
    (row.get(0), row.get(1))
}

fn pass(word: &str) -> String {
    [word, "staple", "battery"].join("-")
}

#[tokio::test]
async fn the_owner_connection_is_over_powered() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, owner) = fresh(&base).await;
    let facts = read_posture(&owner).await.unwrap();
    assert!(facts.owns_cairn_table, "{facts:?}");
    assert!(over_powered(&facts).is_some(), "{facts:?}");
}

#[tokio::test]
async fn the_window_login_has_none_of_the_powers() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, owner) = fresh(&base).await;
    let window = db::connect(&window_login(&owner, &base).await)
        .await
        .unwrap();
    let facts = read_posture(&window).await.unwrap();
    assert_eq!(
        facts,
        PostureFacts {
            superuser: false,
            owns_cairn_table: false,
            reads_vault: false,
            inserts_event_log: false,
            runs_peer_door: false,
        }
    );
    assert_eq!(over_powered(&facts), None);
}

/// The daemon's own role can admit a peer's events: a window running on it is over-powered.
#[tokio::test]
async fn a_login_granted_cairn_node_is_over_powered() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, owner) = fresh(&base).await;
    let login = throwaway_login(
        &owner,
        &base,
        NODE_PROBE,
        &format!("GRANT cairn_node TO {NODE_PROBE};"),
    )
    .await;
    let conn = db::connect(&login).await.unwrap();
    let facts = read_posture(&conn).await.unwrap();
    drop(conn);
    owner
        .batch_execute(&format!("REVOKE cairn_node FROM {NODE_PROBE};"))
        .await
        .unwrap();
    drop_login(&owner, NODE_PROBE).await;
    assert!(facts.runs_peer_door, "{facts:?}");
    assert!(over_powered(&facts).is_some(), "{facts:?}");
}

/// Focus 3 and focus 1 on one login. A MEMBER of the owner role is not itself a superuser and
/// owns nothing by `relowner`, so only `pg_has_role(…, 'USAGE')` catches it. Then the operator
/// revokes the membership while the window's connection stays open (#807), and the SAME client's
/// next release is judged again: it reaches the vault (no one is enrolled, so `Unknown`).
#[tokio::test]
async fn a_member_of_the_owner_role_is_refused_until_the_membership_is_revoked() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, owner) = fresh(&base).await;
    // `current_user` on the owner connection is the role that owns every `public` table.
    let grant_owner = format!(
        "DO $$ BEGIN EXECUTE format('GRANT %I TO {OWNER_MEMBER_PROBE}', current_user); END $$;"
    );
    let login = throwaway_login(&owner, &base, OWNER_MEMBER_PROBE, &grant_owner).await;
    let conn = db::connect(&login).await.unwrap();

    let facts = read_posture(&conn).await.unwrap();
    assert!(!facts.superuser, "membership must not make it rolsuper");
    assert!(facts.owns_cairn_table, "{facts:?}");
    let first = release_for_act(&conn, "nobody", &pass("x"), Purpose::Clinical)
        .await
        .unwrap();
    assert!(matches!(first, Release::OverPowered { .. }), "{first:?}");

    owner
        .batch_execute(&format!(
            "DO $$ BEGIN EXECUTE format('REVOKE %I FROM {OWNER_MEMBER_PROBE}', current_user); END $$;"
        ))
        .await
        .unwrap();
    let second = release_for_act(&conn, "nobody", &pass("x"), Purpose::Clinical)
        .await
        .unwrap();
    drop(conn);
    drop_login(&owner, OWNER_MEMBER_PROBE).await;
    assert!(matches!(second, Release::Unknown), "{second:?}");
}

/// Focus 4: the refusal comes before the vault is asked, so a WRONG passphrase over the owner's
/// connection leaves the attempt sequences exactly as they were. The same wrong guess over the
/// window's login IS counted, which proves the measurement can see a guess at all.
#[tokio::test]
async fn a_wrong_passphrase_over_an_over_powered_connection_is_never_counted() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, owner) = fresh(&base).await;
    let right = pass("boss");
    bootstrap_superuser(
        &owner,
        &NewMember {
            staff_id: "boss",
            display_name: "Test Person",
            role: "superuser",
            phone: None,
            email: Some("t@example.org"),
            passphrase: &right,
            hint: None,
        },
    )
    .await
    .unwrap();
    let before = attempts(&owner, "boss").await;

    let refused = release_for_act(&owner, "boss", &pass("wrong"), Purpose::Enrol)
        .await
        .unwrap();
    assert!(
        matches!(refused, Release::OverPowered { .. }),
        "{refused:?}"
    );
    assert_eq!(
        attempts(&owner, "boss").await,
        before,
        "a refused guess was counted"
    );

    let window = db::connect(&window_login(&owner, &base).await)
        .await
        .unwrap();
    let counted = release_for_act(&window, "boss", &pass("wrong"), Purpose::Enrol)
        .await
        .unwrap();
    assert!(matches!(counted, Release::Wrong { .. }), "{counted:?}");
    assert_eq!(attempts(&owner, "boss").await.0, before.0 + 1);
}
