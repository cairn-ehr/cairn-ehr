//! Issue #726 — a connect whose schema has not changed replays NOTHING, so it locks nothing.
//!
//! `connect_and_load_schema` used to replay every embedded `db/*.sql` on every connect. Each
//! statement is idempotent, but idempotent is not lock-free: measured on PG 18, a NO-OP
//! replay took ACCESS EXCLUSIVE on `event_log` and twenty other tables plus 34 views, and
//! SHARE / SHARE ROW EXCLUSIVE on more. Each of those waits behind any open reader or writer
//! (the matcher worker's multi-second read, a long reprojection), and PostgreSQL then queues
//! every later clinical write behind the waiting lock — so starting any node process could
//! stall charting.
//!
//! The fix: after a successful replay the loader stamps `node_schema` with WHO replayed
//! (`loader_build`) and a hash of exactly WHAT it replayed (`schema_hash`, db/058). A later
//! connect whose generation, loader and hash all match skips the file replay entirely.
//!
//! What these tests pin:
//!   * the headline guarantee, mechanically: with another session holding ROW EXCLUSIVE on
//!     every table and view (what any clinical writer holds), a second connect completes
//!     under a short `lock_timeout` — before the gate it timed out on its first DDL;
//!   * the skip is real (a dropped index stays dropped across a matching connect) — the
//!     control that makes the lock test meaningful;
//!   * a changed hash, or another loader's stamp, forces a full replay that repairs it;
//!   * the stamp records this build's hash.
//!
//! Every DB test still starts from a full replay: `db::test_serial_guard` clears the stamp's
//! hash, so a test cannot inherit a schema a previous test sabotaged. Within a test, a second
//! connect is gated like production — use `db::force_schema_replay` when a test needs it.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_node::db;
use tokio_postgres::{Client, NoTls};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A probe index the replay re-creates (db/056: `CREATE INDEX IF NOT EXISTS`). Dropping it is
/// a cheap, visible stand-in for "the schema differs from what the files say".
const PROBE_INDEX: &str = "match_pending_patient_idx";

async fn index_exists(c: &Client, name: &str) -> bool {
    c.query_one(
        "SELECT to_regclass($1) IS NOT NULL",
        &[&format!("public.{name}")],
    )
    .await
    .unwrap()
    .get(0)
}

/// A plain (non-loader) connection, for the "other session" and for inspection.
async fn raw(conn: &str) -> Client {
    let (c, connection) = tokio_postgres::connect(conn, NoTls).await.unwrap();
    tokio::spawn(connection);
    c
}

#[tokio::test]
async fn an_unchanged_schema_is_not_replayed_so_a_connect_waits_on_no_lock() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    // First connect: the guard cleared the stamp, so this replays in full and stamps.
    drop(db::connect_and_load_schema(&base).await.unwrap());

    // The other session: ROW EXCLUSIVE on every table and view in the schema, held open.
    // That is the lock any clinical INSERT/UPDATE holds, and it conflicts with every strong
    // lock the replay used to take (SHARE for CREATE INDEX, SHARE ROW EXCLUSIVE for
    // CREATE TRIGGER, ACCESS EXCLUSIVE for ALTER/DROP/CREATE OR REPLACE VIEW).
    let mut writer = raw(&base).await;
    let tx = writer.transaction().await.unwrap();
    let relations: Vec<String> = tx
        .query(
            "SELECT format('%I', c.relname) FROM pg_class c \
              WHERE c.relnamespace = 'public'::regnamespace AND c.relkind IN ('r','p','v')",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert!(
        relations.len() > 50,
        "anti-vacuity: the schema is loaded ({} relations)",
        relations.len()
    );
    tx.batch_execute(&format!(
        "LOCK TABLE {} IN ROW EXCLUSIVE MODE",
        relations.join(", ")
    ))
    .await
    .unwrap();

    // Second connect, with a short lock_timeout: any strong lock it tried to take would wait
    // on the writer and fail with 55P03 after 2 s, instead of stalling forever.
    let impatient = format!("{base} options='-c lock_timeout=2000'");
    let second = db::connect_and_load_schema(&impatient).await;
    tx.rollback().await.unwrap();
    assert!(
        second.is_ok(),
        "a connect with an unchanged schema must not wait on any clinical writer's lock: {:?}",
        second.err()
    );
}

#[tokio::test]
async fn a_matching_stamp_skips_the_replay_and_a_changed_hash_forces_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert!(
        index_exists(&c, PROBE_INDEX).await,
        "precondition: the replay created it"
    );

    // The control: with the stamp matching, a connect does not replay, so a hand-made
    // difference survives it. (This is what makes the lock test above meaningful.)
    c.batch_execute(&format!("DROP INDEX {PROBE_INDEX}"))
        .await
        .unwrap();
    drop(db::connect_and_load_schema(&base).await.unwrap());
    assert!(
        !index_exists(&c, PROBE_INDEX).await,
        "a matching stamp must skip the replay entirely"
    );

    // A different build's files (here: a hash that is not this build's) force a full replay.
    c.batch_execute("UPDATE node_schema SET schema_hash = '\\x00'::bytea")
        .await
        .unwrap();
    drop(db::connect_and_load_schema(&base).await.unwrap());
    assert!(
        index_exists(&c, PROBE_INDEX).await,
        "a changed hash must replay every file, repairing the schema"
    );
}

#[tokio::test]
async fn another_loaders_stamp_forces_a_replay() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute(&format!("DROP INDEX {PROBE_INDEX}"))
        .await
        .unwrap();
    // cairn-sync's `init` replays only ITS subset and stamps its own name, leaving our hash
    // column untouched. That stamp vouches for nothing about cairn-node's files, so it must
    // not be read as "up to date": the hash alone is not enough, the loader must match too.
    c.batch_execute("UPDATE node_schema SET loader_build = 'cairn-sync 0.0.0'")
        .await
        .unwrap();
    drop(db::connect_and_load_schema(&base).await.unwrap());
    assert!(
        index_exists(&c, PROBE_INDEX).await,
        "a stamp written by another loader must force a full replay"
    );
}

#[tokio::test]
async fn the_stamp_records_this_builds_hash_and_force_replay_clears_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let stamped: Option<Vec<u8>> = c
        .query_one("SELECT schema_hash FROM node_schema", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        stamped.as_deref(),
        Some(&db::embedded_schema_hash()[..]),
        "a successful replay stamps exactly this build's hash"
    );

    db::force_schema_replay(&c).await.unwrap();
    c.batch_execute(&format!("DROP INDEX {PROBE_INDEX}"))
        .await
        .unwrap();
    drop(db::connect_and_load_schema(&base).await.unwrap());
    assert!(
        index_exists(&c, PROBE_INDEX).await,
        "force_schema_replay must make the next connect replay in full"
    );
}
