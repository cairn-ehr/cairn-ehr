//! #912 + #1175 — re-adjudicating deferred events from a long-running process.
//!
//! # Why this exists
//!
//! `cairn_readjudicate_deferred()` (db/043, ADR-0056 decision 4) promotes an event a peer's door
//! admitted uninterpreted once what it waited for has arrived. It used to run only inside the
//! schema loader, so on a running node a deferred event stayed powerless until an operator
//! happened to run a loader verb (#912). The fix runs it from `cairn-sync run`'s pull loop — the
//! process through which deferred events arrive — and that daemon may connect as the
//! UNPRIVILEGED runtime role (a `cairn_node` member), not the owner. So the pass became a SECURITY
//! DEFINER granted to `cairn_node`, the same tier as `apply_remote_event` (which takes arbitrary
//! caller bytes; this pass takes none).
//!
//! Two things that change must hold:
//!
//! * the runtime role can run the pass, and the lenient-tier marker it raises
//!   (`cairn.remote_apply`) does NOT survive into the caller's transaction — otherwise a runtime
//!   caller could run the pass and then author on the lenient tier (#602's hole, reopened);
//! * a promotion that waits too long for a lock (#1175) fails the WHOLE pass with `55P03`, so the
//!   caller retries later, instead of being recorded as the event's `adjudication_error` — which
//!   read as a refusal and left the loader's retry nothing to see.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.

mod common;
#[path = "common/late_custody_kit.rs"]
mod late_custody_kit;

use cairn_node::db;
use late_custody_kit::*;
use tokio_postgres::Client;
use uuid::Uuid;

/// A login-less role holding ONLY `cairn_node`: what `provision-runtime-role` grants a daemon.
const RUNTIME_PROBE: &str = "cairn_912_runtime_probe";

/// A deferred sealed assert held WITHOUT its key — promotable (its type is classified) and,
/// being sealed, the pass takes its custody lock first, which is the lock the #1175 test holds.
async fn deferred_sealed(c: &Client, keys: &Keys) -> SealedAssert {
    let e = sealed_assert(
        keys,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        "amoxicillin",
        WALL,
    );
    apply_without_key(c, &e)
        .await
        .expect("premise: admitted without custody");
    c.execute(
        "INSERT INTO event_deferred (event_id, event_type) \
         VALUES ($1::text::uuid, 'clinical.medication.asserted')",
        &[&e.event_id.to_string()],
    )
    .await
    .expect("mark it deferred, as an event awaiting re-adjudication is");
    e
}

/// `(marker rows, recorded adjudication_error)` for one event.
async fn marker(c: &Client, event_id: Uuid) -> (i64, Option<String>) {
    let row = c
        .query_one(
            "SELECT count(*), max(adjudication_error) FROM event_deferred \
              WHERE event_id = $1::text::uuid",
            &[&event_id.to_string()],
        )
        .await
        .unwrap();
    (row.get(0), row.get(1))
}

/// The runtime role runs the pass and promotes; afterwards, in the SAME transaction, the
/// lenient-tier marker is not 'on'.
#[tokio::test]
async fn the_runtime_role_runs_the_pass_and_the_lenient_marker_does_not_leak() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;
    let e = deferred_sealed(&c, &keys).await;
    c.batch_execute(&format!(
        "DO $$ BEGIN \
           IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{RUNTIME_PROBE}') THEN \
             CREATE ROLE {RUNTIME_PROBE} NOLOGIN; \
           END IF; \
         END $$; \
         GRANT cairn_node TO {RUNTIME_PROBE};"
    ))
    .await
    .unwrap();

    // SET ROLE in its own statement, BEFORE the BEGIN (the SET-then-BEGIN-in-one-batch trap).
    c.batch_execute(&format!("SET ROLE {RUNTIME_PROBE}"))
        .await
        .unwrap();
    let who: String = c
        .query_one("SELECT current_user::text", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(who, RUNTIME_PROBE, "premise: running as the runtime role");
    c.batch_execute("BEGIN").await.unwrap();
    let ran = c
        .query(
            "SELECT promoted_count FROM cairn_readjudicate_deferred()",
            &[],
        )
        .await;
    let rows = match ran {
        Ok(rows) => rows,
        Err(e) => {
            c.batch_execute("ROLLBACK").await.unwrap();
            c.batch_execute("RESET ROLE").await.unwrap();
            panic!(
                "THE RUNTIME ROLE MUST BE ABLE TO RUN THE PASS: `cairn-sync run` connects as it, \
                 and a pass it cannot run leaves every deferred event powerless on a running \
                 node (#912): {e:?}"
            );
        }
    };
    // The pass succeeded, so this transaction is healthy and the probe must answer.
    let leaked: Option<String> = c
        .query_one("SELECT current_setting('cairn.remote_apply', true)", &[])
        .await
        .expect("the leak probe runs in the pass's (healthy) transaction")
        .get(0);
    c.batch_execute("COMMIT").await.unwrap();
    c.batch_execute("RESET ROLE").await.unwrap();

    let promoted: i64 = rows.iter().map(|r| r.get::<_, i64>(0)).sum();
    assert_eq!(promoted, 1, "the deferred event is promoted");
    assert_eq!(
        leaked.as_deref().unwrap_or(""),
        "",
        "THE LENIENT MARKER MUST NOT OUTLIVE THE PASS: a runtime caller could otherwise author \
         on the lenient tier in the same transaction (#602)"
    );
    assert_eq!(marker(&c, e.event_id).await.0, 0, "the marker is gone");
}

/// #1175: a promotion that cannot get its lock within `lock_timeout` fails the pass with
/// `55P03`, and the event's marker records NO error (it was never adjudicated).
#[tokio::test]
async fn a_lock_wait_fails_the_pass_and_is_not_recorded_as_a_refusal() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;
    let e = deferred_sealed(&c, &keys).await;

    // Another transaction holds the event's custody lock (a late key landing, say).
    c.batch_execute("BEGIN").await.unwrap();
    c.execute(
        "SELECT cairn_lock_custody($1::text::uuid)",
        &[&e.event_id.to_string()],
    )
    .await
    .unwrap();

    let (c2, _) = second_connection(&base).await;
    c2.batch_execute("SET lock_timeout = '200ms'")
        .await
        .unwrap();
    let ran = c2
        .execute("SELECT count(*) FROM cairn_readjudicate_deferred()", &[])
        .await;
    c.batch_execute("ROLLBACK").await.unwrap();

    let err = ran.expect_err(
        "A LOCK WAIT MUST FAIL THE PASS (#1175): swallowed, the caller's retry never runs and the \
         event reads as refused",
    );
    assert_eq!(
        err.code(),
        Some(&tokio_postgres::error::SqlState::LOCK_NOT_AVAILABLE),
        "the pass fails with 55P03: {err:?}"
    );
    assert_eq!(
        marker(&c, e.event_id).await,
        (1, None),
        "still deferred, and NOT flagged: a lock wait is not an adjudication"
    );
}

/// Passes never overlap. Before #912 every pass ran under the loader's `SCHEMA_LOAD_LOCK`; now
/// `cairn-sync run` calls it too, outside that lock, and the gate-4 appliers were never written to
/// run concurrently over the same unsealed event (only sealed ones meet the custody lock). So the
/// pass takes its own transaction-scoped advisory lock first; a second caller waits for it, or
/// under a `lock_timeout` gives up with `55P03` and retries, like any other lock wait.
#[tokio::test]
async fn a_second_pass_waits_for_the_first() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Hold the pass's own lock, as a pass in flight on another connection does.
    c.batch_execute("BEGIN").await.unwrap();
    c.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended('cairn.readjudicate', 0))",
        &[],
    )
    .await
    .unwrap();

    let (c2, _) = second_connection(&base).await;
    c2.batch_execute("SET lock_timeout = '200ms'")
        .await
        .unwrap();
    let ran = c2
        .execute("SELECT count(*) FROM cairn_readjudicate_deferred()", &[])
        .await;
    c.batch_execute("ROLLBACK").await.unwrap();

    let err = ran.expect_err(
        "A PASS MUST WAIT FOR ONE ALREADY RUNNING: overlapping passes run the same appliers over \
         the same unsealed event at once",
    );
    assert_eq!(
        err.code(),
        Some(&tokio_postgres::error::SqlState::LOCK_NOT_AVAILABLE),
        "the pass waits on the shared lock and gives up with 55P03: {err:?}"
    );
}

/// The two replay-tier markers a caller can raise are pinned OFF on the definer, as
/// `submit_event` (db/005) pins both: `cairn.remote_apply` (above), and `cairn.reprojecting`,
/// which would let a runtime caller skip db/082's per-stop lock during a promotion and race a
/// strike (ADR-0105 decision 18). NOT every caller-settable setting: the size caps
/// (`cairn.max_component_size`, `cairn.max_medication_group_size`) stay caller-settable here
/// exactly as on both doors — #1196 decides that for all three definers together.
#[tokio::test]
async fn the_definer_pins_both_tier_markers_off() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let row = c
        .query_one(
            "SELECT prosecdef, coalesce(proconfig, '{}')::text[] FROM pg_proc \
              WHERE oid = 'cairn_readjudicate_deferred()'::regprocedure",
            &[],
        )
        .await
        .unwrap();
    let (definer, config): (bool, Vec<String>) = (row.get(0), row.get(1));
    assert!(definer, "the pass is a SECURITY DEFINER (#912)");
    for pin in ["cairn.remote_apply=", "cairn.reprojecting="] {
        assert!(
            config.iter().any(|c| c == pin),
            "THE DEFINER MUST PIN `{pin}` (empty): a caller who raised it changes what a \
             promotion does. proconfig = {config:?}"
        );
    }
}
