//! DB-gated, #875 option 2: a command that holds the window's connection past
//! [`NodeConnection::ANSWER_WITHIN`] is cut off, so a link that died silently can no longer hang
//! the window behind its ONE connection lock.
//!
//! A silently dead socket cannot be staged on one machine, so a slow statement (`pg_sleep`)
//! stands in for it: the watchdog cannot tell the two apart, and that is the honest limit of the
//! mechanism (a node alive but slower than the limit is cut off too; plan
//! `2026-10-30-875-window-answer-limit.md`). Each test sets a short limit so the suite stays fast.
mod common;

use cairn_gui_data::port::{DataError, PatientSearch};
use cairn_gui_live::error::data_error_from;
use cairn_gui_live::{name_cut_in, LiveData, NodeConnection};
use std::sync::Arc;
use std::time::Duration;

/// The limit these tests run with, in place of the window's 15 s.
const LIMIT: Duration = Duration::from_secs(1);

/// How long past the limit a cut-off may take to show: the abort and the closed client.
const SLACK: Duration = Duration::from_secs(5);

/// What a cut-off command's error says (#893), at `LIMIT`.
const CUT_TEXT: &str =
    "the command was not finished within 1 s, so this window cut the connection off";

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

/// The probe table, with a deferred constraint trigger that sleeps `trigger_sleep`, which
/// makes the COMMIT itself the slow statement. The test's own scratch table, so it is granted to
/// the window's test login here (and dropped at the end), never through db/062.
async fn slow_commit_table(reader: &tokio_postgres::Client) {
    reader
        .batch_execute(
            "DROP TABLE IF EXISTS answer_limit_probe;
             CREATE TABLE answer_limit_probe (id int);
             CREATE OR REPLACE FUNCTION answer_limit_slow() RETURNS trigger
               LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(3); RETURN NULL; END $$;
             CREATE CONSTRAINT TRIGGER answer_limit_slow_commit AFTER INSERT ON answer_limit_probe
               DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION answer_limit_slow();
             GRANT SELECT, INSERT ON answer_limit_probe TO cairn_window_probe;",
        )
        .await
        .expect("the probe table");
}

async fn probe_rows(db: &NodeConnection) -> i64 {
    db.lock()
        .await
        .expect("open")
        .query_one("SELECT count(*) FROM answer_limit_probe", &[])
        .await
        .expect("a count")
        .get(0)
}

/// A COMMIT cut off mid-flight is NOT KNOWN, never "failed" (`commit_failure` marks it), and the
/// re-read that settles it is FINAL (PR #892 review): the cut does not stop the server, so the
/// reopen waits until the cut-off backend is gone. Without that, a re-read on the fresh
/// connection says "not recorded", the commit lands a moment later, and the clerk who pressed
/// again on the strength of the re-read has made a second chart (#815's hazard).
#[tokio::test]
async fn a_commit_cut_off_is_not_known_and_its_re_read_is_final() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    // Watched FIRST: it ensures the window's login, which the scratch table is granted to.
    let db = watched(&cs).await;
    slow_commit_table(&reader).await;

    let committed = {
        let mut client = db.lock().await.expect("open");
        let tx = client.transaction().await.expect("a transaction");
        tx.execute("INSERT INTO answer_limit_probe VALUES (1)", &[])
            .await
            .expect("the insert");
        // As an orchestrator marks it, then as a call site explains it under the guard (#893).
        let committed = tx.commit().await.map_err(|fault| {
            cairn_node::db_diagnosis::commit_failure(uuid::Uuid::now_v7(), fault.into())
        });
        committed.map_err(|marked| client.explain(marked))
    };
    let marked = committed.expect_err("the commit is cut off before it answers");
    assert!(
        cairn_node::db_diagnosis::commit_outcome_unknown_event(&marked).is_some(),
        "a cut-off commit is not known: {marked:#}"
    );
    assert!(
        matches!(data_error_from(&marked), DataError::OutcomeUnknown(_)),
        "saying it was cut off does not change its class: {marked:#}"
    );
    assert!(
        cairn_gui_live::cause_of(&marked).contains(CUT_TEXT),
        "the outcome line's cause says the window cut it off: {marked:#}"
    );

    // The trigger sleeps 3 s from the commit; the cut came at 1 s. The re-read must already
    // say what the end state is: the same count once the trigger's sleep would have ended.
    let re_read = probe_rows(&db).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(probe_rows(&db).await, re_read, "the re-read was final");
    reader
        .batch_execute("DROP TABLE answer_limit_probe; DROP FUNCTION answer_limit_slow();")
        .await
        .expect("the probe table dropped");
}

/// A reopened connection is watched as the first was: a second stuck hold is cut off too, and
/// reads in between stay on one backend (a reopen that kept the old client's cut mark would
/// reopen on every lock, and a fresh client with no watchdog of its own would hang again).
#[tokio::test]
async fn a_reopened_connection_is_watched_again() {
    let Some(cs) = common::cs() else { return };
    let (_reader, _guard) = common::connect(&cs).await;
    let db = watched(&cs).await;
    for _ in 0..2 {
        let before = backend_pid(&db).await;
        assert_eq!(backend_pid(&db).await, before, "one backend between cuts");
        let stuck = tokio::time::timeout(LIMIT + SLACK, async {
            db.lock()
                .await
                .expect("open")
                .execute("SELECT pg_sleep(10)", &[])
                .await
        })
        .await
        .expect("each stuck hold is cut off near the limit");
        assert!(stuck.expect_err("cut off").is_closed());
        assert_ne!(backend_pid(&db).await, before, "reopened");
    }
}

/// A press queued behind a stuck one is not charged for the wait: its own limit starts when it
/// gets the connection, so once the stuck hold is cut it runs on a fresh one and answers.
#[tokio::test(flavor = "multi_thread")]
async fn a_press_queued_behind_a_stuck_hold_gets_a_limit_of_its_own() {
    let Some(cs) = common::cs() else { return };
    let (_reader, _guard) = common::connect(&cs).await;
    let db = watched(&cs).await;
    let stuck = async {
        db.lock()
            .await
            .expect("open")
            .execute("SELECT pg_sleep(10)", &[])
            .await
    };
    let queued = async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        db.lock()
            .await
            .expect("reopened")
            .execute("SELECT pg_sleep(0.5)", &[])
            .await
    };
    let (stuck, queued) = tokio::join!(stuck, queued);
    assert!(stuck.expect_err("cut off").is_closed());
    queued.expect("the queued press answers on the fresh connection");
}

/// The HOLD is timed, not a query: a command that keeps the guard past the limit between
/// statements loses the connection too, and the next lock opens a fresh one. (That a cut client
/// reads as lost before its task has died is pinned in `src/connection/tests.rs`, which can set
/// the mark without the abort.)
#[tokio::test]
async fn a_hold_kept_past_the_limit_between_statements_is_cut_off() {
    let Some(cs) = common::cs() else { return };
    let (_reader, _guard) = common::connect(&cs).await;
    let db = watched(&cs).await;
    let before = backend_pid(&db).await;

    {
        // Well past the limit: a single-thread runtime polls this test before the watchdog, so a
        // stall across both timers would drop the guard first (PR #892 review).
        let _held = db.lock().await.expect("open");
        tokio::time::sleep(LIMIT * 2).await;
    }

    assert_ne!(backend_pid(&db).await, before, "a fresh connection");
}

/// #893: a cut-off command's error says the WINDOW cut it off, not merely "connection closed"
/// (the text of a server restart too), and it stays an outage to retry.
#[tokio::test]
async fn a_cut_off_command_says_the_window_cut_it_off() {
    let Some(cs) = common::cs() else { return };
    let (_reader, _guard) = common::connect(&cs).await;
    let db = watched(&cs).await;

    let explained = {
        let client = db.lock().await.expect("open");
        let stuck = client.execute("SELECT pg_sleep(10)", &[]).await;
        let stuck = stuck.expect_err("cut off");
        // `name_cut_in` finds a cut in text by this ending: a real cut's own words end so.
        assert!(stuck.to_string().ends_with("connection closed"), "{stuck}");
        let worded = format!("reading the record of chart X: {stuck}");
        assert_eq!(
            name_cut_in(&worded, client.cut_off()),
            format!("{CUT_TEXT}: {worded}")
        );
        client.explain(stuck.into())
    };
    assert!(format!("{explained:#}").contains(CUT_TEXT), "{explained:#}");
    match data_error_from(&explained) {
        DataError::Unavailable(text) => assert!(text.contains(CUT_TEXT), "{text}"),
        other => panic!("a cut-off read is an outage to retry: {other:?}"),
    }

    // A hold that answered in time explains nothing.
    let client = db.lock().await.expect("reopened");
    let refused = client.execute("SELECT no_such_column", &[]).await;
    let plain = client.explain(refused.expect_err("an answered error").into());
    assert!(
        !format!("{plain:#}").contains("cut the connection off"),
        "{plain:#}"
    );
}

/// #893 at a call site: a patient search stuck behind a lock is cut off, and the error the
/// front door shows says so.
#[tokio::test]
async fn a_cut_off_search_says_the_window_cut_it_off() {
    let Some(cs) = common::cs() else { return };
    let (mut reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;
    let live = LiveData::sharing(
        Arc::new(watched(&cs).await),
        sk,
        &common::identity("gui-live-origin"),
    );

    let lock = reader.transaction().await.expect("a transaction");
    lock.batch_execute("LOCK TABLE patient_name IN ACCESS EXCLUSIVE MODE")
        .await
        .expect("the lock");
    let query = cairn_patient_search::SearchQuery::new("Stuck", None, &[]);
    let searched = live.search(&query, "2026-10-31").await;
    lock.rollback().await.expect("the lock released");

    match searched {
        Err(DataError::Unavailable(text)) => assert!(text.contains(CUT_TEXT), "{text}"),
        other => panic!("a cut-off search is an outage that says it was cut off: {other:?}"),
    }
}
