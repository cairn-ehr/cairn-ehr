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

/// The binary's schema generation is DERIVED from the embedded migration list (the
/// numeric prefix of the last entry), never hand-counted — a hand-maintained constant
/// beside the list would be exactly the Rust↔SQL drift-pair disease issue #212 catalogs.
/// Appending db/039_* must bump this without anyone remembering to.
#[test]
fn embedded_schema_version_is_the_last_migration_prefix() {
    // db/038_node_schema.sql is currently the newest embedded migration.
    assert_eq!(db::embedded_schema_version(), 38);
}

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
}
