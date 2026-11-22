//! #909 slice B, Task 7 — the terminal gateway end to end: admission, revoke, the plain-text and
//! flood limits, and the gateway's own database going away. The rig (a real gateway reached by
//! the window's real pinned link) is shared with `terminal_gateway_e2e_refusals.rs`; see
//! `common/gateway_rig.rs`.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`. Keys are generated at
//! runtime (house rule 6).
mod common;
#[path = "common/gateway_rig.rs"]
mod gateway_rig;

use std::time::{Duration, Instant};

use cairn_event::generate_key;
use cairn_node::terminal_service::gateway::admit::REFUSED;
use gateway_rig::{failure, rig, select_one, Rig, GATEWAY_LOGIN};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_postgres::Client;
async fn pid_of(c: &Client) -> i32 {
    c.query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0)
}

/// `n` raw TCP connections that send nothing (half-open sockets from a LAN host).
async fn idle_sockets(addr: &str, n: usize) -> Vec<TcpStream> {
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(TcpStream::connect(addr).await.unwrap());
    }
    // Let the gateway accept them all before the next connection queues behind them.
    tokio::time::sleep(Duration::from_millis(300)).await;
    v
}

/// Run `pg_sleep(30)` on `c` and return once it is really running in backend `pid`.
async fn start_sleep(
    r: &gateway_rig::Rig,
    c: Client,
    pid: i32,
) -> tokio::task::JoinHandle<Result<tokio_postgres::Row, tokio_postgres::Error>> {
    let sleeping = tokio::spawn(async move { c.query_one("SELECT pg_sleep(30)", &[]).await });
    let deadline = Instant::now() + Duration::from_secs(5);
    while r
        .owner
        .query_opt(
            "SELECT 1 FROM pg_stat_activity WHERE pid = $1 AND state = 'active' \
               AND query LIKE '%pg_sleep(30)%'",
            &[&pid],
        )
        .await
        .unwrap()
        .is_none()
    {
        assert!(Instant::now() < deadline, "the sleep never started");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    sleeping
}

/// Read until the peer closes; `Ok(n)` is how many bytes came first.
async fn read_to_close(s: &mut TcpStream) -> std::io::Result<usize> {
    let mut all = Vec::new();
    s.read_to_end(&mut all).await.map(|_| all.len())
}

#[tokio::test]
async fn an_admitted_window_login_reads_through_the_gateway() {
    let Some(r) = rig().await else { return };
    let (t, _) = r.admitted_terminal().await;
    let c = r.window(&t).await;
    assert_eq!(select_one(&c).await, 1);
    r.finish().await;
}

#[tokio::test]
async fn an_unadmitted_terminal_is_refused_and_recorded_pending() {
    let Some(r) = rig().await else { return };
    let (t, kid) = generate_key().unwrap();
    // Wait for the gateway's first load, so the refusal is "not admitted", not "not loaded".
    let (admitted, _) = r.admitted_terminal().await;
    drop(r.window(&admitted).await);
    let Err(err) = r
        .link(&t)
        .connect_with_task(&r.conn("cairn_window_probe"))
        .await
    else {
        panic!("an unadmitted terminal is refused");
    };
    let (text, refused, sqlstate) = failure(err);
    assert!(text.contains("not admitted"), "{text}");
    assert!(
        refused,
        "a REFUSAL, not an outage the window retries: {text}"
    );
    assert_eq!(sqlstate, None);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let status: Option<String> = r
            .owner
            .query_opt(
                "SELECT status FROM terminal_trust WHERE terminal_pubkey = $1",
                &[&kid],
            )
            .await
            .unwrap()
            .map(|row| row.get(0));
        if status.as_deref() == Some("pending") {
            break;
        }
        assert!(Instant::now() < deadline, "never recorded: {status:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    r.finish().await;
}

#[tokio::test]
async fn a_revoke_ends_the_open_session_within_2_s() {
    let Some(r) = rig().await else { return };
    let (t, kid) = r.admitted_terminal().await;
    let c = r.window(&t).await;
    let pid = pid_of(&c).await;
    let sleeping = start_sleep(&r, c, pid).await;
    r.revoke(&kid).await;
    let revoked = Instant::now();
    let ended = tokio::time::timeout(Duration::from_secs(2), sleeping)
        .await
        .expect("the window's statement ends within 2 s of the revoke")
        .unwrap();
    assert!(ended.is_err(), "the statement errors, it does not finish");
    while r.backend_alive(pid).await {
        assert!(
            revoked.elapsed() < Duration::from_secs(5),
            "the backend is still running its statement 5 s after the revoke (no cancel?)"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    r.finish().await;
}

/// A run that ends (a panic, which the supervisor restarts) must take its sessions with it: the
/// next run's sweep never sees them, so a later revoke could not end them (final review I1).
#[tokio::test]
async fn a_session_ends_when_its_gateway_run_ends() {
    let Some(r) = rig().await else { return };
    let (t, _) = r.admitted_terminal().await;
    let c = r.window(&t).await;
    let pid = pid_of(&c).await;
    r.end_run();
    let ended = Instant::now();
    while r.backend_alive(pid).await {
        assert!(
            ended.elapsed() < Duration::from_secs(2),
            "the session outlived its gateway run by 2 s"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(c.query_one("SELECT 1", &[]).await.is_err());
    r.finish().await;
}

/// A statement running when the run ends is cancelled too, or its backend (and its locks) would
/// outlive the session until it next writes, beyond any later revoke (final review R1).
#[tokio::test]
async fn a_running_statement_ends_when_its_gateway_run_ends() {
    let Some(r) = rig().await else { return };
    let (t, _) = r.admitted_terminal().await;
    let c = r.window(&t).await;
    let pid = pid_of(&c).await;
    let _sleeping = start_sleep(&r, c, pid).await;
    r.end_run();
    let ended = Instant::now();
    while r.backend_alive(pid).await {
        assert!(
            ended.elapsed() < Duration::from_secs(5),
            "the backend still runs its statement 5 s after its gateway run ended (no cancel?)"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    r.finish().await;
}

#[tokio::test]
async fn a_revoke_ends_an_idle_in_transaction_session() {
    const LOCK: i64 = 0x0909_7E57;
    let Some(r) = rig().await else { return };
    let (t, kid) = r.admitted_terminal().await;
    let c = r.window(&t).await;
    c.batch_execute(&format!("BEGIN; SELECT pg_advisory_xact_lock({LOCK})"))
        .await
        .unwrap();
    let held: bool = r
        .owner
        .query_one("SELECT pg_try_advisory_lock($1)", &[&LOCK])
        .await
        .unwrap()
        .get(0);
    assert!(!held, "the window's open transaction holds the lock");
    r.revoke(&kid).await;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let got: bool = r
            .owner
            .query_one("SELECT pg_try_advisory_lock($1)", &[&LOCK])
            .await
            .unwrap()
            .get(0);
        if got {
            break;
        }
        assert!(Instant::now() < deadline, "the lock was never released");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    r.owner
        .execute("SELECT pg_advisory_unlock($1)", &[&LOCK])
        .await
        .unwrap();
    r.finish().await;
}

#[tokio::test]
async fn the_service_login_is_refused_with_28000_and_the_fixed_message() {
    let Some(r) = rig().await else { return };
    let (t, _) = r.admitted_terminal().await;
    drop(r.window(&t).await); // admitted: what follows is the startup verdict, not the TLS pin
    let Err(err) = r
        .link(&t)
        .connect_with_task(&r.conn("cairn_terminal_service_probe"))
        .await
    else {
        panic!("the service's login may not pass");
    };
    let (text, _, sqlstate) = failure(err);
    assert_eq!(sqlstate.as_deref(), Some("28000"), "{text}");
    assert!(text.contains(REFUSED), "{text}");
    assert!(
        !text.contains("cairn_terminal_service"),
        "names no role: {text}"
    );
    r.finish().await;
}

#[tokio::test]
async fn a_plain_text_startup_gets_no_reply() {
    let Some(r) = rig().await else { return };
    let mut s = TcpStream::connect(&r.addr).await.unwrap();
    // A protocol-3.0 StartupMessage in clear text: user=x.
    let body = b"\x00\x03\x00\x00user\x00x\x00\x00";
    let mut msg = ((body.len() + 4) as u32).to_be_bytes().to_vec();
    msg.extend_from_slice(body);
    s.write_all(&msg).await.unwrap();
    let n = tokio::time::timeout(Duration::from_secs(3), read_to_close(&mut s))
        .await
        .expect("closed, not left hanging")
        .expect("a clean close, not a reset");
    assert_eq!(n, 0, "no reply at all");
    r.finish().await;
}

/// Take the gateway's own database connection away and wait until its backend is gone. NOLOGIN
/// first, so the gateway cannot reconnect until [`gateway_back_online`].
async fn gateway_offline(r: &Rig) {
    r.owner
        .batch_execute(&format!(
            "ALTER ROLE {GATEWAY_LOGIN} NOLOGIN; \
             SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = '{GATEWAY_LOGIN}'"
        ))
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let n: i64 = r
            .owner
            .query_one(
                "SELECT count(*) FROM pg_stat_activity WHERE usename = $1",
                &[&GATEWAY_LOGIN],
            )
            .await
            .unwrap()
            .get(0);
        if n == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the gateway's backend never went"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn gateway_back_online(r: &Rig) {
    r.owner
        .batch_execute(&format!("ALTER ROLE {GATEWAY_LOGIN} LOGIN"))
        .await
        .unwrap();
}

#[tokio::test]
async fn startups_are_refused_while_the_gateways_db_connection_is_down() {
    let Some(r) = rig().await else { return };
    let (t, _) = r.admitted_terminal().await;
    drop(r.window(&t).await);
    gateway_offline(&r).await;
    let Err(err) = r
        .link(&t)
        .connect_with_task(&r.conn("cairn_window_probe"))
        .await
    else {
        panic!("refused while the gateway cannot check the login");
    };
    let (text, refused, sqlstate) = failure(err);
    assert_eq!(
        sqlstate.as_deref(),
        Some("57P03"),
        "a transient refusal: {text}"
    );
    assert!(
        !refused,
        "an outage the window retries, not 'not admitted': {text}"
    );
    gateway_back_online(&r).await;
    let c = r.window(&t).await; // retries until the gateway has reconnected
    assert_eq!(select_one(&c).await, 1);
    r.finish().await;
}

/// A revoke made while the gateway cannot read the database cannot be seen until it can. Until
/// then the last trust set is kept (an outage ends no session); the first sweep after the
/// gateway reconnects ends the revoked session (review T4).
#[tokio::test]
async fn a_revoke_made_during_the_gateways_outage_ends_the_session_once_it_reconnects() {
    let Some(r) = rig().await else { return };
    let (t, kid) = r.admitted_terminal().await;
    let c = r.window(&t).await;
    gateway_offline(&r).await;
    r.revoke(&kid).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        select_one(&c).await,
        1,
        "an outage ends no session: the last trust set is kept"
    );
    gateway_back_online(&r).await;
    let deadline = Instant::now() + Duration::from_secs(5);
    while c.query_one("SELECT 1", &[]).await.is_ok() {
        assert!(
            Instant::now() < deadline,
            "the revoked session outlived the gateway's recovery by 5 s"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    r.finish().await;
}

/// The tick and every window login's facts read share the gateway's one database connection. An
/// owner who leaves a transaction open holding a terminal's row (an admit or revoke typed in psql
/// inside BEGIN) makes the tick's next attempt for that key wait on the lock; unbounded, that wait
/// would stall every later window login behind it, and revoke enforcement with it (review SF4).
#[tokio::test]
async fn a_held_trust_row_lock_does_not_stall_window_logins() {
    let Some(r) = rig().await else { return };
    let (t, kid) = r.admitted_terminal().await;
    drop(r.window(&t).await);
    r.owner
        .batch_execute(&format!(
            "BEGIN; SELECT 1 FROM terminal_trust WHERE terminal_pubkey = '{kid}' FOR UPDATE"
        ))
        .await
        .unwrap();
    // This handshake queues an attempt for the locked key; the next tick's attempt then waits.
    drop(r.window(&t).await);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let reached = tokio::time::timeout(
        Duration::from_secs(8),
        r.link(&t).connect_with_task(&r.conn("cairn_window_probe")),
    )
    .await;
    r.owner.batch_execute("ROLLBACK").await.unwrap();
    let reached = reached.expect("a window login stalled behind the tick's lock wait");
    assert!(reached.is_ok(), "{:#}", reached.err().unwrap());
    r.finish().await;
}

#[tokio::test]
async fn the_65th_pre_startup_connection_waits_and_then_proceeds() {
    let Some(r) = rig().await else { return };
    let (t, _) = r.admitted_terminal().await;
    drop(r.window(&t).await);
    let mut idle = idle_sockets(&r.addr, 64).await;
    let link = r.link(&t);
    let conn = r.conn("cairn_window_probe");
    let waiting = tokio::spawn(async move { link.connect_with_task(&conn).await });
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(!waiting.is_finished(), "the 65th waits for a permit");
    drop(idle.pop());
    let (c, _) = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("it proceeds once a permit is free")
        .unwrap()
        .unwrap();
    assert_eq!(select_one(&c).await, 1);
    r.finish().await;
}

#[tokio::test]
async fn a_waiting_connection_is_closed_10_s_after_accept() {
    let Some(r) = rig().await else { return };
    let _idle = idle_sockets(&r.addr, 64).await;
    let mut s = TcpStream::connect(&r.addr).await.unwrap();
    let start = Instant::now();
    let n = tokio::time::timeout(Duration::from_secs(13), read_to_close(&mut s))
        .await
        .expect("closed by its deadline")
        .unwrap_or(0);
    let took = start.elapsed();
    assert_eq!(n, 0);
    assert!(
        took > Duration::from_secs(9) && took < Duration::from_millis(11_500),
        "closed {took:?} after accept, not about 10 s"
    );
    r.finish().await;
}

#[tokio::test]
async fn the_129th_pre_startup_connection_is_closed_at_once() {
    let Some(r) = rig().await else { return };
    let _idle = idle_sockets(&r.addr, 128).await;
    let mut s = TcpStream::connect(&r.addr).await.unwrap();
    let start = Instant::now();
    let closed = tokio::time::timeout(Duration::from_secs(2), read_to_close(&mut s)).await;
    // EOF or a reset: either way closed, with nothing said, and well before any 10 s deadline.
    assert!(
        matches!(closed, Ok(Ok(0)) | Ok(Err(_))),
        "the 129th is closed at once: {closed:?} after {:?}",
        start.elapsed()
    );
    r.finish().await;
}
