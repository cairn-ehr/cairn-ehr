//! DB-gated, #807: the window's ONE connection is reopened after it is lost.
//!
//! The window opens one connection at launch and shares it between the chart commands and the
//! funnel ([`NodeConnection`]). Before #807 a Postgres restart, or any dropped connection, made
//! every later read and write fail until the window itself was restarted, and "Read this chart
//! again" sent the clinician to a button that could not succeed.
//!
//! The server-side backend is killed with `pg_terminate_backend`, the same FATAL a Postgres
//! restart or an administrator delivers. A node that is still DOWN is a local TCP listener
//! standing where the reopen connects ([`unreachable_node`]).
mod common;

use cairn_gui_data::port::DataError;
use cairn_gui_live::{NodeConnection, Refusal};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_postgres::Client;

/// How long a test waits for the client to notice its backend died. The FATAL arrives on the
/// socket at once; the client's connection task needs a moment to read it and close.
const NOTICE_WITHIN: Duration = Duration::from_secs(5);

/// The backend pid the connection is talking to, or `None` if the read failed.
async fn backend_pid(db: &NodeConnection) -> Option<i32> {
    db.lock()
        .await
        .ok()?
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .ok()
        .map(|row| row.get(0))
}

/// Kill the connection's backend from another session, and wait until it has exited: without
/// the wait, the next read could still be answered by the dying backend.
async fn terminate(reader: &Client, pid: i32) {
    let killed: bool = reader
        .query_one("SELECT pg_terminate_backend($1, 5000)", &[&pid])
        .await
        .expect("terminating a backend of our own role")
        .get(0);
    assert!(killed, "backend {pid} was not terminated");
}

/// Kill `db`'s backend, then lock until the client has noticed. A lock that notices also makes
/// the first reopen attempt (if `db` reopens at all), so this returns after that attempt: its
/// error if the reopen failed (#876), `None` for a closed client handed back unreopened. Only
/// for a `db` that CANNOT reopen (its node down, or `fixed`): a reopen that succeeds hands back
/// an open client, and this loops until it panics with "never noticed".
async fn lose(db: &NodeConnection, reader: &Client) -> Option<String> {
    let pid = backend_pid(db).await.expect("the first read works");
    terminate(reader, pid).await;
    let deadline = tokio::time::Instant::now() + NOTICE_WITHIN;
    loop {
        match db.lock().await {
            Err(lost) => return Some(lost.to_string()),
            Ok(client) if client.is_closed() => return None,
            Ok(_) => {}
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the client never noticed its backend died"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A node that is still down: a listener that counts the connections offered to it, then hangs
/// up at once (`hang_up`) or never says a word. Returns a connection string naming it.
async fn unreachable_node(hang_up: bool) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let port = listener.local_addr().expect("a bound port").port();
    let offered = Arc::new(AtomicUsize::new(0));
    let counter = offered.clone();
    tokio::spawn(async move {
        let mut silent = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            // Counted BEFORE the hang-up, so a reopen that failed has always been counted.
            counter.fetch_add(1, Ordering::SeqCst);
            if !hang_up {
                silent.push(socket);
            }
        }
    });
    (
        format!("host=127.0.0.1 port={port} user=nobody dbname=nowhere"),
        offered,
    )
}

/// Lose the connection of a `db` that reopens towards a node still down, then press five times
/// at once. Returns the connection and how many reopen attempts the node was offered.
///
/// #876: every press is an error that says WHY, never a closed client whose query can only say
/// "connection closed". The first names the failed attempt; the five inside the backoff say
/// when it failed, so a press nothing was tried for does not read like a fresh failure.
async fn lose_then_press_five_times(hang_up: bool) -> Option<(NodeConnection, Arc<AtomicUsize>)> {
    let cs = common::cs()?;
    let (reader, _guard) = common::connect(&cs).await;
    let (down, offered) = unreachable_node(hang_up).await;
    let db = NodeConnection::reopening(common::connect_for_live(&cs).await, down);
    let first = lose(&db, &reader)
        .await
        .expect("a reopen towards a node that is down fails");
    assert!(first.contains("could not be reopened"), "{first}");
    let cause = if hang_up {
        "connecting to the database"
    } else {
        "no answer within"
    };
    assert!(
        first.contains(cause),
        "the cause reaches the caller: {first}"
    );
    // A node that hangs up or never answers is an outage, never a refusal (#884 review): the
    // words of `Closed`, `Io` or `Timeout` in `client_refusal` would send for an operator here.
    assert!(!first.contains("until an operator acts"), "{first}");
    for _ in 0..5 {
        let Err(lost) = db.lock().await else {
            panic!("the node is down: nothing to reopen");
        };
        let text = lost.to_string();
        assert!(text.contains(" s ago"), "a press in the backoff: {text}");
        assert!(text.contains(cause), "{text}");
        assert!(
            matches!(DataError::from(lost), DataError::Unavailable(_)),
            "an outage offers the press: {text}"
        );
    }
    Some((db, offered))
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

/// A node that refuses at once: a burst of presses costs ONE attempt, not one per lock, and the
/// next attempt comes once the backoff is over.
#[tokio::test]
async fn a_failed_reopen_is_not_retried_until_the_backoff_ends() {
    let Some((db, offered)) = lose_then_press_five_times(true).await else {
        return;
    };
    assert_eq!(
        offered.load(Ordering::SeqCst),
        1,
        "one attempt for the burst"
    );

    tokio::time::sleep(NodeConnection::RETRY_AFTER).await;
    let again = db
        .lock()
        .await
        .err()
        .map(|e| e.to_string())
        .expect("still down");
    assert!(!again.contains(" s ago"), "this press tried: {again}");
    assert_eq!(
        offered.load(Ordering::SeqCst),
        2,
        "tried again after the backoff"
    );
}

/// A node that never answers: the attempt gives up after `REOPEN_WITHIN` instead of holding the
/// lock for a TCP timeout, and a give-up starts the backoff like any other failure.
#[tokio::test]
async fn a_reopen_that_gets_no_answer_gives_up() {
    let budget = NOTICE_WITHIN + NodeConnection::REOPEN_WITHIN + Duration::from_secs(5);
    let Some((_db, offered)) = tokio::time::timeout(budget, lose_then_press_five_times(false))
        .await
        .expect("a reopen that gets no answer must give up, once")
    else {
        return;
    };
    assert_eq!(
        offered.load(Ordering::SeqCst),
        1,
        "one attempt for the burst"
    );
}

/// Any error the server ANSWERS (here 22012; a floor refusal, P0001, is the same case) comes
/// from a live connection, so the connection is kept. Reopening on one would make every floor
/// refusal cost a reconnect.
#[tokio::test]
async fn an_answered_error_keeps_the_same_connection() {
    let Some(cs) = common::cs() else { return };
    let (_reader, _guard) = common::connect(&cs).await;
    let db = NodeConnection::reopening(common::connect_for_live(&cs).await, cs.clone());
    let before = backend_pid(&db).await.expect("the first read works");

    let refused = db
        .lock()
        .await
        .expect("open")
        .execute("SELECT 1/0", &[])
        .await;
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

    assert_eq!(
        lose(&db, &reader).await,
        None,
        "handed back closed, not an error"
    );

    assert_eq!(backend_pid(&db).await, None, "still the lost connection");
}

/// #876: gesture timing, written just after an act, takes the connection with `lock_if_open`.
/// It must not spend the reopen attempt (and start the backoff) that the act's re-read needs.
#[tokio::test]
async fn lock_if_open_never_spends_a_reopen_attempt() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (down, offered) = unreachable_node(true).await;
    let db = NodeConnection::reopening(common::connect_for_live(&cs).await, down);
    let pid = backend_pid(&db).await.expect("the first read works");
    terminate(&reader, pid).await;

    let deadline = tokio::time::Instant::now() + NOTICE_WITHIN;
    while db.lock_if_open().await.is_some() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the client never noticed its backend died"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(offered.load(Ordering::SeqCst), 0, "nothing was tried");

    assert!(db.lock().await.is_err(), "the node is still down");
    assert_eq!(offered.load(Ordering::SeqCst), 1, "the next lock tries");
}

/// #880: a node that ANSWERS and refuses the reopen (here a database that does not exist,
/// `3D000`, the same class of answer as a rotated password) is not an outage. Every press says
/// an operator must act, the press that tried and the one inside the backoff alike, so the
/// window never offers a retry that keeps failing. This is the end-to-end proof that the code
/// survives `db::connect` and is stored with the cause.
#[tokio::test]
async fn a_reopen_the_server_refuses_asks_for_an_operator_on_every_press() {
    let Some(cs) = common::cs() else { return };
    // The ` dbname=` append is keyword-form syntax (as in cairn-node's db_diagnosis.rs).
    if cs.starts_with("postgres://") || cs.starts_with("postgresql://") {
        eprintln!("skipped: CAIRN_TEST_PG is URI-form; this test appends keyword-form options");
        return;
    }
    let (reader, _guard) = common::connect(&cs).await;
    let refusing = format!("{cs} dbname=cairn_no_such_database_880");
    let db = NodeConnection::reopening(common::connect_for_live(&cs).await, refusing);
    let first = lose(&db, &reader)
        .await
        .expect("a reopen towards a database that does not exist fails");
    assert!(first.contains("[3D000]"), "{first}");
    assert!(first.contains("until an operator acts"), "{first}");

    let Err(lost) = db.lock().await else {
        panic!("inside the backoff: nothing is tried, so still lost");
    };
    match DataError::from(lost) {
        DataError::NotProvisioned(text) => {
            assert!(text.contains(" s ago"), "a press in the backoff: {text}");
            assert!(text.contains("until an operator acts"), "{text}");
        }
        other => panic!("a refused reopen offers no retry-now: {other:?}"),
    }
}

/// A node that asks for a password and then waits: it reads the startup packet and answers
/// `AuthenticationCleartextPassword`. The same fake as cairn-node's `db_diagnosis.rs` (another
/// Cargo tree, so not shared). Returns a connection string naming it, with no password.
async fn node_asking_for_a_password() -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let port = listener.local_addr().expect("a bound port").port();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Ok(len) = socket.read_i32().await else {
                    return;
                };
                let mut rest = vec![0; usize::try_from(len - 4).unwrap_or(0)];
                if socket.read_exact(&mut rest).await.is_err() {
                    return;
                }
                // 'R', the length of what follows it (8, itself included), then 3: a password.
                let mut ask = vec![b'R'];
                ask.extend_from_slice(&8i32.to_be_bytes());
                ask.extend_from_slice(&3i32.to_be_bytes());
                let _ = socket.write_all(&ask).await;
                let _ = socket.read(&mut [0; 1]).await;
            });
        }
    });
    format!("host=127.0.0.1 port={port} user=nobody dbname=nowhere")
}

/// #884: a refusal tokio-postgres raises itself (here "password missing": the role was given a
/// password and `pg_hba` moved to scram, but the window's connection string has none) carries
/// no SQLSTATE. It must still ask for an operator, never offer a press that keeps failing.
#[tokio::test]
async fn a_reopen_the_client_cannot_authenticate_asks_for_an_operator() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let asking = node_asking_for_a_password().await;
    let db = NodeConnection::reopening(common::connect_for_live(&cs).await, asking);
    let first = lose(&db, &reader)
        .await
        .expect("a reopen without the password the node asks for fails");
    assert!(first.contains("password missing"), "{first}");
    assert!(first.contains("until an operator acts"), "{first}");
    // The node asked and the WINDOW could not answer: no rejected login is in the node's log.
    assert!(!first.contains("refused the connection"), "{first}");
    assert!(first.contains(Refusal::Client.clause()), "{first}");
    assert!(!first.contains("press again in"), "{first}");

    let Err(lost) = db.lock().await else {
        panic!("inside the backoff: nothing is tried, so still lost");
    };
    assert!(
        matches!(DataError::from(lost), DataError::NotProvisioned(_)),
        "a client-side refusal offers no retry-now"
    );
}
