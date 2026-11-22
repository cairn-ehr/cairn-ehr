//! The gateway's accept loop and one connection's life (#909 slice B, design D2).
//!
//! HOW IT FITS. [`run_gateway`] runs inside `cairn-node terminal-service --gateway-listen`, beside
//! the service's job loop. It opens its OWN database connection (the service's login), loads the
//! terminal trust set and the recorded upstream route, and only then accepts. Two things then run
//! for ever, in one future:
//!
//! * the accept loop: each connection gets a task that walks D2's steps in order
//!   (`pre_startup`) and, if the startup is admitted, pipes it (`session`, `pipe.rs`);
//! * the 1 s tick (`tick`): refresh the trust set, end the sessions of every terminal no longer
//!   active (and cancel what their backends are running), record each key seen (an unknown one
//!   as a `pending` attempt, a known one's `last_seen`), log one summary line, reconnect the
//!   gateway's own connection if it dropped.
//!
//! THE FLOOD LIMITS (Review Focus 2). Before startup a connection holds one of 128 slots from
//! `accept` until it closes; with all 128 taken a new one is closed at once, so idle sockets
//! from one LAN host cannot use up the service's file descriptors. At most 64 of those walk the
//! steps at once (a permit, held until startup); the rest wait. Each has 10 s counted from
//! `accept`, the wait included. An `accept` error is counted, never fatal, and followed by a
//! 100 ms pause so it cannot busy-loop.
//!
//! TWO KINDS OF REFUSAL (controller ruling R2). A key not active in the trust set fails the TLS
//! handshake with `access_denied`: permanent, the window says "not admitted". Anything the
//! gateway cannot check right now (its own database connection is down, Postgres refused the
//! upstream route) is refused after the handshake with SQLSTATE 57P03, which the window retries.
//! A login that may not pass is refused with 28000 and the fixed `admit::REFUSED` text
//! (the split is `admit::judge`).
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anyhow::Context;
use cairn_event::{short_fingerprint, SigningKey};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio_postgres::Client;
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

use super::accept::{
    keepalive, next_connection, AcceptErrors, PRE_STARTUP_ACTIVE, PRE_STARTUP_DEADLINE,
    PRE_STARTUP_TOTAL,
};
use super::admit::{judge, read_login_facts, DbUnavailable, Refusal, NOT_NOW};
use super::pipe::{close_gently, pipe, send_cancel};
use super::sessions::{CloseOnDrop, Sessions};
use super::trust::TerminalTrust;
use super::upstream::{self, AsyncReadWrite, Upstream};
use super::wire::{error_response, inside, opening, Inside, Opening, Startup, MAX_STARTUP};
use crate::terminal_service::commit::primary_message;
use crate::terminal_service::run::fresh_failure;

/// The trust-set refresh and revoke sweep period.
pub const TICK: Duration = Duration::from_secs(1);

/// The gateway's own database connection, replaced by the tick when it drops.
struct GatewayDb {
    conn: String,
    client: RwLock<Arc<Client>>,
}

impl GatewayDb {
    fn current(&self) -> Arc<Client> {
        self.client
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// The current client, reconnected first if it has closed. Only the tick calls this; a
    /// connection that finds the client closed refuses with 57P03 instead of waiting.
    async fn healthy(&self) -> anyhow::Result<Arc<Client>> {
        let c = self.current();
        if !c.is_closed() {
            return Ok(c);
        }
        let fresh = Arc::new(crate::db::connect(&self.conn).await?);
        *self.client.write().unwrap_or_else(|p| p.into_inner()) = fresh.clone();
        Ok(fresh)
    }
}

/// Everything the connection tasks and the tick share.
struct Gateway {
    tls: TlsAcceptor,
    upstream: Upstream,
    /// The database a startup must name: the one the gateway's own login is connected to.
    node_db: String,
    db: GatewayDb,
    sessions: Sessions,
    permits: Semaphore,
    accept_errors: AcceptErrors,
}

/// Bind `listen` and run the gateway on it ([`run_gateway_on`]).
pub async fn run_gateway(listen: SocketAddr, sk: &SigningKey, conn: &str) -> anyhow::Result<()> {
    let listener = TcpListener::bind(listen)
        .await
        .with_context(|| format!("listening for terminals on {listen}"))?;
    run_gateway_on(listener, sk, conn).await
}

/// Run the gateway on a bound listener. Never returns while healthy; an `Err` means it could not
/// start (no database, no recorded route, no first trust-set load), and the caller retries.
pub async fn run_gateway_on(
    listener: TcpListener,
    sk: &SigningKey,
    conn: &str,
) -> anyhow::Result<()> {
    let client = crate::db::connect(conn).await?;
    let (trust, attempts) = TerminalTrust::new();
    // The first load comes BEFORE any accept: until it, every key would be refused as unknown.
    trust.refresh(&client).await?;
    let route: Option<String> = client
        .query_opt("SELECT upstream FROM terminal_gateway", &[])
        .await
        .context("reading the gateway's recorded route")?
        .map(|r| r.get(0));
    let Some(route) = route else {
        anyhow::bail!(
            "no gateway route is recorded: run cairn-node gateway-setup --upstream <route> as the owner"
        );
    };
    let up = upstream::parse(&route).map_err(anyhow::Error::msg)?;
    let node_db: String = client
        .query_one("SELECT current_database()::text", &[])
        .await
        .context("reading the node's database name")?
        .get(0);
    let tls = crate::transport::gateway_server_config(sk, trust.store())?;
    let gw = Arc::new(Gateway {
        tls: TlsAcceptor::from(tls),
        upstream: up,
        node_db,
        db: GatewayDb {
            conn: conn.to_string(),
            client: RwLock::new(Arc::new(client)),
        },
        sessions: Sessions::default(),
        permits: Semaphore::new(PRE_STARTUP_ACTIVE),
        accept_errors: AcceptErrors::default(),
    });
    eprintln!("terminal gateway listening on {}", listener.local_addr()?);
    // However this run ends (a panic included), its sessions end with it: no later run's sweep
    // sees them, so left open a revoke could never end them (`sessions.rs`, final review I1).
    let _close = CloseOnDrop(gw.sessions.clone());
    // One future: dropping it (the job loop's supervisor, a test) stops the tick as well.
    tokio::select! {
        () = accept_loop(listener, gw.clone()) => {}
        () = tick(&gw, &trust, attempts) => {}
    }
    Ok(())
}

async fn accept_loop(listener: TcpListener, gw: Arc<Gateway>) {
    let slots = Arc::new(Semaphore::new(PRE_STARTUP_TOTAL));
    loop {
        // An accept error (EMFILE, ENFILE, …) is counted and paused on inside, never returned.
        let (tcp, peer) = next_connection(|| listener.accept(), &gw.accept_errors).await;
        let accepted = Instant::now();
        // All 128 taken: closed at once, with no reply (dropping it closes it).
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            continue;
        };
        let gw = gw.clone();
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::from_std(accepted + PRE_STARTUP_DEADLINE);
            match tokio::time::timeout_at(deadline, pre_startup(&gw, tcp, peer)).await {
                Ok(Outcome::Admitted(a)) => {
                    drop(slot);
                    session(&gw, *a, peer).await;
                }
                // The slot is held through the (bounded) gentle close: it counts sockets.
                Ok(Outcome::Refused(s)) => close_gently(s).await,
                Ok(Outcome::Dropped) | Err(_) => {}
            }
        });
    }
}

/// An admitted startup, already replayed upstream: what [`session`] pipes.
struct Admitted {
    tls: TlsStream<TcpStream>,
    up: Box<dyn AsyncReadWrite>,
    key: String,
    user: String,
}

enum Outcome {
    Admitted(Box<Admitted>),
    /// Refused after something was (or may have been) written: close gently.
    Refused(Box<dyn AsyncReadWrite>),
    /// The peer went away, or sent nothing usable before the TLS session: just drop it.
    Dropped,
}

/// D2 steps 1–3 for one connection, under one of the 64 permits.
async fn pre_startup(gw: &Gateway, mut tcp: TcpStream, peer: SocketAddr) -> Outcome {
    let Ok(_permit) = gw.permits.acquire().await else {
        return Outcome::Dropped;
    };
    if let Err(e) = keepalive(&tcp) {
        eprintln!("terminal gateway: could not set keepalive for {peer}: {e}");
    }
    // Step 1: SSLRequest, or GSSENCRequest (answered N) then SSLRequest. Else no reply.
    let Some(mut first) = read8(&mut tcp).await else {
        return Outcome::Dropped;
    };
    if opening(first) == Opening::Gss {
        if tcp.write_all(b"N").await.is_err() {
            return Outcome::Dropped;
        }
        let Some(next) = read8(&mut tcp).await else {
            return Outcome::Dropped;
        };
        first = next;
    }
    if opening(first) != Opening::Ssl || tcp.write_all(b"S").await.is_err() {
        return Outcome::Refused(Box::new(tcp));
    }
    // Step 2: mutual TLS 1.3, pinned by the trust set. A refusal has sent its alert already.
    let mut tls = match gw.tls.accept(tcp).into_fallible().await {
        Ok(tls) => tls,
        Err((_, tcp)) => return Outcome::Refused(Box::new(tcp)),
    };
    let Some(key) = peer_key(&tls) else {
        return Outcome::Refused(Box::new(tls));
    };
    // Step 3: one packet, a startup to vet or a cancel to forward.
    let Some(packet) = read_packet(&mut tls).await else {
        return Outcome::Refused(Box::new(tls));
    };
    let startup = match inside(&packet) {
        Ok(Inside::Startup(s)) => s,
        Ok(Inside::Cancel(c)) => {
            if let Err(e) = send_cancel(&gw.upstream, &c).await {
                eprintln!("terminal gateway: forwarding a cancel from {peer} failed: {e}");
            }
            return Outcome::Refused(Box::new(tls));
        }
        Err(_) => return Outcome::Refused(Box::new(tls)),
    };
    let user = startup.get("user").unwrap_or_default().to_string();
    if let Err(r) = vet(gw, &startup).await {
        eprintln!(
            "terminal gateway: refused login {} from {peer} [{}]: {}",
            shown(&user),
            r.sqlstate,
            r.why
        );
        return refuse(tls, r.sqlstate, r.message).await;
    }
    // Step 4's start: the route, and the startup replayed byte for byte.
    let mut up = match gw.upstream.connect().await {
        Ok(up) => up,
        Err(e) => {
            eprintln!("terminal gateway: the upstream route failed for {peer}: {e}");
            return refuse(tls, "57P03", NOT_NOW).await;
        }
    };
    if up.write_all(&startup.raw).await.is_err() {
        return refuse(tls, "57P03", NOT_NOW).await;
    }
    Outcome::Admitted(Box::new(Admitted { tls, up, key, user }))
}

/// Write an `ErrorResponse` and hand the stream back to be closed gently.
async fn refuse(mut tls: TlsStream<TcpStream>, sqlstate: &str, message: &str) -> Outcome {
    let _ = tls.write_all(&error_response(sqlstate, message)).await;
    let _ = tls.flush().await;
    Outcome::Refused(Box::new(tls))
}

/// Read the login's facts on the gateway's own connection and judge the startup.
async fn vet(gw: &Gateway, s: &Startup) -> Result<(), Refusal> {
    let client = gw.db.current();
    let facts = if client.is_closed() {
        Err(anyhow::anyhow!(
            "the gateway's database connection is closed"
        ))
        .context(DbUnavailable)
    } else {
        read_login_facts(&client, s.get("user").unwrap_or_default()).await
    };
    judge(facts, s, &gw.node_db)
}

/// Pipe an admitted session, registered in [`Sessions`] so a revoke can end it. Logs one line
/// when it opens and one when it closes, naming no startup parameter but `user`.
async fn session(gw: &Gateway, a: Admitted, peer: SocketAddr) {
    let Admitted { tls, up, key, user } = a;
    let fp = short_fingerprint(&key).unwrap_or_default();
    let user = shown(&user);
    eprintln!("terminal session opened: peer {peer}, terminal {fp}, login {user}");
    // The pipe task needs its own registration, which needs the task's abort handle: hand the
    // guard in after spawning.
    let (tx, rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        match rx.await {
            Ok(guard) => pipe(tls, up, &guard).await,
            Err(_) => Ok(()),
        }
    });
    let _ = tx.send(gw.sessions.open(&key, task.abort_handle()));
    let cause = match task.await {
        Ok(Ok(())) => "closed".to_string(),
        Ok(Err(e)) => format!("ended: {e}"),
        Err(j) if j.is_cancelled() && gw.sessions.is_closed() => {
            "ended: the gateway's run ended (it is restarted)".to_string()
        }
        Err(j) if j.is_cancelled() => "ended: the terminal is no longer admitted".to_string(),
        Err(_) => "ended: the pipe failed".to_string(),
    };
    eprintln!("terminal session closed: peer {peer}, terminal {fp}, login {user}: {cause}");
}

/// The 1 s tick, for ever. A failing step is logged once per distinct failure; the summary of
/// refused handshakes and accept errors is logged on every tick that has something to say.
async fn tick(gw: &Gateway, trust: &TerminalTrust, mut attempts: mpsc::Receiver<String>) {
    let mut every = tokio::time::interval(TICK);
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_failure = None;
    loop {
        every.tick().await;
        let r = sweep(gw, trust, &mut attempts).await;
        if let Some(line) = fresh_failure(&mut last_failure, r) {
            eprintln!("terminal gateway: {line}");
        }
        let refused = trust.take_refused();
        if refused > 0 {
            eprintln!("terminal gateway: {refused} unadmitted handshakes refused");
        }
        if let Some((n, last)) = gw.accept_errors.take() {
            eprintln!("terminal gateway: {n} connections could not be accepted (the last: {last})");
        }
    }
}

/// One tick's database work: reconnect if needed, refresh, end revoked sessions, record attempts.
async fn sweep(
    gw: &Gateway,
    trust: &TerminalTrust,
    attempts: &mut mpsc::Receiver<String>,
) -> Result<(), String> {
    let client = gw.db.healthy().await.map_err(|e| {
        format!(
            "reconnecting to the database failed: {}",
            primary_message(&e)
        )
    })?;
    let active = trust.refresh(&client).await.map_err(|e| {
        format!(
            "refreshing the trust set failed (the last one is kept): {}",
            primary_message(&e)
        )
    })?;
    for cancel in gw.sessions.close_all_but(&active) {
        if let Err(e) = send_cancel(&gw.upstream, &cancel).await {
            eprintln!("terminal gateway: cancelling a revoked session's statement failed: {e}");
        }
    }
    let mut keys = HashSet::new();
    while let Ok(k) = attempts.try_recv() {
        keys.insert(k);
    }
    for k in keys {
        client
            .execute("SELECT cairn_terminal_attempt($1)", &[&k])
            .await
            .map_err(|e| {
                format!(
                    "recording a terminal's attempt failed: {}",
                    primary_message(&e.into())
                )
            })?;
    }
    Ok(())
}

async fn read8(tcp: &mut TcpStream) -> Option<[u8; 8]> {
    let mut b = [0u8; 8];
    tcp.read_exact(&mut b).await.ok().map(|_| b)
}

/// One length-prefixed packet of 8 to [`MAX_STARTUP`] bytes, length included.
async fn read_packet(tls: &mut TlsStream<TcpStream>) -> Option<Vec<u8>> {
    let mut len = [0u8; 4];
    tls.read_exact(&mut len).await.ok()?;
    let n = u32::from_be_bytes(len) as usize;
    if !(8..=MAX_STARTUP).contains(&n) {
        return None;
    }
    let mut packet = len.to_vec();
    packet.resize(n, 0);
    tls.read_exact(&mut packet[4..]).await.ok()?;
    Some(packet)
}

/// The terminal's key (hex), from the certificate it presented in the handshake.
fn peer_key(tls: &TlsStream<TcpStream>) -> Option<String> {
    let cert = tls.get_ref().1.peer_certificates()?.first()?;
    crate::transport::peer_pubkey_hex(cert).ok()
}

/// A client-supplied string (the startup's `user`) as it may appear in the log: quoted, with
/// newlines and control characters escaped, so a crafted login cannot forge log lines. **Pure.**
fn shown(client_text: &str) -> String {
    format!("{client_text:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crafted_login_cannot_forge_a_log_line() {
        let s = shown("w\nterminal session opened: peer 10.0.0.9\u{1b}[2K");
        assert!(!s.contains('\n') && !s.contains('\u{1b}'), "{s}");
        assert_eq!(shown("cairn_window_probe"), "\"cairn_window_probe\"");
    }
}
