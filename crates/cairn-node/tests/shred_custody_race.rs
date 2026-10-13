//! #604 — a crypto-shred and a late key for the SAME event, in two concurrent transactions.
//!
//! # The defect
//!
//! Both doors' step 9 grants custody only `IF NOT EXISTS (erasure_shred_log row for E)`, and
//! `cairn_execute_shred` (db/037) records that row and scrubs E's custody and projections. Under
//! READ COMMITTED neither side can see the other's uncommitted rows, and nothing made them wait for
//! each other, so two interleavings ended with a shred on record and the body still readable:
//!
//! 1. **The shred holds first.** T_shred records the shred but has not committed; T_key's check sees
//!    no shred row and writes `event_dek` + `event_clear` (and, since ADR-0070, the medication list
//!    row). T_shred already ran its deletes, so it scrubbed nothing of T_key's.
//! 2. **The key holds first.** T_key has written custody but not committed; T_shred's deletes cannot
//!    see those rows, delete nothing, and commit. T_key commits and the custody is back.
//!
//! ADR-0005 / ADR-0052 promise custody never comes back after a shred, in either arrival order. The
//! fix makes both sides take one per-event transaction lock (`cairn_lock_custody`, db/005) before
//! reading or scrubbing custody, so the second one to arrive waits and then reads committed state.
//!
//! # How the interleaving is forced
//!
//! The FIRST transaction is opened on one connection and left uncommitted. The SECOND runs on a
//! separate connection inside a spawned task. The test then waits until that task has either
//! FINISHED (the unfixed door: nothing makes it wait) or is provably PARKED on a lock
//! (`pg_stat_activity.wait_event_type = 'Lock'`, the fixed door) — a real signal, never a guessed
//! sleep — and only then commits the first. Both outcomes are deterministic, so the final
//! assertion is red before the fix and green after it, with no timing luck either way.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.

mod common;
#[path = "common/late_custody_kit.rs"]
mod late_custody_kit;

use cairn_event::{sign, Hlc};
use cairn_node::{db, shred::build_shred_body};
use late_custody_kit::*;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_postgres::Client;
use uuid::Uuid;

/// The signed bytes of a device-authored `erasure.shred.asserted` naming `e` as its target, filed
/// on `e`'s own chart — the shape `cairn-node shred` produces without `--attest-as`.
fn shred_of(keys: &Keys, e: &SealedAssert) -> Vec<u8> {
    let body = build_shred_body(
        Uuid::now_v7(),
        e.patient,
        e.event_id,
        "retention ceiling",
        &keys.kid_device,
        None,
        Hlc {
            wall: WALL + 5,
            counter: 0,
            node_origin: "peer".into(),
        },
    );
    sign(&body, &keys.sk_device)
        .expect("sign the tombstone")
        .signed_bytes
}

/// `(event_dek rows, event_clear rows)` for `event_id`: custody, and the readable body it opens.
async fn custody(c: &Client, event_id: Uuid) -> (i64, i64) {
    let row = c
        .query_one(
            "SELECT (SELECT count(*) FROM event_dek   WHERE event_id = $1::text::uuid), \
                    (SELECT count(*) FROM event_clear WHERE event_id = $1::text::uuid)",
            &[&event_id.to_string()],
        )
        .await
        .unwrap();
    (row.get(0), row.get(1))
}

/// Wait until `second` has either finished or is parked on a lock, observed through `observer`
/// (any connection other than the second's own). Bounded at 10 s so a door that hangs for some
/// unrelated reason fails with a message instead of stalling the suite.
///
/// Returns nothing: BOTH outcomes are legitimate here. An unfixed door finishes without waiting;
/// a fixed one parks. What the test asserts is the end state after the first transaction commits.
async fn until_finished_or_parked(
    observer: &Client,
    second_pid: i32,
    second: &JoinHandle<Result<u64, tokio_postgres::Error>>,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if second.is_finished() {
            return;
        }
        let wait: Option<String> = observer
            .query_one(
                "SELECT wait_event_type FROM pg_stat_activity WHERE pid = $1",
                &[&second_pid],
            )
            .await
            .unwrap()
            .get(0);
        if wait.as_deref() == Some("Lock") {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the second transaction (pid {second_pid}) neither finished nor parked on a lock \
             within 10 s (last wait_event_type = {wait:?})"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A second connection to the same database, and its backend pid.
async fn second_connection(base: &str) -> (Client, i32) {
    let c2 = db::connect(base).await.unwrap();
    let pid: i32 = c2
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    (c2, pid)
}

/// INTERLEAVING 1 — the shred is recorded but uncommitted when the key arrives through `door`
/// (the SQL that hands the door E's bytes and DEK). The key must wait for the shred and then find
/// it, so custody is never granted. Shared by the sync-door and strict-door tests below, because
/// each door takes its own lock in its own step 9 and a mutation dropping either must go red.
async fn a_key_racing_an_uncommitted_shred(door: &'static str) {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;

    let e = sealed_assert(
        &keys,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        "amoxicillin",
        WALL,
    );
    apply_without_key(&c, &e)
        .await
        .expect("premise: admitted without custody");

    // T_shred: open, shred E, and hold the transaction open.
    c.batch_execute("BEGIN").await.unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&shred_of(&keys, &e)])
        .await
        .expect("the shred is admitted");

    // T_key: E's key, on its own connection, while T_shred is still open.
    let (c2, pid2) = second_connection(&base).await;
    let signed = e.signed.clone();
    let dek = e.dek.clone();
    let key_landing = tokio::spawn(async move { c2.execute(door, &[&signed, &dek]).await });
    let (observer, _) = second_connection(&base).await;
    until_finished_or_parked(&observer, pid2, &key_landing).await;

    c.batch_execute("COMMIT").await.unwrap();
    key_landing
        .await
        .unwrap()
        .expect("the key's event is admitted (set-union), custody or not");

    assert_eq!(
        custody(&c, e.event_id).await,
        (0, 0),
        "A SHRED ON RECORD MEANS NO CUSTODY ({door}): the key raced an uncommitted shred and \
         must have waited for it, then found it — instead it wrote event_dek/event_clear the \
         shred could no longer see (#604)"
    );
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        0,
        "and the late-custody landing (ADR-0070) must not have put the shredded record back on \
         the medication list ({door})"
    );
}

/// Interleaving 1 at the SYNC door (db/020's step 9 lock).
#[tokio::test]
async fn a_key_racing_an_uncommitted_shred_is_never_granted_custody() {
    a_key_racing_an_uncommitted_shred("SELECT apply_remote_event($1, NULL, NULL, $2)").await;
}

/// Interleaving 1 at the STRICT door (db/005's step 9 lock). The chart already holds E (keyless),
/// so #345's registration-precedence rule does not refuse this re-submit.
#[tokio::test]
async fn a_key_racing_an_uncommitted_shred_at_the_strict_door_is_never_granted_custody() {
    a_key_racing_an_uncommitted_shred("SELECT submit_event($1, NULL, NULL, $2)").await;
}

/// INTERLEAVING 2 — the key has landed but is uncommitted when the shred arrives. The shred must
/// wait for it and then scrub what it wrote, so the committed key does not resurrect custody.
#[tokio::test]
async fn a_shred_racing_an_uncommitted_key_still_scrubs_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;

    let e = sealed_assert(
        &keys,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        "amoxicillin",
        WALL,
    );
    apply_without_key(&c, &e)
        .await
        .expect("premise: admitted without custody");

    // T_key: E's key lands (custody + the late-custody projection) and the transaction stays open.
    c.batch_execute("BEGIN").await.unwrap();
    apply_with_key(&c, &e).await.expect("the key is admitted");
    assert_eq!(
        custody(&c, e.event_id).await,
        (1, 1),
        "premise: inside T_key, custody is written"
    );

    // T_shred, on its own connection, while T_key is still open.
    let (c2, pid2) = second_connection(&base).await;
    let shred = shred_of(&keys, &e);
    let shredding =
        tokio::spawn(async move { c2.execute("SELECT apply_remote_event($1)", &[&shred]).await });
    let (observer, _) = second_connection(&base).await;
    until_finished_or_parked(&observer, pid2, &shredding).await;

    c.batch_execute("COMMIT").await.unwrap();
    shredding
        .await
        .unwrap()
        .expect("the shred is admitted and executes");

    assert_eq!(
        custody(&c, e.event_id).await,
        (0, 0),
        "A SHRED ON RECORD MEANS NO CUSTODY: the shred raced an uncommitted key and must have \
         waited for it, then scrubbed it — instead its deletes ran before the key committed and \
         the custody came back (#604)"
    );
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        0,
        "and the medication list row the late key projected is scrubbed too"
    );
}
