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
    // An idle open transaction would hold hlc_state's row lock and stall every writer (db/062).
    let settings: Vec<String> = db
        .query_one(
            "SELECT coalesce(s.setconfig, '{}') FROM pg_roles r \
               LEFT JOIN pg_db_role_setting s ON s.setrole = r.oid AND s.setdatabase = 0 \
              WHERE r.rolname = $1",
            &[&ROLE],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        settings.contains(&"idle_in_transaction_session_timeout=60s".to_string()),
        "{settings:?}"
    );
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

/// A set-but-empty password (a failed `$(cat file)`) is refused before anything, never read as
/// "keep the old one".
#[tokio::test]
async fn an_empty_password_variable_is_refused_before_anything() {
    let Some(cs) = common::cs() else {
        eprintln!("skipped: no CAIRN_TEST_PG");
        return;
    };
    let db = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    drop_role(&db).await;
    let out = Command::new(env!("CARGO_BIN_EXE_cairn-node"))
        .args(["--conn", &cs, "provision-window-role", "--role", ROLE])
        .env("CAIRN_WINDOW_DB_PASSWORD", "  ")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("set but empty"), "{err}");
    assert!(!cairn_node::window_access::role_exists(&db, ROLE)
        .await
        .unwrap());
}

#[tokio::test]
async fn an_existing_role_that_is_not_a_window_login_is_refused_and_untouched() {
    let Some(cs) = common::cs() else {
        eprintln!("skipped: no CAIRN_TEST_PG");
        return;
    };
    let _guard = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    let db = cairn_node::db::connect_and_load_schema(&cs).await.unwrap();
    let pw = format!("{}-{}", "a long enough", "test phrase");
    let me: String = db
        .query_one("SELECT current_user::text", &[])
        .await
        .unwrap()
        .get(0);
    let before: Option<String> = db
        .query_one(
            "SELECT rolpassword FROM pg_authid WHERE rolname = current_user",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    // The owner's own login (a superuser).
    let why = provision_window_login(&db, &me, Some(&pw))
        .await
        .unwrap_err()
        .to_string();
    assert!(why.contains("is not a window login") && why.contains("superuser"));
    let after: Option<String> = db
        .query_one(
            "SELECT rolpassword FROM pg_authid WHERE rolname = current_user",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(before, after);

    // NOLOGIN role, and a login that is a member of cairn_node.
    let nologin = "cairn_wtest_nologin";
    let node_login = "cairn_wtest_nodelogin";
    for r in [nologin, node_login] {
        db.batch_execute(&format!("DROP ROLE IF EXISTS {r}"))
            .await
            .unwrap();
    }
    db.batch_execute(&format!(
        "CREATE ROLE {nologin} NOLOGIN; CREATE ROLE {node_login} LOGIN; GRANT cairn_node TO {node_login}"
    ))
    .await
    .unwrap();
    let why = provision_window_login(&db, nologin, Some(&pw))
        .await
        .unwrap_err()
        .to_string();
    assert!(why.contains("is not a window login") && why.contains("it cannot log in"));
    let why = provision_window_login(&db, node_login, Some(&pw))
        .await
        .unwrap_err()
        .to_string();
    assert!(why.contains("is not a window login") && why.contains("other than cairn_window"));
    for r in [nologin, node_login] {
        db.batch_execute(&format!("DROP ROLE {r}")).await.unwrap();
    }

    // A plain login whose only excess is an object grant: only the posture check can refuse it,
    // and it must leave the login alone (it has no password, so a refresh would give it one).
    let reader = "cairn_wtest_reader";
    db.batch_execute(&format!(
        "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='{reader}') THEN \
           EXECUTE 'DROP OWNED BY {reader}'; EXECUTE 'DROP ROLE {reader}'; END IF; END $$; \
         CREATE ROLE {reader} LOGIN; GRANT SELECT (staff_id) ON vault_member TO {reader}"
    ))
    .await
    .unwrap();
    let why = provision_window_login(&db, reader, Some(&pw))
        .await
        .unwrap_err()
        .to_string();
    assert!(why.contains("can read the vault's sealed keys"), "{why}");
    let untouched: bool = db
        .query_one(
            "SELECT rolpassword IS NULL AND NOT pg_has_role(rolname, 'cairn_window', 'MEMBER') \
               FROM pg_authid WHERE rolname = $1",
            &[&reader],
        )
        .await
        .unwrap()
        .get(0);
    assert!(untouched, "the refused login was changed");
    db.batch_execute(&format!("DROP OWNED BY {reader}; DROP ROLE {reader}"))
        .await
        .unwrap();

    // A login other roles are members of: a GRANT to it would reach every member.
    let group = "cairn_wtest_group";
    let in_group = "cairn_wtest_in_group";
    for r in [in_group, group] {
        db.batch_execute(&format!("DROP ROLE IF EXISTS {r}"))
            .await
            .unwrap();
    }
    db.batch_execute(&format!(
        "CREATE ROLE {group} LOGIN; CREATE ROLE {in_group} NOLOGIN; GRANT {group} TO {in_group}"
    ))
    .await
    .unwrap();
    let why = provision_window_login(&db, group, Some(&pw))
        .await
        .unwrap_err()
        .to_string();
    assert!(why.contains("other roles are members of it"), "{why}");
    for r in [in_group, group] {
        db.batch_execute(&format!("DROP ROLE {r}")).await.unwrap();
    }

    // The upgrade path: an existing window login is still refreshed.
    drop_role(&db).await;
    provision_window_login(&db, ROLE, Some(&pw)).await.unwrap();
    provision_window_login(&db, ROLE, Some(&pw)).await.unwrap();
    drop_role(&db).await;
}

/// Right after a reload the files are not newer than what the server loaded. (The stale side
/// needs the server's own files edited, which no test may do; `hba_freshness` pins its verdict.)
#[tokio::test]
async fn the_hba_files_read_fresh_after_a_reload() {
    let Some(cs) = common::cs() else {
        eprintln!("skipped: no CAIRN_TEST_PG");
        return;
    };
    let db = cairn_node::db::connect(&cs).await.unwrap();
    let before: std::time::SystemTime = db
        .query_one("SELECT pg_conf_load_time()", &[])
        .await
        .unwrap()
        .get(0);
    db.batch_execute("SELECT pg_reload_conf()").await.unwrap();
    // The postmaster reloads asynchronously; a new backend shows its new load time.
    let mut fresh = None;
    for _ in 0..50 {
        let probe = cairn_node::db::connect(&cs).await.unwrap();
        let now: std::time::SystemTime = probe
            .query_one("SELECT pg_conf_load_time()", &[])
            .await
            .unwrap()
            .get(0);
        if now > before {
            fresh = Some(probe);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let probe = fresh.expect("the server reloaded within 5 s");
    assert!(!cairn_node::window_access::hba_file_is_stale(&probe)
        .await
        .unwrap());
}

/// All of the role's DDL is one transaction: a step that fails leaves no half-made LOGIN behind
/// (one with no password, which a re-run would then report as provisioned). A CREATEROLE login
/// with no ADMIN on `cairn_window` can CREATE the role but not GRANT it, so the GRANT fails.
#[tokio::test]
async fn a_failed_step_leaves_no_half_made_login() {
    let Some(cs) = common::cs() else {
        eprintln!("skipped: no CAIRN_TEST_PG");
        return;
    };
    let _guard = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    let owner = cairn_node::db::connect_and_load_schema(&cs).await.unwrap();
    drop_role(&owner).await;
    let creator = "cairn_wtest_creator";
    owner
        .batch_execute(&format!(
            "DROP ROLE IF EXISTS {creator}; CREATE ROLE {creator} LOGIN CREATEROLE"
        ))
        .await
        .unwrap();
    let db = cairn_node::db::connect(&format!("{cs} user={creator}"))
        .await
        .unwrap();
    let pw = format!("{}-{}", "a long enough", "test phrase");
    let err = provision_window_login(&db, ROLE, Some(&pw))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("provisioning window role"), "{err}");
    drop(db);
    let left = cairn_node::window_access::role_exists(&owner, ROLE)
        .await
        .unwrap();
    drop_role(&owner).await;
    owner
        .batch_execute(&format!("DROP ROLE {creator}"))
        .await
        .unwrap();
    assert!(!left, "the failed provisioning left the login behind");
}

/// A login that may not read `pg_hba_file_rules` is told to run the step as the schema owner,
/// never steered to some other superuser (whose replay would own the schema instead).
#[tokio::test]
async fn reading_the_access_rules_without_the_power_names_the_schema_owner() {
    let Some(cs) = common::cs() else {
        eprintln!("skipped: no CAIRN_TEST_PG");
        return;
    };
    let _guard = cairn_node::db::test_serial_guard(&cs).await.unwrap();
    let owner = cairn_node::db::connect_and_load_schema(&cs).await.unwrap();
    let window = cairn_node::db::connect(&common::window_login(&owner, &cs).await)
        .await
        .unwrap();
    let why = cairn_node::window_access::read_hba_rules(&window)
        .await
        .unwrap_err()
        .to_string();
    assert!(why.contains("as the schema owner"), "{why}");
}

/// The verb must refuse before it mutates: each step appears in this order in its match arm.
#[test]
fn the_verb_refuses_before_it_mutates_in_source_order() {
    let src = include_str!("../src/main.rs");
    let arm = &src[src.find("Cmd::ProvisionWindowRole { role } =>").unwrap()..];
    let arm = &arm[..arm.find("Cmd::Status {").unwrap()];
    let mut at = 0;
    for needle in [
        "require_local",
        "db::connect(",
        "read_hba_rules",
        "hba_file_is_stale",
        "hba_verdict",
        "connect_and_load_schema",
        "provision_window_login",
    ] {
        let i = arm[at..]
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} missing or out of order"));
        at += i + needle.len();
    }
}
