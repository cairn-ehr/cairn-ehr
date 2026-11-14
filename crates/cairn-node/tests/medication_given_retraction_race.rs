//! Charting slice 2, Task 3 fix round 1: a stop and the strike that voids it, applied at the SAME
//! time by two transactions (db/082 §2, finding a').
//!
//! A hub runs one `cairn-sync pull` per peer, so it can apply the giving terminal's stop while
//! another pull applies the striking terminal's retraction. Under READ COMMITTED neither sees the
//! other's uncommitted rows: without a lock the stop writes its winner row, the strike's delete
//! finds nothing, both commit, and a struck stat dose reads GIVEN. Both now take
//! `cairn_lock_medication_stop(stop)`, so the second waits for the first and then sees its rows.
//! Each test holds one side open on a second connection, starts the other, waits until that one
//! is blocked, commits, and checks the line reads current. Both orders.
//!
//! DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard` (inside `rig!`).
use cairn_event::EventBody;
use cairn_node::db;
use std::time::Duration;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;
#[path = "common/given_kit.rs"]
mod given_kit;
use given_kit::*;
#[macro_use]
#[path = "common/set_aside_kit.rs"]
mod set_aside_kit;
use set_aside_kit::*;

/// A peer's stop of the stat line `med`, and the peer's strike of a committed given voiding it.
/// The given is applied (committed) first, as it would have synced before either race.
async fn peer_stop_and_strike(
    c: &Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    p: Uuid,
    med: Uuid,
) -> (EventBody, EventBody) {
    let wall = db::next_hlc(c, "test-node").await.unwrap().wall + 1_000;
    let mut stop = stop_body(c, kid, p, med).await;
    stop.hlc = peer_hlc(wall);
    let stop_id: Uuid = stop.event_id.parse().unwrap();
    let mut g = given(c, kid, p, med, Some(stop_id)).await;
    g.hlc = peer_hlc(wall + 1);
    let g = effective_at_wall(c, g).await;
    apply_remote(c, sk, &g)
        .await
        .expect("the given syncs first");
    let mut r = retraction(c, kid, p, med, g.event_id.parse().unwrap(), Some(stop_id)).await;
    r.hlc = peer_hlc(wall + 2);
    (stop, r)
}

/// Hold `first` open on `c`, apply `second` on a fresh connection, wait until it blocks, commit.
async fn race(c: &Client, sk: &cairn_event::SigningKey, first: &EventBody, second: EventBody) {
    let c2 = db::connect(&cs().unwrap()).await.unwrap();
    let pid: i32 = c2
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    c.batch_execute("BEGIN").await.unwrap();
    apply_remote(c, sk, first)
        .await
        .expect("the first side applies");
    let sk2 = sk.clone();
    let second = tokio::spawn(async move { apply_remote(&c2, &sk2, &second).await.map(|_| ()) });
    // The second side must be WAITING on a lock before the first commits, or this proves nothing.
    let mut waited = false;
    for _ in 0..200 {
        let n: i64 = c
            .query_one(
                "SELECT count(*) FROM pg_stat_activity WHERE pid = $1 AND wait_event_type = 'Lock'",
                &[&pid],
            )
            .await
            .unwrap()
            .get(0);
        if n == 1 {
            waited = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(waited, "the second side never blocked on the first");
    c.batch_execute("COMMIT").await.unwrap();
    second
        .await
        .unwrap()
        .map_err(|e| db_msg(&e))
        .expect("the second side applies once the first commits");
}

const STAT_REASONS: &str = "SELECT count(*) FROM medication_cessation_reason \
     WHERE medication_id = $1::text::uuid AND reason = 'stat dose given'";

#[tokio::test]
async fn a_strike_applied_while_its_stop_is_uncommitted_still_voids_it() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, true).await;
    let (stop, strike) = peer_stop_and_strike(&c, &sk, &kid, p, med).await;
    race(&c, &sk, &stop, strike).await;
    assert!(
        is_current(&c, med).await,
        "the struck stat line reads current"
    );
    assert_eq!(count(&c, STAT_REASONS, med).await, 0);
}

#[tokio::test]
async fn a_stop_applied_while_its_strike_is_uncommitted_never_wins() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, true).await;
    let (stop, strike) = peer_stop_and_strike(&c, &sk, &kid, p, med).await;
    race(&c, &sk, &strike, stop).await;
    assert!(
        is_current(&c, med).await,
        "the struck stat line reads current"
    );
    assert_eq!(count(&c, STAT_REASONS, med).await, 0);
}

/// Whether this session holds `cairn_lock_medication_stop(stop)` (pg_locks splits the 64-bit
/// advisory key into classid:objid).
const HOLDS_STOP_LOCK: &str = "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' \
     AND pid = pg_backend_pid() AND ((classid::bigint << 32) | objid::bigint) = \
     hashtextextended('cairn.medication-stop:' || $1, 0)";

/// The per-stop lock is a DOOR lock: a stop applied through a door holds it to commit, a rebuild
/// (one transaction over every stop) holds none, and the replay marker is cleared after it.
#[tokio::test]
async fn a_rebuild_takes_no_per_stop_lock() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, true).await;
    c.batch_execute("BEGIN").await.unwrap();
    let stop = submit(&c, &sk, stop_body(&c, &kid, p, med).await)
        .await
        .unwrap();
    assert_eq!(
        count(&c, HOLDS_STOP_LOCK, stop).await,
        1,
        "the door holds it"
    );
    c.batch_execute("COMMIT").await.unwrap();
    c.batch_execute("BEGIN").await.unwrap();
    c.execute(
        "SELECT count(*) FROM cairn_reproject('clinical.medication', true, 'test')",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        count(&c, HOLDS_STOP_LOCK, stop).await,
        0,
        "the rebuild does not"
    );
    let marker: String = c
        .query_one("SELECT current_setting('cairn.reprojecting', true)", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(marker, "", "cleared for the rest of the transaction");
    c.batch_execute("COMMIT").await.unwrap();
    assert!(!is_current(&c, med).await, "the rebuild kept the stop");
}
