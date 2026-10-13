//! #560 — the local-state export's coverage figure must never outrun the custody it carries.
//!
//! # The defect
//!
//! `backup` records `export_covers_seq`: "every clinical event up to this seq has its key in the
//! export". `verify-backup` then calls the kit `Restorable` when that figure reaches the medium's.
//! The figure was `MAX(event_log.seq)`, read just before the export's custody read. But `seq` is an
//! IDENTITY value handed out at INSERT, so commits land out of order:
//!
//! 1. T10 inserts seq 10 (and its `event_dek` row) and has not committed.
//! 2. T11 inserts seq 11 and commits.
//! 3. The export reads `MAX(seq)` = 11, then reads custody — T10's DEK is invisible.
//! 4. T10 commits.
//!
//! The export claims 11 over a bundle without seq 10's key: a false green, and seq 10's sealed body
//! restores as ciphertext. Reading both under ONE snapshot (the fix the issue first proposed) does
//! not close it — T10 is just as invisible inside that snapshot. What closes it is WAITING: after
//! reading `MAX(seq)`, wait out every transaction then writing `event_log` (any holder of a seq at
//! or below it must be one of them), and only then read custody.
//!
//! # How the interleaving is forced
//!
//! A probe applier, registered for the medication type, parks T10 INSIDE the sync door on an
//! advisory lock this test holds. The door writes custody (step 9) and the log row before its
//! appliers run, so T10 is parked holding seq 10 and its DEK, uncommitted — exactly step 1. T11
//! then applies and commits. The coverage read starts while T10 is still parked: an unfixed read
//! finishes at once (red), a fixed one cannot finish until T10 does.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.

mod common;
#[path = "common/late_custody_kit.rs"]
mod late_custody_kit;

use cairn_node::db;
use cairn_node::localstate::{episode_dek_from_cbor, read_local_state_with_coverage};
use late_custody_kit::*;
use std::time::Duration;
use tokio_postgres::Client;
use uuid::Uuid;

/// The advisory-lock key the probe parks on. Arbitrary, and distinct from every product lock key
/// (those are derived from event ids or named constants such as CARNLK).
const PARK_KEY: i64 = 560_560_560;

/// Remove the parking probe, if present. Called at test START (the #583 reset-at-start rule) and
/// before asserting, so a failure never leaves a row in the projection registry
/// `projection_registry.rs` pins at an exact count.
async fn remove_parking_probe(c: &Client) {
    c.batch_execute(
        "DELETE FROM cairn_projection_apply WHERE apply_fn = 'cairn_test_park_one_event'; \
         DROP FUNCTION IF EXISTS cairn_test_park_one_event(event_log); \
         DROP TABLE IF EXISTS cairn_test_park_runs;",
    )
    .await
    .unwrap();
}

/// Register an applier that parks on [`PARK_KEY`] when — and only when — it runs over `parked`.
/// Every other event passes straight through, so T11 is not held up by it.
async fn install_parking_probe(c: &Client, parked: Uuid) {
    remove_parking_probe(c).await;
    // The registry's `projection_tables` check needs a real table to name.
    c.batch_execute(&format!(
        "CREATE TABLE cairn_test_park_runs (event_id uuid NOT NULL); \
         CREATE FUNCTION cairn_test_park_one_event(e event_log) RETURNS void LANGUAGE plpgsql AS \
           $$ BEGIN \
                IF e.event_id = '{parked}'::uuid THEN PERFORM pg_advisory_xact_lock({PARK_KEY}); END IF; \
              END $$;"
    ))
    .await
    .unwrap();
    c.execute(
        "INSERT INTO cairn_projection_apply \
           (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES \
           ('clinical.medication.asserted', 'cairn_test_park_one_event', \
            ARRAY['cairn_test_park_runs'], 900, TRUE)",
        &[],
    )
    .await
    .unwrap();
}

/// The seq `event_log` gave `event_id`.
async fn seq_of(c: &Client, event_id: Uuid) -> i64 {
    c.query_one(
        "SELECT seq FROM event_log WHERE event_id = $1::text::uuid",
        &[&event_id.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// The scene: T10 parked inside the door holding the lower seq and its DEK, T11 committed above it.
/// Returns the parked event, T10's task, the connection holding the park lock, and T11's event.
struct Scene {
    parked: SealedAssert,
    t10: tokio::task::JoinHandle<Result<u64, tokio_postgres::Error>>,
    park_holder: Client,
    later: SealedAssert,
}

async fn t10_parked_below_a_committed_t11(base: &str, c: &Client) -> Scene {
    let keys = fresh_node(c).await;
    remove_probe(c).await;

    let parked = sealed_assert(
        &keys,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        "amoxicillin",
        WALL,
    );
    let later = sealed_assert(
        &keys,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        "metformin",
        WALL + 1,
    );
    install_parking_probe(c, parked.event_id).await;

    // Hold the park lock BEFORE T10 starts, on a connection of its own.
    let (park_holder, _) = second_connection(base).await;
    park_holder
        .execute("SELECT pg_advisory_lock($1)", &[&PARK_KEY])
        .await
        .unwrap();

    // T10: the sync door with custody, on its own connection — it takes its seq, writes its DEK,
    // and parks in the probe.
    let (c10, pid10) = second_connection(base).await;
    let (signed, dek) = (parked.signed.clone(), parked.dek.clone());
    let t10 = tokio::spawn(async move {
        c10.execute(
            "SELECT apply_remote_event($1, NULL, NULL, $2)",
            &[&signed, &dek],
        )
        .await
    });
    let (observer, _) = second_connection(base).await;
    until_finished_or_parked(&observer, pid10, &t10).await;
    assert!(
        !t10.is_finished(),
        "premise: T10 must be parked inside the door, not finished"
    );

    // T11: applies and commits above T10's seq.
    apply_with_key(c, &later)
        .await
        .expect("premise: T11 is admitted while T10 is parked");

    Scene {
        parked,
        t10,
        park_holder,
        later,
    }
}

/// Let T10 finish and confirm the premise that it holds the LOWER seq.
async fn release_t10(c: &Client, scene: Scene) -> (SealedAssert, SealedAssert) {
    scene
        .park_holder
        .execute("SELECT pg_advisory_unlock($1)", &[&PARK_KEY])
        .await
        .unwrap();
    scene
        .t10
        .await
        .unwrap()
        .expect("T10 is admitted once released");
    assert!(
        seq_of(c, scene.parked.event_id).await < seq_of(c, scene.later.event_id).await,
        "premise: the parked event holds the lower seq and committed second"
    );
    (scene.parked, scene.later)
}

/// Whether the export bundle carries `event_id`'s DEK.
fn carries_dek_of(state: &cairn_node::localstate::LocalState, event_id: Uuid) -> bool {
    state
        .episode_deks()
        .iter()
        .map(|b| episode_dek_from_cbor(b).expect("a well-formed episode DEK"))
        .any(|d| d.event_id == event_id.to_string())
}

/// THE RACE. The coverage read starts with T10 parked below a committed T11. It must not finish
/// until T10 has — and once it does, the figure it reports must be backed by T10's key.
#[tokio::test]
async fn coverage_waits_for_a_lower_seq_still_in_flight_and_then_carries_its_key() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    remove_parking_probe(&c).await;
    let scene = t10_parked_below_a_committed_t11(&base, &c).await;

    let (reader, _) = second_connection(&base).await;
    let read = tokio::spawn(async move {
        read_local_state_with_coverage(&reader, None, Duration::from_secs(30)).await
    });
    // An unfixed read returns in milliseconds. Give it ample time to show that it did, then let
    // T10 go: a fixed read cannot have returned yet, because T10 still holds its seq.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !read.is_finished() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let finished_early = read.is_finished();
    let (parked, later) = release_t10(&c, scene).await;
    let (covers, state) = read.await.unwrap().expect("the read succeeds");
    remove_parking_probe(&c).await;

    let covers = covers.expect("T10 finished well inside the wait, so coverage is settled");
    assert!(
        covers >= seq_of(&c, later.event_id).await,
        "the settled figure still reaches the committed T11 (got {covers})"
    );
    assert!(
        !finished_early && carries_dek_of(&state, parked.event_id),
        "A COVERAGE FIGURE IS A PROMISE ABOUT THE KEYS (#560): the export claims seq {covers}, \
         which is above the parked event's seq, so it must carry that event's key. \
         finished_early = {finished_early} — the read did not wait for the in-flight lower seq"
    );
}

/// A writer that outlasts the wait leaves coverage UNSETTLED (`None`), never a guessed figure: the
/// export is still written, but its coverage must not move (`export_outcome_for_write`).
#[tokio::test]
async fn a_writer_outlasting_the_wait_leaves_coverage_unsettled() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    remove_parking_probe(&c).await;
    let scene = t10_parked_below_a_committed_t11(&base, &c).await;

    let (reader, _) = second_connection(&base).await;
    let (covers, _) = read_local_state_with_coverage(&reader, None, Duration::from_millis(300))
        .await
        .expect("an unsettled read is an answer, not an error");
    release_t10(&c, scene).await;

    // Positive control: with nothing in flight the same read settles at once, at MAX(seq).
    let (settled, _) = read_local_state_with_coverage(&reader, None, Duration::from_millis(300))
        .await
        .unwrap();
    let max: i64 = c
        .query_one("SELECT MAX(seq) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    remove_parking_probe(&c).await;

    assert_eq!(
        covers, None,
        "with a lower seq still in flight after the wait, coverage must be withheld"
    );
    assert_eq!(
        settled,
        Some(max),
        "and with nothing in flight it settles at the log's high-water mark"
    );
}
