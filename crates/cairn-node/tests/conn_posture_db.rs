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
/// Members by a grant `WITH INHERIT FALSE`: no privilege flows, but `SET ROLE` still works.
const NOINHERIT_OWNER_PROBE: &str = "cairn_window_probe_noinherit_owner";
const NOINHERIT_NODE_PROBE: &str = "cairn_window_probe_noinherit_node";
/// A login granted, one object at a time, each power a window must not have.
const DIRECT_GRANTS_PROBE: &str = "cairn_window_probe_direct_grants";
/// Logins given a power that reaches the sealed blobs outside every grant (server files, WAL).
const SERVER_FILES_PROBE: &str = "cairn_window_probe_server_files";
const REPLICATION_PROBE: &str = "cairn_window_probe_replication";

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
    // A leftover may hold object grants, which block a bare DROP ROLE: DROP OWNED revokes them.
    owner
        .batch_execute(&format!(
            "DO $$ BEGIN \
               IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{name}') THEN \
                 EXECUTE 'DROP OWNED BY {name}'; EXECUTE 'DROP ROLE {name}'; \
               END IF; \
             END $$; \
             CREATE ROLE {name} LOGIN; GRANT cairn_window TO {name}; {extra}"
        ))
        .await
        .expect("the throwaway login");
    format!("{base} user={name}")
}

/// Undo [`throwaway_login`]: revoke what it was given (its role memberships go with the role,
/// its object grants with DROP OWNED), then drop it.
async fn drop_login(owner: &Client, name: &str) {
    owner
        .batch_execute(&format!(
            "REVOKE cairn_window FROM {name}; DROP OWNED BY {name}; DROP ROLE {name};"
        ))
        .await
        .expect("dropping the throwaway login");
}

/// The facts a throwaway login granted `extra` reads for ITSELF; the login is gone afterwards.
async fn facts_of(owner: &Client, base: &str, name: &str, extra: &str) -> PostureFacts {
    let login = throwaway_login(owner, base, name, extra).await;
    let conn = db::connect(&login).await.unwrap();
    let facts = read_posture(&conn).await.unwrap();
    drop(conn);
    drop_login(owner, name).await;
    facts
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
    let grant = format!("GRANT cairn_node TO {NODE_PROBE};");
    let facts = facts_of(&owner, &base, NODE_PROBE, &grant).await;
    assert!(facts.runs_peer_door, "{facts:?}");
    assert!(over_powered(&facts).is_some(), "{facts:?}");
}

/// A grant `WITH INHERIT FALSE` passes no privilege on, so a check of the login's OWN
/// privileges sees nothing. But the login can still `SET ROLE` into the role (PG16+ keeps the
/// SET option by default) and then has all of it. So every fact is judged over every role the
/// login is a member of.
#[tokio::test]
async fn a_membership_that_does_not_inherit_still_counts() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, owner) = fresh(&base).await;
    let owner_grant = format!(
        "DO $$ BEGIN EXECUTE format('GRANT %I TO {NOINHERIT_OWNER_PROBE} WITH INHERIT FALSE', \
         current_user); END $$;"
    );
    let facts = facts_of(&owner, &base, NOINHERIT_OWNER_PROBE, &owner_grant).await;
    assert!(facts.owns_cairn_table, "{facts:?}");
    assert!(over_powered(&facts).is_some(), "{facts:?}");

    let node_grant = format!("GRANT cairn_node TO {NOINHERIT_NODE_PROBE} WITH INHERIT FALSE;");
    let facts = facts_of(&owner, &base, NOINHERIT_NODE_PROBE, &node_grant).await;
    assert!(facts.runs_peer_door, "{facts:?}");
    assert!(over_powered(&facts).is_some(), "{facts:?}");
}

/// Each of the three object grants is seen on its own, so none of those facts is a constant
/// `false` that a test of the owner (who has everything) could never tell apart. The vault grant
/// is column-level on purpose: one column of the sealed table is already too much.
#[tokio::test]
async fn each_direct_grant_of_a_power_is_seen() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, owner) = fresh(&base).await;
    let grants = format!(
        "GRANT SELECT (staff_id) ON vault_member TO {DIRECT_GRANTS_PROBE}; \
         GRANT INSERT ON event_log TO {DIRECT_GRANTS_PROBE}; \
         GRANT EXECUTE ON FUNCTION cairn_vault_unseal(bytea, text) TO {DIRECT_GRANTS_PROBE};"
    );
    let facts = facts_of(&owner, &base, DIRECT_GRANTS_PROBE, &grants).await;
    // The window role holds none of the doors (pinned above), so `runs_peer_door` here can only
    // come from the `cairn_vault_unseal` grant.
    assert_eq!(
        facts,
        PostureFacts {
            superuser: false,
            owns_cairn_table: false,
            reads_vault: true,
            inserts_event_log: true,
            runs_peer_door: true,
        }
    );
}

/// Some powers reach the sealed keys without any grant on `vault_member`: reading the data
/// files (`pg_read_server_files`, or writing/executing on the server to the same end) or taking
/// a base backup / streaming the WAL (`REPLICATION`). They count as the superuser fact.
#[tokio::test]
async fn powers_that_reach_the_files_outside_grants_are_over_powered() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, owner) = fresh(&base).await;
    let grant = format!("GRANT pg_read_server_files TO {SERVER_FILES_PROBE};");
    let facts = facts_of(&owner, &base, SERVER_FILES_PROBE, &grant).await;
    assert!(facts.superuser, "{facts:?}");
    assert!(over_powered(&facts).is_some(), "{facts:?}");

    let alter = format!("ALTER ROLE {REPLICATION_PROBE} REPLICATION;");
    let facts = facts_of(&owner, &base, REPLICATION_PROBE, &alter).await;
    assert!(facts.superuser, "{facts:?}");
    assert!(over_powered(&facts).is_some(), "{facts:?}");
}

/// Focus 3 and focus 1 on one login. A MEMBER of the owner role is not itself a superuser and
/// owns nothing by `relowner`, so a check of the login's own row alone would miss it; judging
/// the roles it is a member of catches it. Then the operator
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
    let own_rolsuper: bool = conn
        .query_one(
            "SELECT rolsuper FROM pg_roles WHERE rolname = current_user",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !own_rolsuper,
        "membership must not make the login itself rolsuper"
    );
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

/// A connection string can change the CURRENT role without changing the login:
/// `options='-c role=cairn_window'` starts the owner's session already `SET ROLE cairn_window`,
/// so a judgement of `current_user` would read the window's empty powers and release a key over
/// a session that `SET ROLE NONE` turns straight back into the owner. The posture is therefore
/// judged on `session_user`, the authenticated login, which no startup option can change.
#[tokio::test]
async fn a_startup_role_option_does_not_hide_the_owner_login() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, _owner) = fresh(&base).await;
    let disguised = db::connect(&format!("{base} options='-c role=cairn_window'"))
        .await
        .unwrap();
    let current: String = disguised
        .query_one("SELECT current_user::text", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(current, "cairn_window", "the startup option took effect");

    let facts = read_posture(&disguised).await.unwrap();
    assert!(over_powered(&facts).is_some(), "{facts:?}");
    let refused = release_for_act(&disguised, "nobody", &pass("x"), Purpose::Clinical)
        .await
        .unwrap();
    assert!(
        matches!(refused, Release::OverPowered { .. }),
        "{refused:?}"
    );
}
