//! #909 slice B, Task 7 — the terminal gateway end to end: a real gateway (`serve::run_gateway_on`
//! on `127.0.0.1:0`), forwarding to the test cluster's own TCP route, reached by the window's real
//! pinned link (Task 6's [`Link`]).
//!
//! The gateway reads its upstream from the `terminal_gateway` row like production does. The
//! test cluster uses `trust` auth, which `gateway-setup` rightly refuses, so each rig writes that
//! row directly as the owner (an upsert) and [`Rig::finish`] deletes it again.
//!
//! The gateway runs as its own probe login (`cairn_gateway_e2e_login`, a `cairn_terminal_service`
//! member) so that one test can take its database connection away (NOLOGIN + terminate) without
//! touching the login the other suites share. Every rig sets it back to LOGIN first.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`. Keys are generated at
//! runtime (house rule 6).
mod common;

use std::time::{Duration, Instant};

use cairn_event::{generate_key, short_fingerprint, SigningKey};
use cairn_node::db::{self, ConnectFailed};
use cairn_node::terminal_link::{Link, PinnedTls};
use cairn_node::terminal_service::gateway::admit::REFUSED;
use cairn_node::terminal_service::gateway::serve::run_gateway_on;
use common::{cs, service_login, window_login};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_postgres::config::Host;
use tokio_postgres::{Client, Config};

const GATEWAY_LOGIN: &str = "cairn_gateway_e2e_login";

struct Rig {
    _guard: Client,
    owner: Client,
    /// The gateway's `host:port`.
    addr: String,
    dbname: String,
    node_kid: String,
    gateway: JoinHandle<anyhow::Result<()>>,
}

async fn rig() -> Option<Rig> {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = db::test_serial_guard(&base).await.unwrap();
    let owner = db::connect_and_load_schema(&base).await.unwrap();
    window_login(&owner, &base).await;
    service_login(&owner, &base).await;
    let cfg: Config = base.parse().unwrap();
    let host = match &cfg.get_hosts()[0] {
        Host::Tcp(h) => h.clone(),
        _ => panic!("CAIRN_TEST_PG must name a TCP host"),
    };
    let upstream = format!(
        "{host}:{}",
        cfg.get_ports().first().copied().unwrap_or(5432)
    );
    owner
        .batch_execute(&format!(
            "DO $$ BEGIN \
               IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{GATEWAY_LOGIN}') THEN \
                 CREATE ROLE {GATEWAY_LOGIN} LOGIN; \
               END IF; \
             EXCEPTION WHEN duplicate_object THEN NULL; \
             END $$; \
             ALTER ROLE {GATEWAY_LOGIN} LOGIN; \
             GRANT cairn_terminal_service TO {GATEWAY_LOGIN}; \
             TRUNCATE terminal_trust; \
             INSERT INTO terminal_gateway (upstream, checked_at, checked_by) \
             VALUES ('{upstream}', now(), 'e2e') \
             ON CONFLICT (only_row) DO UPDATE SET upstream = EXCLUDED.upstream"
        ))
        .await
        .unwrap();
    let (node, node_kid) = generate_key().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let gw_conn = format!("{base} user={GATEWAY_LOGIN}");
    let gateway = tokio::spawn(async move { run_gateway_on(listener, &node, &gw_conn).await });
    Some(Rig {
        _guard: guard,
        owner,
        addr,
        dbname: cfg.get_dbname().unwrap().to_string(),
        node_kid,
        gateway,
    })
}

impl Rig {
    /// The window's connection string to the gateway, as `user`.
    fn conn(&self, user: &str) -> String {
        let (host, port) = self.addr.rsplit_once(':').unwrap();
        format!(
            "host={host} port={port} user={user} dbname={} connect_timeout=5",
            self.dbname
        )
    }

    /// A new terminal key, admitted by the owner (as `admit-terminal` does).
    async fn admitted_terminal(&self) -> (SigningKey, String) {
        let (sk, kid) = generate_key().unwrap();
        self.owner
            .execute("SELECT cairn_terminal_attempt($1)", &[&kid])
            .await
            .unwrap();
        let fp = short_fingerprint(&kid).unwrap();
        self.owner
            .execute("SELECT cairn_terminal_admit($1, 'e2e')", &[&fp])
            .await
            .unwrap();
        (sk, kid)
    }

    fn link(&self, terminal: &SigningKey) -> Link {
        Link::Pinned(PinnedTls::new(terminal, &self.node_kid).unwrap())
    }

    /// Connect as the window login, retrying for up to 5 s while the gateway's 1 s tick catches
    /// up with an admission (or a recovery).
    async fn window(&self, terminal: &SigningKey) -> Client {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match self
                .link(terminal)
                .connect_with_task(&self.conn("cairn_window_probe"))
                .await
            {
                Ok((c, _)) => return c,
                Err(e) if Instant::now() > deadline => panic!("never connected: {e:#}"),
                Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
    }

    async fn revoke(&self, kid: &str) {
        let fp = short_fingerprint(kid).unwrap();
        self.owner
            .execute("SELECT cairn_terminal_revoke($1)", &[&fp])
            .await
            .unwrap();
    }

    async fn backend_alive(&self, pid: i32) -> bool {
        self.owner
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1)",
                &[&pid],
            )
            .await
            .unwrap()
            .get(0)
    }

    /// Stop the gateway and remove the route row this rig wrote.
    async fn finish(self) {
        self.gateway.abort();
        self.owner
            .batch_execute("DELETE FROM terminal_gateway")
            .await
            .unwrap();
    }
}

/// How a failed connect ended: (text, refused_by_client, sqlstate).
fn failure(err: anyhow::Error) -> (String, bool, Option<String>) {
    let cf = err
        .downcast_ref::<ConnectFailed>()
        .unwrap_or_else(|| panic!("every error is a ConnectFailed: {err:#}"));
    (
        cf.to_string(),
        cf.refused_by_client(),
        cf.sqlstate().map(str::to_string),
    )
}

async fn select_one(c: &Client) -> i32 {
    c.query_one("SELECT 1", &[]).await.unwrap().get(0)
}

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
    let sleeping = tokio::spawn(async move { c.query_one("SELECT pg_sleep(30)", &[]).await });
    // Until the statement is really running in its backend.
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

#[tokio::test]
async fn startups_are_refused_while_the_gateways_db_connection_is_down() {
    let Some(r) = rig().await else { return };
    let (t, _) = r.admitted_terminal().await;
    drop(r.window(&t).await);
    // NOLOGIN first, so the gateway cannot reconnect before the next startup is tried.
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
    r.owner
        .batch_execute(&format!("ALTER ROLE {GATEWAY_LOGIN} LOGIN"))
        .await
        .unwrap();
    let c = r.window(&t).await; // retries until the gateway has reconnected
    assert_eq!(select_one(&c).await, 1);
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
