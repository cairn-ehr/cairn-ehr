//! DB-gated, #909 slice B Task 9: a window on another machine reopens its lost connection through
//! the SAME pinned link it launched on, and a revoked terminal's reopen is a refusal naming the
//! operator, never an outage pressed for ever.
//!
//! A real gateway (`run_gateway_on` on `127.0.0.1:0`) forwards to the test cluster. The rig is a
//! minimal copy of `crates/cairn-node/tests/common/gateway_rig.rs` (test modules cannot be shared
//! across the two Cargo trees): it writes the `terminal_gateway` row as the owner, runs the
//! gateway as its own probe login, and deletes the row again in [`Rig::finish`]. Serialized by
//! `common::connect`'s `test_serial_guard`, the same lock every root-tree DB suite takes. Keys
//! are generated at runtime (house rule 6).
mod common;

use std::time::{Duration, Instant};

use cairn_event::{generate_key, short_fingerprint, SigningKey};
use cairn_gui_data::port::DataError;
use cairn_gui_live::{NodeConnection, Refusal};
use cairn_node::terminal_link::{Link, PinnedTls};
use cairn_node::terminal_service::gateway::serve::run_gateway_on;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_postgres::config::Host;
use tokio_postgres::{Client, Config};

/// The gateway's own login. Not the root rig's: that suite takes its login away in one test.
const GATEWAY_LOGIN: &str = "cairn_gateway_gui_probe";

struct Rig {
    _guard: Client,
    owner: Client,
    /// The window's connection string: the GATEWAY's address, never the database's.
    conn: String,
    node_kid: String,
    gateway: JoinHandle<anyhow::Result<()>>,
}

async fn rig() -> Option<Rig> {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let (owner, guard) = common::connect(&base).await;
    common::window_login(&owner, &base).await;
    let cfg: Config = base.parse().unwrap();
    let Host::Tcp(host) = &cfg.get_hosts()[0] else {
        panic!("CAIRN_TEST_PG must name a TCP host");
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
             VALUES ('{upstream}', now(), 'gui-e2e') \
             ON CONFLICT (only_row) DO UPDATE SET upstream = EXCLUDED.upstream"
        ))
        .await
        .unwrap();
    let (node, node_kid) = generate_key().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let gw_conn = format!("{base} user={GATEWAY_LOGIN}");
    let gateway = tokio::spawn(async move { run_gateway_on(listener, &node, &gw_conn).await });
    Some(Rig {
        _guard: guard,
        owner,
        conn: format!(
            "host=127.0.0.1 port={port} user=cairn_window_probe dbname={} connect_timeout=5",
            cfg.get_dbname().unwrap()
        ),
        node_kid,
        gateway,
    })
}

impl Rig {
    /// A new terminal key, admitted by the owner (as `admit-terminal` does).
    async fn admitted_terminal(&self) -> (SigningKey, String) {
        let (sk, kid) = generate_key().unwrap();
        self.owner
            .execute("SELECT cairn_terminal_attempt($1)", &[&kid])
            .await
            .unwrap();
        let fp = short_fingerprint(&kid).unwrap();
        self.owner
            .execute("SELECT cairn_terminal_admit($1, 'gui-e2e')", &[&fp])
            .await
            .unwrap();
        (sk, kid)
    }

    fn link(&self, terminal: &SigningKey) -> Link {
        Link::Pinned(PinnedTls::new(terminal, &self.node_kid).unwrap())
    }

    /// The window's connection, opened through the pinned link as the window's launch opens it,
    /// retrying for up to 5 s while the gateway's 1 s tick catches up with the admission.
    async fn window(&self, terminal: &SigningKey) -> NodeConnection {
        let deadline = Instant::now() + Duration::from_secs(5);
        let (client, task) = loop {
            match self.link(terminal).connect_with_task(&self.conn).await {
                Ok(opened) => break opened,
                Err(e) if Instant::now() > deadline => panic!("never connected: {e:#}"),
                Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        };
        NodeConnection::reopening_via(client, task, self.conn.clone(), self.link(terminal))
            .await
            .expect("the connection's backend")
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

/// The backend pid the connection is talking to, or `None` if the lock or the read failed.
async fn backend_pid(db: &NodeConnection) -> Option<i32> {
    db.lock()
        .await
        .ok()?
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .ok()
        .map(|row| row.get(0))
}

/// The window's reopen after a lost backend goes back through the pinned link: the connection
/// string names only the gateway, which the plain local door cannot reach.
#[tokio::test]
async fn a_reopen_goes_back_through_the_same_link() {
    let Some(r) = rig().await else { return };
    let (t, _) = r.admitted_terminal().await;
    let db = r.window(&t).await;
    let before = backend_pid(&db).await.expect("the first read works");

    let killed: bool = r
        .owner
        .query_one("SELECT pg_terminate_backend($1, 5000)", &[&before])
        .await
        .unwrap()
        .get(0);
    assert!(killed, "backend {before} was not terminated");

    // One read may fail first, racing the client noticing its backend died.
    let deadline = Instant::now() + Duration::from_secs(5);
    let after = loop {
        if let Some(pid) = backend_pid(&db).await {
            if pid != before {
                break pid;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the connection was never reopened"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_ne!(after, before);
    // The same string over the plain local door reaches nothing: the reopen above went through
    // the link, not around it.
    assert!(
        cairn_node::db::connect_with_task(&r.conn).await.is_err(),
        "the gateway answered a connection that was not the pinned link"
    );
    r.finish().await;
}

/// Once a terminal is revoked, its window's reopen is a CLIENT refusal (the gateway refused the
/// terminal's key, no server answered in Postgres's protocol), worded with `Refusal::Client`'s
/// clause and the link's not-admitted sentence, which names the operator. A second press inside
/// the backoff tries nothing and says so.
#[tokio::test]
async fn a_revoked_terminals_reopen_is_a_refusal_naming_the_operator() {
    let Some(r) = rig().await else { return };
    let (t, kid) = r.admitted_terminal().await;
    let db = r.window(&t).await;
    assert!(backend_pid(&db).await.is_some(), "the first read works");
    let fp = short_fingerprint(&kid).unwrap();
    r.owner
        .execute("SELECT cairn_terminal_revoke($1)", &[&fp])
        .await
        .unwrap();

    // The gateway's 1 s tick refreshes its trust set and THEN ends the revoked key's sessions
    // (`serve.rs`'s `sweep`), so by the time this connection is lost the key is already refused:
    // the first reopen must be the refusal. Until the tick runs the connection stays open. A reopen
    // that reads as an outage is the regression this pins (a window pressed for ever, the
    // operator never named), so it fails the test rather than being pressed again.
    let deadline = Instant::now() + Duration::from_secs(10);
    let refused = loop {
        match db.lock().await {
            Err(lost) if matches!(DataError::from(lost.clone()), DataError::NotProvisioned(_)) => {
                break lost
            }
            Err(lost) => panic!("a revoked terminal's reopen read as an outage: {lost}"),
            Ok(_) => tokio::time::sleep(Duration::from_millis(200)).await,
        }
        assert!(
            Instant::now() < deadline,
            "no refusal 10 s after the revoke"
        );
    };
    let text = refused.to_string();
    assert!(text.contains(Refusal::Client.clause()), "{text}");
    assert!(text.contains("not admitted"), "{text}");
    assert!(text.contains("operator"), "{text}");
    assert!(!text.contains(" s ago"), "the press that tried: {text}");

    // The next press, inside the backoff, dials nothing: its text says when the last attempt
    // failed, and it is still the same refusal.
    let Err(again) = db.lock().await else {
        panic!("a revoked terminal's connection cannot be reopened");
    };
    let again = again.to_string();
    assert!(again.contains(" s ago"), "a press in the backoff: {again}");
    assert!(again.contains("tried nothing"), "{again}");
    assert!(again.contains(Refusal::Client.clause()), "{again}");
    assert!(again.contains("not admitted"), "{again}");
    r.finish().await;
}
