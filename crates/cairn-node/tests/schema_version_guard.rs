//! Issue #188 (2026-07-15 review, finding D1) — an older binary must REFUSE to
//! replay its schema over a database already loaded by a newer binary.
//!
//! `connect_and_load_schema` re-runs every embedded db/*.sql on every connect. Without
//! a recorded schema generation there is no refusal rule: an older `cairn-node` binary
//! connecting to a newer database `CREATE OR REPLACE`s newer function bodies — including
//! the in-DB safety-floor checks — back to their older versions, silently. Two binary
//! versions touching one DB (a pilot mid-upgrade, a GUI sidecar, any second tool linking
//! the loader) is all it takes. This is the first brick of the ADR-0012 code plane: the
//! full signed distribution plane can wait; the refusal rule cannot.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via
//! `db::test_serial_guard` (mirrors migration_replay_widening.rs).
use cairn_node::db;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

// The generation itself is the repo-wide `cairn_event::schema_generation::
// SCHEMA_GENERATION` constant, shared with cairn-sync so one git build reports one
// generation through either door (the sync subset legitimately lags the newest
// migration, so per-list derivation would split the two). Its honesty guards live
// where the facts live: crates/cairn-event/tests/schema_generation.rs pins the
// constant to the newest db/*.sql on disk, and db.rs's unit test pins that
// cairn-node's FULL list actually embeds that newest file.

#[tokio::test]
async fn loader_stamps_the_generation_and_refuses_a_newer_db() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let embedded = db::embedded_schema_version();

    // Plain connection (no schema replay) for tampering with the recorded version —
    // and for healing residue: if a previous run of this test aborted between tamper
    // and restore, the DB still claims a future generation and EVERY suite's
    // connect_and_load_schema would refuse. Clamp it back down before starting.
    let admin = db::connect(&base).await.unwrap();
    admin
        .batch_execute(&format!(
            "DO $$ BEGIN
               IF to_regclass('public.node_schema') IS NOT NULL THEN
                 UPDATE node_schema SET version = LEAST(version, {embedded});
               END IF;
             END $$;"
        ))
        .await
        .unwrap();

    // 1. Happy path: a successful load stamps the singleton record with this binary's
    //    generation and an identifying build string.
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let row = c
        .query_one("SELECT version, loader_build FROM node_schema", &[])
        .await
        .unwrap();
    assert_eq!(
        row.get::<_, i32>(0),
        embedded,
        "a successful replay must record the loader's schema generation"
    );
    assert!(
        !row.get::<_, String>(1).is_empty(),
        "loader_build must identify the stamping binary"
    );
    drop(c);

    // 2. Old binary, new DB: pretend a newer binary loaded this database. THIS binary
    //    must now refuse to connect-and-replay rather than downgrade the floor.
    admin
        .execute("UPDATE node_schema SET version = $1", &[&(embedded + 1)])
        .await
        .unwrap();
    let refused = db::connect_and_load_schema(&base).await;
    // Restore BEFORE asserting: a panic below must not leave the shared test database
    // claiming a future generation (which would wedge every later suite's connect).
    admin
        .execute("UPDATE node_schema SET version = $1", &[&embedded])
        .await
        .unwrap();
    let err = match refused {
        Ok(_) => panic!(
            "an older binary replayed its schema over a newer database — \
             silent safety-floor downgrade (issue #188)"
        ),
        Err(e) => e.to_string(),
    };
    assert!(
        err.contains(&format!("{}", embedded + 1)) && err.contains(&format!("{embedded}")),
        "the refusal must name both generations so the operator can act on it: {err}"
    );

    // 3. The restored database loads again — the guard refuses only genuine downgrades.
    db::connect_and_load_schema(&base).await.unwrap();

    // 4. Table present, row ABSENT — the documented "hand-loaded rig, never stamped"
    //    posture: generation unknown, replay proceeds (and re-stamps).
    admin.execute("DELETE FROM node_schema", &[]).await.unwrap();
    db::connect_and_load_schema(&base)
        .await
        .expect("an unstamped database is 'generation unknown' and must load");
    let restamped: i32 = admin
        .query_one("SELECT version FROM node_schema", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(restamped, embedded, "the load must re-stamp the record");
}

/// The guard must read the recorded generation UNDER the loaders' advisory load-lock
/// (2026-07-19 review of PR #251, finding 1). Check-then-act is not enough: an old
/// and a new binary connecting together can interleave so the old one reads a stale
/// generation, passes the check, and still replays over the schema the new one just
/// loaded — the exact silent downgrade #188 exists to stop, through a timing window.
///
/// Deterministic shape: an admin session holds `SCHEMA_LOAD_LOCK` (standing in for a
/// concurrent newer loader mid-replay), we spawn this binary's loader, and only THEN
/// bump the recorded generation and release the lock. A correct loader parks on the
/// lock and sees the bumped generation — refusal. A check-first loader has already
/// read the stale generation (the sleep gives it ample time to finish outright) and
/// succeeds — which this test turns into a failure.
#[tokio::test]
async fn guard_check_happens_under_the_load_lock() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let embedded = db::embedded_schema_version();
    let admin = db::connect(&base).await.unwrap();
    // Heal tamper residue from any previously aborted run (same clamp as above).
    admin
        .batch_execute(&format!(
            "DO $$ BEGIN
               IF to_regclass('public.node_schema') IS NOT NULL THEN
                 UPDATE node_schema SET version = LEAST(version, {embedded});
               END IF;
             END $$;"
        ))
        .await
        .unwrap();
    // Baseline: schema present and stamped at this binary's generation.
    db::connect_and_load_schema(&base).await.unwrap();

    // The "concurrent newer loader": holds the load-lock while mid-replay.
    admin
        .execute(
            "SELECT pg_advisory_lock($1)",
            &[&cairn_event::schema_generation::SCHEMA_LOAD_LOCK],
        )
        .await
        .unwrap();
    let base2 = base.clone();
    let loader = tokio::spawn(async move { db::connect_and_load_schema(&base2).await });
    // Long enough for a check-first (buggy) loader to run to completion; a correct
    // loader is parked on the lock for however long this takes, so the duration
    // cannot make the test flaky in the green state.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    // The "newer loader" finishes: stamp a newer generation, release the lock.
    admin
        .execute("UPDATE node_schema SET version = $1", &[&(embedded + 1)])
        .await
        .unwrap();
    admin
        .execute(
            "SELECT pg_advisory_unlock($1)",
            &[&cairn_event::schema_generation::SCHEMA_LOAD_LOCK],
        )
        .await
        .unwrap();
    let result = loader.await.unwrap();
    // Restore BEFORE asserting so a failure cannot strand the shared database.
    admin
        .execute("UPDATE node_schema SET version = $1", &[&embedded])
        .await
        .unwrap();
    assert!(
        result.is_err(),
        "the loader read the recorded generation BEFORE taking the load-lock: \
         check-then-act TOCTOU — a concurrent old binary can still downgrade the floor"
    );
}
