//! Repair path R7 (#742): db/056's status read reports `charts_changing` — how many charts have
//! a waiting CHANGE notice — so the banner of a chart that is itself already checked can say
//! "running" while a duplicate registered after its check may still be proposed against it. Split from
//! `match_pending.rs` (R4's suite) to keep that file under 500 lines.
//!
//! DB-gated on $CAIRN_TEST_PG, serialized via `db::test_serial_guard`.
mod common;
use cairn_node::db;
use common::cs;
use tokio_postgres::Client;
use uuid::Uuid;

/// A loaded schema with an empty notice log and no worker row. The guard is the
/// advisory-lock-holding connection `test_serial_guard` returns; keep it alive. (A file-local copy
/// of `match_pending.rs`'s helper: a shared `pub fn` in `tests/common` would also need registering
/// in `identity_scaffolding_shared.rs`.)
async fn fresh_db() -> Option<(Client, Client)> {
    let base = cs()?;
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE match_pending, match_worker_state")
        .await
        .unwrap();
    Some((c, guard))
}

/// #742 (R7): `charts_changing` counts the distinct charts with a waiting CHANGE notice. A
/// whole-population `config` re-check is queued before any chart's own check runs, so a checked
/// chart's banner may stay quiet through one; a `change` notice is one this chart's check may not
/// have seen, so the banner names it. Two notices for one chart count once.
#[tokio::test]
async fn the_status_counts_the_charts_with_a_waiting_change() {
    let Some((c, _g)) = fresh_db().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    async fn charts_changing(c: &Client) -> i64 {
        c.query_one(
            "SELECT charts_changing FROM cairn_duplicate_check_status()",
            &[],
        )
        .await
        .unwrap()
        .get(0)
    }
    assert_eq!(charts_changing(&c).await, 0, "nothing waits");
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    c.batch_execute("INSERT INTO match_worker_state (matcher_version) VALUES ('v')")
        .await
        .unwrap();
    c.execute(
        "INSERT INTO match_pending (patient_id, reason) VALUES ($1::text::uuid, 'config')",
        &[&a.to_string()],
    )
    .await
    .unwrap();
    assert_eq!(
        charts_changing(&c).await,
        0,
        "a config re-check alone is not a change"
    );
    c.execute(
        "INSERT INTO match_pending (patient_id, reason) VALUES \
         ($1::text::uuid, 'change'), ($1::text::uuid, 'change')",
        &[&b.to_string()],
    )
    .await
    .unwrap();
    assert_eq!(
        charts_changing(&c).await,
        1,
        "two notices for one chart count once"
    );
}

/// R7 (#742) added `charts_changing`. A database that loaded the shape before it (`quiet_age_s`,
/// no `charts_changing`) is healed the same way: the guard drops ANY shape lacking the newest
/// column, so one check covers every older shape.
#[tokio::test]
async fn a_database_with_the_pre_r7_status_shape_is_healed_on_connect() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute(
        "DROP FUNCTION cairn_duplicate_check_status(); \
         CREATE FUNCTION cairn_duplicate_check_status(OUT charts_waiting bigint, \
           OUT quiet_age_s bigint, OUT config_recheck boolean, OUT worker_seen boolean, \
           OUT last_drained_hhmm text) LANGUAGE sql AS $$ SELECT 0::bigint, NULL::bigint, \
           false, false, NULL::text $$;",
    )
    .await
    .unwrap();
    db::force_schema_replay(&c).await.unwrap();
    let healed = db::connect_and_load_schema(&base)
        .await
        .expect("loading the schema over the pre-R7 shape must not fail");
    let r = healed
        .query_one("SELECT * FROM cairn_duplicate_check_status()", &[])
        .await
        .unwrap();
    assert!(r
        .columns()
        .iter()
        .any(|col| col.name() == "charts_changing"));
}

/// `cairn-node duplicate-check` reads without replaying the schema (#726), so a binary upgraded
/// before its database's next replaying connect meets the pre-R7 shape. It must fail LEGIBLY,
/// naming the remedy — never panic on the missing `charts_changing` column (PR #788 review).
#[tokio::test]
async fn a_pre_r7_status_shape_read_without_replay_is_a_legible_error() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute(
        "DROP FUNCTION cairn_duplicate_check_status(); \
         CREATE FUNCTION cairn_duplicate_check_status(OUT charts_waiting bigint, \
           OUT quiet_age_s bigint, OUT config_recheck boolean, OUT worker_seen boolean, \
           OUT last_drained_hhmm text) LANGUAGE sql AS $$ SELECT 0::bigint, NULL::bigint, \
           false, false, NULL::text $$;",
    )
    .await
    .unwrap();
    // The CLI's own connect: no replay. Spawned, so a panic comes back as a JoinError AFTER the
    // restore below, never leaving the old shape for the next test.
    let plain = db::connect(&base).await.unwrap();
    let read = tokio::spawn(async move {
        cairn_node::duplicate_check::read_snapshot(&plain)
            .await
            .map_err(|e| format!("{e:#}"))
    })
    .await;
    db::force_schema_replay(&c).await.unwrap();
    db::connect_and_load_schema(&base).await.unwrap();
    let err = read
        .expect("reading an older status shape must not panic")
        .expect_err("an older status shape is an error");
    assert!(err.contains("charts_changing"), "{err}");
    assert!(err.contains("replay"), "names the remedy: {err}");
}
