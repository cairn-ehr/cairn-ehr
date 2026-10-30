//! DB-gated, #875 option 2: a command that holds the window's connection past
//! [`NodeConnection::ANSWER_WITHIN`] is cut off, so a link that died silently can no longer hang
//! the window behind its ONE connection lock.
//!
//! A silently dead socket cannot be staged on one machine, so a slow statement (`pg_sleep`)
//! stands in for it: the watchdog cannot tell the two apart, and that is the honest limit of the
//! mechanism (a node alive but slower than the limit is cut off too; plan
//! `2026-10-30-875-window-answer-limit.md`). Each test sets a short limit so the suite stays fast.
mod common;

use cairn_gui_live::NodeConnection;
use std::time::Duration;

/// The limit these tests run with, in place of the window's 15 s.
const LIMIT: Duration = Duration::from_secs(1);

/// How long past the limit a cut-off may take to show: the abort and the closed client.
const SLACK: Duration = Duration::from_secs(5);

/// The window's kind of connection, watched with the short limit, reopening from `cs`.
async fn watched(cs: &str) -> NodeConnection {
    common::reopening(cs, cs.to_owned())
        .await
        .answer_within(LIMIT)
}

async fn backend_pid(db: &NodeConnection) -> i32 {
    db.lock()
        .await
        .expect("open")
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .expect("a read")
        .get(0)
}

/// The stuck statement fails as a closed connection soon after the limit, which releases the
/// lock, and the next lock reads on a NEW backend (#807's reopen takes over).
#[tokio::test]
async fn a_hold_past_the_limit_is_cut_off_and_the_next_lock_reopens() {
    let Some(cs) = common::cs() else { return };
    let (_reader, _guard) = common::connect(&cs).await;
    let db = watched(&cs).await;
    let before = backend_pid(&db).await;

    let stuck = tokio::time::timeout(LIMIT + SLACK, async {
        let client = db.lock().await.expect("open");
        client.execute("SELECT pg_sleep(10)", &[]).await
    })
    .await
    .expect("the watchdog must cut the statement off near the limit");
    let err = stuck.expect_err("a cut-off statement fails");
    assert!(err.is_closed(), "it fails as a closed connection: {err}");

    assert_ne!(backend_pid(&db).await, before, "the next lock reopened");
}

/// A command that gives the connection back in time is not touched, even once the limit has
/// passed since it started: the timer belongs to the hold, not to the connection.
#[tokio::test]
async fn a_hold_released_in_time_keeps_the_connection() {
    let Some(cs) = common::cs() else { return };
    let (_reader, _guard) = common::connect(&cs).await;
    let db = watched(&cs).await;
    let before = backend_pid(&db).await;

    db.lock()
        .await
        .expect("open")
        .execute("SELECT pg_sleep(0.2)", &[])
        .await
        .expect("a short statement answers");
    tokio::time::sleep(LIMIT * 2).await;

    assert_eq!(backend_pid(&db).await, before, "the same connection");
}

/// A COMMIT cut off mid-flight is NOT KNOWN, never "failed": `commit_failure` marks it, and the
/// row lands anyway once the server finishes. A deferred constraint trigger that sleeps makes
/// the COMMIT itself the slow statement.
#[tokio::test]
async fn a_commit_cut_off_is_not_known_and_may_land() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    reader
        .batch_execute(
            "DROP TABLE IF EXISTS answer_limit_probe;
             CREATE TABLE answer_limit_probe (id int);
             CREATE OR REPLACE FUNCTION answer_limit_slow() RETURNS trigger
               LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(3); RETURN NULL; END $$;
             CREATE CONSTRAINT TRIGGER answer_limit_slow_commit AFTER INSERT ON answer_limit_probe
               DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION answer_limit_slow();",
        )
        .await
        .expect("the probe table");
    let db = watched(&cs).await;

    let committed = {
        let mut client = db.lock().await.expect("open");
        let tx = client.transaction().await.expect("a transaction");
        tx.execute("INSERT INTO answer_limit_probe VALUES (1)", &[])
            .await
            .expect("the insert");
        tx.commit().await
    };
    let fault = committed.expect_err("the commit is cut off before it answers");
    let marked = cairn_node::db_diagnosis::commit_failure(uuid::Uuid::now_v7(), fault.into());
    assert!(
        cairn_node::db_diagnosis::commit_outcome_unknown_event(&marked).is_some(),
        "a cut-off commit is not known: {marked:#}"
    );

    // The server never saw the hang-up in time to stop the commit: the row is there.
    let deadline = tokio::time::Instant::now() + SLACK;
    loop {
        let landed: i64 = reader
            .query_one("SELECT count(*) FROM answer_limit_probe", &[])
            .await
            .expect("a count")
            .get(0);
        if landed == 1 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the commit never landed"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    reader
        .batch_execute("DROP TABLE answer_limit_probe; DROP FUNCTION answer_limit_slow();")
        .await
        .expect("the probe table dropped");
}
