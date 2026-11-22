//! The shared rig of the terminal gateway's end-to-end suites (#909 slice B, Task 7):
//! `terminal_gateway_e2e.rs` and `terminal_gateway_e2e_refusals.rs` pull it in with
//! `#[path = "common/gateway_rig.rs"] mod gateway_rig;` (after their own `mod common;`).
//!
//! A real gateway (`serve::run_gateway_on` on `127.0.0.1:0`) forwards to the test cluster's own TCP
//! route and is reached by the window's real pinned link (Task 6's `Link`). It reads its upstream
//! from the `terminal_gateway` row like production does. The test cluster uses `trust` auth, which
//! `gateway-setup` rightly refuses, so each rig writes that row directly as the owner (an upsert)
//! and [`Rig::finish`] deletes it again.
//!
//! The gateway runs as its own probe login ([`GATEWAY_LOGIN`], a `cairn_terminal_service` member)
//! so that one test can take its database connection away (NOLOGIN + terminate) without touching
//! the login the other suites share. Every rig sets it back to LOGIN first.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`. Keys are generated at
//! runtime (house rule 6).
#![allow(dead_code)] // each suite uses its own subset

use std::time::{Duration, Instant};

use cairn_event::{generate_key, short_fingerprint, SigningKey};
use cairn_node::db::{self, ConnectFailed};
use cairn_node::terminal_link::{Link, PinnedTls};
use cairn_node::terminal_service::gateway::serve::run_gateway_on;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_postgres::config::Host;
use tokio_postgres::{Client, Config};

use crate::common::{cs, service_login, window_login};

pub const GATEWAY_LOGIN: &str = "cairn_gateway_e2e_login";

pub struct Rig {
    _guard: Client,
    pub owner: Client,
    /// The gateway's `host:port`.
    pub addr: String,
    dbname: String,
    node_kid: String,
    gateway: JoinHandle<anyhow::Result<()>>,
}

pub async fn rig() -> Option<Rig> {
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
    pub fn conn(&self, user: &str) -> String {
        let (host, port) = self.addr.rsplit_once(':').unwrap();
        format!(
            "host={host} port={port} user={user} dbname={} connect_timeout=5",
            self.dbname
        )
    }

    /// A new terminal key, admitted by the owner (as `admit-terminal` does).
    pub async fn admitted_terminal(&self) -> (SigningKey, String) {
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

    pub fn link(&self, terminal: &SigningKey) -> Link {
        Link::Pinned(PinnedTls::new(terminal, &self.node_kid).unwrap())
    }

    /// Connect as the window login, retrying for up to 5 s while the gateway's 1 s tick catches
    /// up with an admission (or a recovery).
    pub async fn window(&self, terminal: &SigningKey) -> Client {
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

    pub async fn revoke(&self, kid: &str) {
        let fp = short_fingerprint(kid).unwrap();
        self.owner
            .execute("SELECT cairn_terminal_revoke($1)", &[&fp])
            .await
            .unwrap();
    }

    pub async fn backend_alive(&self, pid: i32) -> bool {
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
    pub async fn finish(self) {
        self.gateway.abort();
        self.owner
            .batch_execute("DELETE FROM terminal_gateway")
            .await
            .unwrap();
    }
}

/// How a failed connect ended: (text, refused_by_client, sqlstate).
pub fn failure(err: anyhow::Error) -> (String, bool, Option<String>) {
    let cf = err
        .downcast_ref::<ConnectFailed>()
        .unwrap_or_else(|| panic!("every error is a ConnectFailed: {err:#}"));
    (
        cf.to_string(),
        cf.refused_by_client(),
        cf.sqlstate().map(str::to_string),
    )
}

pub async fn select_one(c: &Client) -> i32 {
    c.query_one("SELECT 1", &[]).await.unwrap().get(0)
}
