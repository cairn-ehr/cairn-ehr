//! #909 slice B, Task 1 — db/084: the terminal trust set, the gateway record, and the
//! registration turn.
//!
//! WHY THESE TESTS EXIST. Slice B lets a window on another machine reach the node's Postgres
//! through a pinned mTLS gateway. The gateway's login (`cairn_terminal_service`) may only READ
//! the trust set and REPORT an attempt; approving a terminal is the owner's act, so nobody who
//! can reach the network can admit itself. Slice A's advisory lock, takeable by ANY login, is now
//! a row lock only the service and the owner's CLI take, so no window can starve registration.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`. Test keys are computed
//! at runtime (house rule 6).
mod common;

use cairn_event::{short_fingerprint, SigningKey};
use cairn_node::db;
use cairn_node::patient::register::register_patient;
use cairn_node::terminal_service::registration::{serve_one, Served};
use cairn_patient_search::{CandidateList, SearchQuery};
use common::{cs, owner_and_window, service_login, setup};
use std::time::Duration;
use tokio_postgres::Client;
use uuid::Uuid;

const TODAY: &str = "2026-11-20";
const NODE: &str = "n";
/// Slice A's advisory-lock key (CARNRG, now unused): a window holding it stops no registration.
const OLD_WINDOW_LOCK: i64 = 0x4341_524E_5247;

struct Rig {
    _guard: Client,
    base: String,
    owner: Client,
    window: Client,
    service: Client,
    sk: SigningKey,
    kid: String,
}

/// An empty trust set, queue and record. `None` = no database (the test says it skipped).
async fn rig() -> Option<Rig> {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = db::test_serial_guard(&base).await.unwrap();
    let (owner, window) = owner_and_window(&base).await;
    let (sk, kid) = setup(&owner, &["patient_registration", "patient_name"]).await;
    owner
        .batch_execute("TRUNCATE registration_request; TRUNCATE terminal_trust")
        .await
        .unwrap();
    let service = db::connect(&service_login(&owner, &base).await)
        .await
        .unwrap();
    Some(Rig {
        _guard: guard,
        base,
        owner,
        window,
        service,
        sk,
        kid,
    })
}

/// A deterministic 32-byte key as hex, distinct for each `variant` (byte 0 carries it).
fn terminal_key(variant: u8) -> String {
    let bytes: [u8; 32] = std::array::from_fn(|i| {
        if i == 0 {
            variant
        } else {
            (i as u8).wrapping_mul(7).wrapping_add(variant ^ 0x5a)
        }
    });
    hex::encode(bytes)
}

/// The SQLSTATE of a failed statement.
fn code_of(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.code().code().to_string())
        .unwrap_or_default()
}

async fn attempt(c: &Client, key: &str) {
    c.execute("SELECT cairn_terminal_attempt($1)", &[&key])
        .await
        .unwrap();
}

async fn status_of(owner: &Client, key: &str) -> Option<String> {
    owner
        .query_opt(
            "SELECT status FROM terminal_trust WHERE terminal_pubkey = $1",
            &[&key],
        )
        .await
        .unwrap()
        .map(|r| r.get(0))
}

/// `(decided_by, decided_at)` as text, to compare a decision before and after.
async fn decision(owner: &Client, key: &str) -> (Option<String>, Option<String>) {
    let row = owner
        .query_one(
            "SELECT decided_by, decided_at::text FROM terminal_trust WHERE terminal_pubkey = $1",
            &[&key],
        )
        .await
        .unwrap();
    (row.get(0), row.get(1))
}

/// The window writes a registration request for `name` (nobody displayed: an empty search).
async fn submit(window: &Client, name: &str) -> Uuid {
    let id = Uuid::now_v7();
    let q = serde_json::to_string(&SearchQuery::new(name, None, &[])).unwrap();
    let shown = serde_json::to_string(&CandidateList::empty()).unwrap();
    window
        .query_one(
            "SELECT cairn_registration_submit($1::text::uuid, $2, $3::text::jsonb, $4::text::jsonb, $5)",
            &[&id.to_string(), &name, &q, &shown, &TODAY],
        )
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn fingerprint_in_sql_matches_rust() {
    let Some(r) = rig().await else { return };
    let key = terminal_key(9);
    let sql: String = r
        .owner
        .query_one("SELECT cairn_terminal_fingerprint($1)", &[&key])
        .await
        .unwrap()
        .get(0);
    assert_eq!(sql, short_fingerprint(&key).unwrap());
}

#[tokio::test]
async fn an_attempt_records_pending_and_never_changes_a_decided_row() {
    let Some(r) = rig().await else { return };
    let key = terminal_key(1);
    attempt(&r.service, &key).await;
    assert_eq!(status_of(&r.owner, &key).await.as_deref(), Some("pending"));
    let fp = short_fingerprint(&key).unwrap();
    let admitted: String = r
        .owner
        .query_one("SELECT cairn_terminal_admit($1, 'front desk')", &[&fp])
        .await
        .unwrap()
        .get(0);
    assert_eq!(admitted, key);
    attempt(&r.service, &key).await;
    assert_eq!(status_of(&r.owner, &key).await.as_deref(), Some("active"));
    // Revoked stays revoked too: knocking again must not re-pend it.
    r.owner
        .query_one("SELECT cairn_terminal_revoke($1)", &[&fp])
        .await
        .unwrap();
    attempt(&r.service, &key).await;
    assert_eq!(status_of(&r.owner, &key).await.as_deref(), Some("revoked"));
}

#[tokio::test]
async fn pending_is_capped_at_the_64_newest() {
    let Some(r) = rig().await else { return };
    for v in 0..70u8 {
        attempt(&r.service, &terminal_key(v)).await;
    }
    let n: i64 = r
        .owner
        .query_one("SELECT count(*) FROM terminal_trust", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 64);
    for v in 0..6u8 {
        assert_eq!(status_of(&r.owner, &terminal_key(v)).await, None, "{v}");
    }
    for v in 6..70u8 {
        assert!(status_of(&r.owner, &terminal_key(v)).await.is_some(), "{v}");
    }
}

#[tokio::test]
async fn admit_refuses_unknown_ambiguous_revoked_and_empty_label() {
    let Some(r) = rig().await else { return };
    let admit = |fp: String, label: &'static str| {
        let owner = &r.owner;
        async move {
            owner
                .query_one("SELECT cairn_terminal_admit($1, $2)", &[&fp, &label])
                .await
        }
    };
    let key = terminal_key(2);
    attempt(&r.service, &key).await;
    let fp = short_fingerprint(&key).unwrap();
    // Empty label, and unknown fingerprint: both verdicts (P0001).
    for (f, l) in [
        (fp.clone(), "  "),
        ("0000-0000-0000-0000-0000".to_string(), "x"),
    ] {
        let e = admit(f, l).await.unwrap_err();
        assert_eq!(code_of(&e), "P0001");
    }
    // Ambiguous: two rows with one fingerprint (forced by hand; a real collision is 2^-80).
    let other = terminal_key(3);
    attempt(&r.service, &other).await;
    r.owner
        .execute(
            "UPDATE terminal_trust SET fingerprint = $1 WHERE terminal_pubkey = $2",
            &[&fp, &other],
        )
        .await
        .unwrap();
    let e = admit(fp.clone(), "x").await.unwrap_err();
    assert_eq!(code_of(&e), "P0001");
    assert_eq!(status_of(&r.owner, &key).await.as_deref(), Some("pending"));
    r.owner
        .execute(
            "UPDATE terminal_trust SET fingerprint = $1 WHERE terminal_pubkey = $2",
            &[&short_fingerprint(&other).unwrap(), &other],
        )
        .await
        .unwrap();
    // Admit once; admitting again (no longer pending) is refused.
    admit(fp.clone(), "x").await.unwrap();
    let e = admit(fp, "x").await.unwrap_err();
    assert_eq!(code_of(&e), "P0001");
}

#[tokio::test]
async fn revoke_then_admit_is_refused() {
    let Some(r) = rig().await else { return };
    let key = terminal_key(4);
    attempt(&r.service, &key).await;
    let fp = short_fingerprint(&key).unwrap();
    r.owner
        .query_one("SELECT cairn_terminal_revoke($1)", &[&fp])
        .await
        .unwrap();
    let e = r
        .owner
        .query_one("SELECT cairn_terminal_admit($1, 'x')", &[&fp])
        .await
        .unwrap_err();
    assert_eq!(code_of(&e), "P0001");
    assert_eq!(status_of(&r.owner, &key).await.as_deref(), Some("revoked"));
    // Revoking a revoked terminal is idempotent: same key, first decision kept.
    let before = decision(&r.owner, &key).await;
    let again: String = r
        .owner
        .query_one("SELECT cairn_terminal_revoke($1)", &[&fp])
        .await
        .unwrap()
        .get(0);
    assert_eq!(again, key);
    assert_eq!(
        decision(&r.owner, &key).await,
        before,
        "first decision kept"
    );
    // Revoke of an unknown fingerprint raises too.
    let e = r
        .owner
        .query_one(
            "SELECT cairn_terminal_revoke('0000-0000-0000-0000-0000')",
            &[],
        )
        .await
        .unwrap_err();
    assert_eq!(code_of(&e), "P0001");
}

#[tokio::test]
async fn the_window_role_cannot_read_the_trust_set_or_take_the_turn() {
    let Some(r) = rig().await else { return };
    for sql in [
        "SELECT count(*) FROM terminal_trust",
        "SELECT count(*) FROM terminal_gateway",
        "SELECT cairn_registration_take_turn()",
        "SELECT cairn_terminal_attempt('00')",
        "SELECT cairn_terminal_admit('0000-0000-0000-0000-0000', 'x')",
        "SELECT cairn_terminal_revoke('0000-0000-0000-0000-0000')",
        "INSERT INTO terminal_trust (terminal_pubkey, fingerprint, status, first_seen, last_seen) \
         VALUES (repeat('a', 64), 'x', 'active', now(), now())",
    ] {
        let e = r.window.query_one(sql, &[]).await.unwrap_err();
        assert_eq!(code_of(&e), "42501", "{sql}");
    }
}

/// The service's login may read the trust set and report attempts, and nothing more: admitting,
/// revoking and writing the tables are the owner's alone (nobody on the network admits itself).
#[tokio::test]
async fn the_service_role_cannot_admit_revoke_or_write() {
    let Some(r) = rig().await else { return };
    let key = terminal_key(5);
    attempt(&r.service, &key).await;
    let fp = short_fingerprint(&key).unwrap();
    for sql in [
        format!("SELECT cairn_terminal_admit('{fp}', 'x')"),
        format!("SELECT cairn_terminal_revoke('{fp}')"),
        format!("SELECT cairn_terminal_lock_one('{fp}')"),
        "INSERT INTO terminal_trust (terminal_pubkey, fingerprint, status, first_seen, last_seen) \
         VALUES (repeat('b', 64), 'x', 'active', now(), now())"
            .to_string(),
        "UPDATE terminal_trust SET status = 'active'".to_string(),
        "INSERT INTO terminal_gateway (upstream, checked_at, checked_by) \
         VALUES ('u', now(), 'x')"
            .to_string(),
    ] {
        let e = r.service.execute(sql.as_str(), &[]).await.unwrap_err();
        assert_eq!(code_of(&e), "42501", "{sql}");
    }
    assert_eq!(status_of(&r.owner, &key).await.as_deref(), Some("pending"));
}

/// A malformed key is a clear verdict (P0001), not an incidental decode error.
#[tokio::test]
async fn a_malformed_key_is_refused_clearly() {
    let Some(r) = rig().await else { return };
    for bad in ["zz", "ABCD", &"a".repeat(63)] {
        let e = r
            .service
            .execute("SELECT cairn_terminal_attempt($1)", &[&bad])
            .await
            .unwrap_err();
        assert_eq!(code_of(&e), "P0001", "{bad}");
        let e = r
            .owner
            .query_one("SELECT cairn_terminal_fingerprint($1)", &[&bad])
            .await
            .unwrap_err();
        assert_eq!(code_of(&e), "P0001", "{bad}");
    }
}

/// A missing turn row must fail closed: a silent no-lock would let registrations run in parallel.
#[tokio::test]
async fn a_missing_turn_row_fails_closed() {
    let Some(r) = rig().await else { return };
    r.owner
        .batch_execute("BEGIN; DELETE FROM registration_turn")
        .await
        .unwrap();
    let e = r
        .owner
        .query_one("SELECT cairn_registration_take_turn()", &[])
        .await
        .unwrap_err();
    assert_eq!(code_of(&e), "P0001");
    r.owner.batch_execute("ROLLBACK").await.unwrap();
}

/// Connection A (the owner) holds the turn in an open transaction, returned for the caller to end.
async fn hold_turn(base: &str) -> Client {
    let a = db::connect(base).await.unwrap();
    a.batch_execute("BEGIN").await.unwrap();
    a.query_one("SELECT cairn_registration_take_turn()", &[])
        .await
        .unwrap();
    a
}

#[tokio::test]
async fn a_held_turn_blocks_the_service_but_a_window_advisory_lock_does_not() {
    let Some(mut r) = rig().await else { return };
    // A held turn blocks serve_one.
    submit(&r.window, "Turn Holder").await;
    let a = hold_turn(&r.base).await;
    let (sk, mut service) = (r.sk.clone(), r.service);
    let mut task = tokio::spawn(async move {
        let served = serve_one(&mut service, &sk, NODE).await;
        (service, served)
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(500), &mut task)
            .await
            .is_err(),
        "serve_one returned while another transaction held the turn"
    );
    a.batch_execute("COMMIT").await.unwrap();
    let (service, served) = task.await.unwrap();
    assert!(matches!(served.unwrap(), Served::Registered { .. }));
    r.service = service;

    // A window holding slice A's old advisory key blocks nobody.
    submit(&r.window, "Window Lock Victim").await;
    r.window.batch_execute("BEGIN").await.unwrap();
    r.window
        .execute("SELECT pg_advisory_xact_lock($1)", &[&OLD_WINDOW_LOCK])
        .await
        .unwrap();
    let served = tokio::time::timeout(
        Duration::from_millis(2000),
        serve_one(&mut r.service, &r.sk, NODE),
    )
    .await
    .expect("a window's advisory lock must not block the service")
    .unwrap();
    assert!(matches!(served, Served::Registered { .. }));
    r.window.batch_execute("ROLLBACK").await.unwrap();
}

#[tokio::test]
async fn cli_registration_takes_the_turn() {
    let Some(r) = rig().await else { return };
    let a = hold_turn(&r.base).await;
    let mut b = db::connect(&r.base).await.unwrap();
    let (sk, kid) = (r.sk.clone(), r.kid.clone());
    let mut task = tokio::spawn(async move {
        let q = SearchQuery::new("Cli Person", None, &[]);
        register_patient(
            &mut b,
            &sk,
            &kid,
            NODE,
            Some("Cli Person"),
            &q,
            &CandidateList::empty(),
        )
        .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(500), &mut task)
            .await
            .is_err(),
        "the CLI's registration returned while the turn was held"
    );
    a.batch_execute("COMMIT").await.unwrap();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn an_idle_serve_one_takes_no_turn() {
    let Some(mut r) = rig().await else { return };
    let a = hold_turn(&r.base).await;
    // Nothing pending: idle returns at once, which it could not if it tried to take the turn.
    let idle = tokio::time::timeout(
        Duration::from_millis(500),
        serve_one(&mut r.service, &r.sk, NODE),
    )
    .await
    .expect("an idle serve_one must not wait on the turn")
    .unwrap();
    assert_eq!(idle, Served::Idle);

    // With a pending request it does want the turn, and waits for A.
    submit(&r.window, "Idle Then Busy").await;
    let (sk, mut service) = (r.sk.clone(), r.service);
    let mut task = tokio::spawn(async move {
        let served = serve_one(&mut service, &sk, NODE).await;
        (service, served)
    });
    assert!(tokio::time::timeout(Duration::from_millis(500), &mut task)
        .await
        .is_err());
    a.batch_execute("COMMIT").await.unwrap();
    let (_service, served) = task.await.unwrap();
    assert!(matches!(served.unwrap(), Served::Registered { .. }));
}

#[tokio::test]
async fn every_new_door_is_closed_to_public() {
    let Some(r) = rig().await else { return };
    for f in [
        "cairn_terminal_fingerprint(text)",
        "cairn_terminal_attempt(text)",
        "cairn_terminal_lock_one(text)",
        "cairn_terminal_admit(text, text)",
        "cairn_terminal_revoke(text)",
        "cairn_registration_take_turn(boolean)",
    ] {
        let open: bool = r
            .owner
            .query_one(
                "SELECT has_function_privilege('public', $1::text::regprocedure, 'EXECUTE')",
                &[&f],
            )
            .await
            .unwrap()
            .get(0);
        assert!(!open, "{f} is executable by PUBLIC");
    }
    for t in ["terminal_trust", "terminal_gateway", "registration_turn"] {
        let open: bool = r
            .owner
            .query_one("SELECT has_table_privilege('public', $1, 'SELECT')", &[&t])
            .await
            .unwrap()
            .get(0);
        assert!(!open, "{t} is readable by PUBLIC");
    }
}
