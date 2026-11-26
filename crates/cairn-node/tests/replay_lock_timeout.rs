//! #909 slice C1 (design §4.6): the upgrade replay WAITS for its locks instead of stalling
//! every reader behind it.
//!
//! PostgreSQL queues lock requests in order. A replayed `ALTER TABLE` waiting for ACCESS
//! EXCLUSIVE behind one long reader therefore makes every LATER reader of that table wait too
//! — for as long as the first reader stays open. The loader now replays under a short
//! `lock_timeout`: the waiting change gives up after ~2 s (letting the queued readers through),
//! rolls back, sleeps, and tries the whole file again, until it lands.
//!
//! What these tests pin:
//!   * `run_waiting` lets a third session's read through while its change waits, keeps
//!     retrying (it does not give up at the first timeout), lands once the reader commits, and
//!     leaves its session usable (not stuck in an aborted transaction);
//!   * the loader's connection — which lives on as the daemon's — keeps no `lock_timeout`
//!     after `connect_and_load_schema`, on a replaying and on a skipping connect.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use std::time::{Duration, Instant};

use cairn_node::db;
use cairn_node::replay_wait::{run_waiting, REPLAY_LOCK_TIMEOUT};
use tokio_postgres::{Client, NoTls};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A scratch table of this test's own: no migration touches it, so the test can lock it
/// without disturbing anything else. Dropped on entry too, in case a failed run left it.
const SCRATCH: &str = "zz_replay_lock_timeout_probe";

/// A plain (non-loader) connection.
async fn raw(conn: &str) -> Client {
    let (c, connection) = tokio_postgres::connect(conn, NoTls).await.unwrap();
    tokio::spawn(connection);
    c
}

async fn show_lock_timeout(c: &Client) -> String {
    c.query_one("SHOW lock_timeout", &[]).await.unwrap().get(0)
}

#[tokio::test]
async fn a_waiting_change_lets_readers_through_and_then_lands() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let admin = raw(&base).await;
    admin
        .batch_execute(&format!(
            "DROP TABLE IF EXISTS {SCRATCH}; CREATE TABLE {SCRATCH} (a int); \
             INSERT INTO {SCRATCH} VALUES (1);"
        ))
        .await
        .unwrap();

    // R: the long reader. Its open transaction holds ACCESS SHARE on the table.
    let reader = raw(&base).await;
    reader
        .batch_execute(&format!("BEGIN; SELECT * FROM {SCRATCH};"))
        .await
        .unwrap();

    // C: the loader's connection, under the replay's lock timeout, running one "file" with
    // its own BEGIN/COMMIT — the shape 67 of the db/*.sql files have.
    let loader = raw(&base).await;
    loader
        .batch_execute(&format!("SET lock_timeout = '{REPLAY_LOCK_TIMEOUT}'"))
        .await
        .unwrap();
    let file = format!("BEGIN; ALTER TABLE {SCRATCH} ADD COLUMN IF NOT EXISTS x int; COMMIT;");
    let started = Instant::now();
    let change = tokio::spawn(async move {
        let result = run_waiting(&loader, "test", &file).await;
        (loader, result)
    });

    // Let the ALTER queue behind R, then read from a third session. Behind a waiting change
    // with no timeout this read would stall until R commits; here it waits at most ~2 s.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let third = raw(&base).await;
    let count = tokio::time::timeout(
        Duration::from_secs(3),
        third.query_one(&format!("SELECT count(*) FROM {SCRATCH}"), &[]),
    )
    .await
    .expect("a reader must not stall behind a waiting change for more than ~the timeout")
    .unwrap();
    assert_eq!(count.get::<_, i64>(0), 1);

    // Past the first timeout, the change must still be waiting — retrying, not given up.
    let first_timeout = Duration::from_millis(2_500);
    if let Some(rest) = first_timeout.checked_sub(started.elapsed()) {
        tokio::time::sleep(rest).await;
    }
    assert!(
        !change.is_finished(),
        "the change must keep retrying while the reader holds its lock"
    );

    // R finishes; the change lands on a later attempt.
    reader.batch_execute("COMMIT").await.unwrap();
    let (loader, result) = tokio::time::timeout(Duration::from_secs(10), change)
        .await
        .expect("the change lands within 10 s of the reader committing")
        .unwrap();
    result.expect("run_waiting returns Ok once its lock is free");

    let has_x: bool = admin
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = $1::text::regclass \
                             AND attname = 'x' AND NOT attisdropped)",
            &[&SCRATCH],
        )
        .await
        .unwrap()
        .get(0);
    assert!(has_x, "the retried file really ran");
    // The ROLLBACK after each timeout left the session usable (not aborted, 25P02).
    let one: i32 = loader.query_one("SELECT 1", &[]).await.unwrap().get(0);
    assert_eq!(one, 1);

    admin
        .batch_execute(&format!("DROP TABLE {SCRATCH}"))
        .await
        .unwrap();
}

#[tokio::test]
async fn the_loaders_connection_keeps_no_lock_timeout() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let admin = raw(&base).await;
    let loaded_at = || async {
        admin
            .query_one("SELECT loaded_at::text FROM node_schema", &[])
            .await
            .unwrap()
            .get::<_, String>(0)
    };

    // A replaying connect.
    db::force_schema_replay(&admin).await.unwrap();
    let replayed = db::connect_and_load_schema(&base).await.unwrap();
    assert_eq!(show_lock_timeout(&replayed).await, "0", "after a replay");
    drop(replayed);

    // A skipping connect (anti-vacuity: it wrote no new stamp, so it really skipped).
    let before = loaded_at().await;
    let skipped = db::connect_and_load_schema(&base).await.unwrap();
    assert_eq!(
        loaded_at().await,
        before,
        "precondition: this connect skipped"
    );
    assert_eq!(show_lock_timeout(&skipped).await, "0", "after a skip");
}

/// The loader now replays each file through `run_waiting`, but #467's acceptance criterion is
/// pinned on `load_migration`'s text (tests/db_diagnosis.rs). Pin that the two stay identical.
#[tokio::test]
async fn a_failing_file_reads_the_same_through_run_waiting_as_through_load_migration() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let c = raw(&base).await;
    let body = "SELECT no_such_function_here();";
    let single = db::load_migration(&c, "031_medication", body)
        .await
        .expect_err("the body cannot succeed");
    let waiting = run_waiting(&c, "031_medication", body)
        .await
        .expect_err("the body cannot succeed");
    assert_eq!(format!("{waiting}"), format!("{single}"));
}

/// #1175 (PR #1195 review): since the re-adjudication pass re-raises a lock wait, the loader can
/// retry on an ADVISORY lock — a custody lock held by a late key, a running pass's own lock. The
/// retry's log line must name that holder too, not report "no session found" while the connect
/// waits on it indefinitely.
#[tokio::test]
async fn the_blocker_line_names_an_advisory_lock_holder() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let holder = raw(&base).await;
    let holder_pid: i32 = holder
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    holder.batch_execute("BEGIN").await.unwrap();
    holder
        .execute(
            "SELECT pg_advisory_xact_lock(hashtextextended('cairn.readjudicate', 0))",
            &[],
        )
        .await
        .unwrap();

    let probe = raw(&base).await;
    // A blocker must have held its lock at least `lock_timeout`; keep that short and outwait it.
    probe
        .batch_execute("SET lock_timeout = '100ms'")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let line = cairn_node::replay_wait::blockers(&probe).await;
    holder.batch_execute("ROLLBACK").await.unwrap();

    assert!(
        line.contains(&format!("pid {holder_pid}")),
        "THE LOADER'S WAIT LINE MUST NAME AN ADVISORY-LOCK HOLDER: {line}"
    );
}
