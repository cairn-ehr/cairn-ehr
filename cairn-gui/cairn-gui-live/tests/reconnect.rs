//! DB-gated, #807: the window's ONE connection is reopened after it is lost.
//!
//! The window opens one connection at launch and shares it between the chart commands and the
//! funnel ([`NodeConnection`]). Before #807 a Postgres restart, or any dropped connection, made
//! every later read and write fail until the window itself was restarted, and "Read this chart
//! again" sent the clinician to a button that could not succeed.
//!
//! The server-side backend is killed with `pg_terminate_backend`, the same FATAL a Postgres
//! restart or an administrator delivers.
mod common;

use cairn_gui_live::NodeConnection;
use std::time::Duration;

/// How long a test waits for the client to notice its backend died. The FATAL arrives on the
/// socket at once; the client's connection task needs a moment to read it and close.
const NOTICE_WITHIN: Duration = Duration::from_secs(5);

/// The backend pid the connection is talking to, or `None` if the read failed.
async fn backend_pid(db: &NodeConnection) -> Option<i32> {
    db.lock()
        .await
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .ok()
        .map(|row| row.get(0))
}

/// Kill the connection's backend from another session.
async fn terminate(reader: &tokio_postgres::Client, pid: i32) {
    reader
        .execute("SELECT pg_terminate_backend($1)", &[&pid])
        .await
        .expect("terminating a backend of our own role");
}

/// The window's case: after the backend dies, a later read succeeds on a NEW backend, with no
/// restart. One read may fail first, if it races the client noticing the death; that is the
/// press a clinician would repeat with "Read this chart again".
#[tokio::test]
async fn a_lost_connection_is_reopened_on_the_next_lock() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let db = NodeConnection::reopening(common::connect_for_live(&cs).await, cs.clone());
    let before = backend_pid(&db).await.expect("the first read works");

    terminate(&reader, before).await;

    let deadline = tokio::time::Instant::now() + NOTICE_WITHIN;
    let after = loop {
        if let Some(pid) = backend_pid(&db).await {
            break pid;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no read succeeded within {NOTICE_WITHIN:?} of the backend's death: the connection \
             was never reopened"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_ne!(after, before, "the read must come from a NEW backend");
}

/// A refusal is an ANSWER: the connection is alive, so it is kept. Reopening on a refusal would
/// throw away nothing useful today, but it would make every floor refusal cost a reconnect.
#[tokio::test]
async fn a_refusal_keeps_the_same_connection() {
    let Some(cs) = common::cs() else { return };
    let (_reader, _guard) = common::connect(&cs).await;
    let db = NodeConnection::reopening(common::connect_for_live(&cs).await, cs.clone());
    let before = backend_pid(&db).await.expect("the first read works");

    let refused = db.lock().await.execute("SELECT 1/0", &[]).await;
    assert!(refused.is_err(), "division by zero is refused");

    assert_eq!(backend_pid(&db).await, Some(before));
}

/// `LiveData::new`'s connection is never swapped: the caller made it, and may have set session
/// state on it (the lost-commit suite's kill switch is a SESSION setting) that a fresh
/// connection would silently lack.
#[tokio::test]
async fn a_fixed_connection_is_never_reopened() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let db = NodeConnection::fixed(common::connect_for_live(&cs).await);
    let before = backend_pid(&db).await.expect("the first read works");

    terminate(&reader, before).await;

    let deadline = tokio::time::Instant::now() + NOTICE_WITHIN;
    while !db.lock().await.is_closed() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the client never noticed its backend died"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(backend_pid(&db).await, None, "still the lost connection");
}
