//! Task 10 — `node_event` set-union federation sync over the Task 9 mTLS transport.
//!
//! This is the inter-node path the spec's principle 12 ("uniform core, plural edges")
//! protects: the only thing on the wire is the signed, append-only event core. The
//! protocol ships **raw `signed_bytes`** and the receiver re-derives everything by
//! verifying on apply (the in-DB `apply_remote_node_event` admission gate, §8). There
//! is deliberately no merge logic here — convergence is set-union (idempotent insert
//! keyed by content-address) plus that gate; a node only ever ships bytes, and the
//! receiver verifies and admits-or-rejects. A rejection (deny-all for an un-trusted
//! author) is **normal and non-fatal**: it is logged with the legible DB reason and
//! the pull continues.
//!
//! Wire framing mirrors `cairn-sync`'s length-prefixed frames (`u32` big-endian
//! length + payload) but runs over the `tokio_rustls` stream rather than a bare TCP
//! socket, so every byte is inside the pinned-mTLS session. One request frame (JSON),
//! then a stream of response frames (each a raw `signed_bytes` blob), then EOF.
//!
//! Scope (Task 10): `serve` + `pull_once` + the `serve`/`run` CLI wiring. `status`
//! is Task 11; the full bidirectional two-node convergence E2E is Task 12.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_postgres::Client;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, ServerConfig};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use uuid::Uuid;

use cairn_event::SigningKey;

use crate::db;
use crate::transport::{self, TrustStore};

/// A request on the clinical-federation plane. JSON, one per connection.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "op")]
pub enum Request {
    /// Every `node_event` whose `(recorded_at, node_event_id)` orders after the
    /// given watermark; `after_id: None` means "from the beginning" (full set).
    /// (Task 10 always pulls the full set; the watermark field is the day-one shape
    /// the incremental pull in a later task will populate.)
    NodeEventsAfter { after_id: Option<Uuid> },
}

/// What one `pull_once` did. `received` = frames read off the wire; `admitted` =
/// events the in-DB gate accepted (new or idempotent re-apply); `rejected` =
/// events the gate refused (deny-all for an un-trusted author is the normal case).
#[derive(Debug, Default, Clone, Copy)]
pub struct PullStats {
    pub received: u64,
    pub admitted: u64,
    pub rejected: u64,
}

// ---------------------------------------------------------------------------
// Length-prefixed framing over an async stream (mirrors cairn-sync's
// write_frame/read_frame, but async over the tokio_rustls stream).
// ---------------------------------------------------------------------------

async fn write_frame<S: AsyncWriteExt + Unpin>(s: &mut S, b: &[u8]) -> std::io::Result<()> {
    s.write_all(&(b.len() as u32).to_be_bytes()).await?;
    s.write_all(b).await?;
    s.flush().await
}

/// Read one length-prefixed frame. `Ok(None)` on a clean EOF at a frame boundary
/// (the sender closed after the last frame) — that is the normal stream terminator,
/// not an error.
/// Upper bound on a single wire frame. Node-event envelopes are small; 8 MiB is
/// generous headroom while still capping a hostile/corrupt length prefix.
const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

async fn read_frame<S: AsyncReadExt + Unpin>(s: &mut S) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match s.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_be_bytes(len) as usize;
    // Bound the allocation: node-event frames are tiny (a signed envelope is ~hundreds
    // of bytes). Reject an oversized length so a malformed or compromised-but-pinned
    // peer cannot force a multi-GiB allocation.
    if n > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame length {n} exceeds {MAX_FRAME_BYTES}-byte cap"),
        ));
    }
    let mut buf = vec![0u8; n];
    s.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

// ---------------------------------------------------------------------------
// TrustStore from the DB (snapshot of the active peer pubkeys).
// ---------------------------------------------------------------------------

/// A LIVE set of this node's currently-active peer pubkeys. The rustls verifier
/// closures hold a clone and read it on every handshake, so mutating it (via
/// [`refresh_trust_set`]) updates an *already-built* `ServerConfig`/`ClientConfig`
/// in place — no rebind, no restart. This is what lets `run` apply
/// `peer.added`/`peer.revoked` to BOTH the inbound serve path and the outbound
/// pull on the next cycle (PR #28 review, finding 1).
pub type TrustSet = Arc<RwLock<HashSet<String>>>;

/// Build a [`TrustStore`] backed by a live [`TrustSet`]. The verifier consults the
/// set on every handshake. A poisoned lock fails CLOSED (peer treated as untrusted)
/// — a panic mid-write can only ever *withhold* trust, never grant it.
pub fn trust_store_from_set(set: TrustSet) -> TrustStore {
    Arc::new(move |pk: &str| set.read().map(|s| s.contains(pk)).unwrap_or(false))
}

/// Replace `set`'s contents with this node's currently-active peer pubkeys
/// (`SELECT peer_pubkey FROM trust_peer WHERE status='active'`). Called once at
/// `run` start and again each cycle so revocations/additions take effect live.
pub async fn refresh_trust_set(db: &Client, set: &TrustSet) -> anyhow::Result<()> {
    let rows = db
        .query(
            "SELECT peer_pubkey FROM trust_peer WHERE status='active' AND peer_pubkey IS NOT NULL",
            &[],
        )
        .await
        .context("snapshotting active peer pubkeys for the trust set")?;
    let fresh: HashSet<String> = rows.iter().map(|r| r.get::<_, String>(0)).collect();
    *set.write().map_err(|_| anyhow::anyhow!("trust set lock poisoned"))? = fresh;
    Ok(())
}

/// One-shot snapshot into a [`TrustStore`], for callers that do not refresh (the
/// `serve` CLI command and tests). The returned store is frozen for its lifetime —
/// correct for a single short session; the refreshing path is `run`, which builds
/// the store from a [`TrustSet`] it re-snapshots each cycle. (It reuses the
/// [`TrustSet`] plumbing for DRY; the internal `RwLock` is just never written
/// again — no caller holds the set — so the snapshot is effectively immutable.)
pub async fn trust_store_from_db(db: &Client) -> anyhow::Result<TrustStore> {
    let set: TrustSet = Arc::new(RwLock::new(HashSet::new()));
    refresh_trust_set(db, &set).await?;
    Ok(trust_store_from_set(set))
}

// ---------------------------------------------------------------------------
// serve
// ---------------------------------------------------------------------------

/// Everything one `serve` accept-loop needs: the bound listener, the pinned-mTLS
/// server config, and the Postgres connection string (a fresh DB connection is
/// opened per accepted session, so a slow/poisoned handler never holds a shared
/// client).
pub struct ServeConfig {
    listener: TcpListener,
    tls: Arc<ServerConfig>,
    db_conn: String,
}

/// Bind a `serve` listener and build its pinned-mTLS server config. Returns the
/// **actually-bound** address (so a `127.0.0.1:0` ephemeral bind can be read back
/// for a peer to connect to) alongside the `ServeConfig` to hand to [`serve`].
pub async fn bind_serve(
    listen: SocketAddr,
    db_conn: &str,
    sk: &SigningKey,
    trust: TrustStore,
) -> anyhow::Result<(SocketAddr, ServeConfig)> {
    let listener = TcpListener::bind(listen).await.context("binding serve listener")?;
    let addr = listener.local_addr().context("reading bound serve address")?;
    let tls = transport::server_config(sk, trust)?;
    Ok((addr, ServeConfig { listener, tls, db_conn: db_conn.to_string() }))
}

/// Accept pinned-mTLS sessions forever, serving each in its own task. An unpinned
/// client is rejected by the Task 9 `ClientCertVerifier` during the handshake; a
/// per-connection handler error (a dropped link, a malformed request) is logged and
/// never takes the loop down.
///
/// Whether a revocation takes effect mid-serve depends on how the trust was built:
/// a config from [`trust_store_from_set`] (the `run` path) honours live updates on
/// the next handshake; a frozen [`trust_store_from_db`] snapshot (the one-shot
/// `serve` CLI command) is restart-scoped.
pub async fn serve(cfg: ServeConfig) -> anyhow::Result<()> {
    let acceptor = TlsAcceptor::from(cfg.tls);
    loop {
        let (tcp, peer) = match cfg.listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("serve: accept error: {e}");
                continue;
            }
        };
        let acceptor = acceptor.clone();
        let db_conn = cfg.db_conn.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_conn(acceptor, tcp, &db_conn).await {
                eprintln!("serve: session with {peer} ended: {e}");
            }
        });
    }
}

/// Handle one accepted connection: complete the mTLS handshake (which pins the
/// client), read one request frame, and stream the requested `node_event` bytes.
async fn serve_conn(acceptor: TlsAcceptor, tcp: TcpStream, db_conn: &str) -> anyhow::Result<()> {
    let mut tls = acceptor.accept(tcp).await.context("mTLS handshake (client pin)")?;

    let req_bytes = match read_frame(&mut tls).await? {
        Some(b) => b,
        None => return Ok(()), // client connected and hung up without a request
    };
    let req: Request = serde_json::from_slice(&req_bytes).context("decoding request frame")?;

    let db = db::connect(db_conn).await.context("serve: connecting to DB")?;
    match req {
        Request::NodeEventsAfter { after_id } => {
            stream_node_events(&mut tls, &db, after_id).await?;
        }
    }
    // Closing the stream is the EOF the puller reads as "no more frames".
    tls.shutdown().await.ok();
    Ok(())
}

/// Stream every `node_event.signed_bytes` (after the optional watermark) as a raw
/// length-framed binary frame, ordered by `(recorded_at, node_event_id)` — the
/// deterministic causal order the receiver applies in.
async fn stream_node_events<S: AsyncWriteExt + Unpin>(
    tls: &mut S,
    db: &Client,
    after_id: Option<Uuid>,
) -> anyhow::Result<()> {
    // `after_id` is the day-one watermark shape; Task 10 pulls the full set
    // (after_id = NULL), and a later incremental pull will key off recorded_at.
    // NOTE: the `<> $1` below is an exclude-ONE placeholder, not a true "after"
    // predicate — it must be REPLACED (not extended) by a `(recorded_at, id) >`
    // watermark when incremental pull lands; it is NOT working incremental logic.
    // We keep the SQL uniform: NULL after_id selects everything. The id is bound as
    // text and cast in SQL (matching the codebase's $1::text::uuid convention),
    // avoiding the tokio-postgres `with-uuid-1` feature dependency.
    let after_text: Option<String> = after_id.map(|u| u.to_string());
    let rows = db
        .query(
            "SELECT signed_bytes FROM node_event \
             WHERE $1::text IS NULL OR node_event_id <> $1::text::uuid \
             ORDER BY recorded_at, node_event_id",
            &[&after_text],
        )
        .await
        .context("selecting node_event bytes to stream")?;
    for row in &rows {
        let bytes: Vec<u8> = row.get(0);
        write_frame(tls, &bytes).await.context("writing a node_event frame")?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// pull_once
// ---------------------------------------------------------------------------

/// A built client side of a pull: the pinned-mTLS client config plus the DB conn
/// string the admitted events are applied into.
pub struct PullConfig {
    tls: Arc<ClientConfig>,
    db_conn: String,
}

/// Build the pinned-mTLS client config for a pull and bind it to the DB the
/// admitted events land in.
pub async fn client_config(
    db_conn: &str,
    sk: &SigningKey,
    trust: TrustStore,
) -> anyhow::Result<PullConfig> {
    let tls = transport::client_config(sk, trust)?;
    Ok(PullConfig { tls, db_conn: db_conn.to_string() })
}

/// Connect to `peer` over pinned mTLS, request all node events, and apply each via
/// the in-DB admission gate. Per-event rejections are **non-fatal** (logged with the
/// legible DB reason) — deny-all for an un-trusted author is the expected case, so
/// one rejected event must never abort the pull.
pub async fn pull_once(peer: SocketAddr, cfg: PullConfig) -> anyhow::Result<PullStats> {
    let connector = TlsConnector::from(cfg.tls);
    let tcp = TcpStream::connect(peer).await.with_context(|| format!("connecting to {peer}"))?;
    // The pinned server cert's SAN is "cairn-node" (see transport::node_cert); the
    // ServerName is cosmetic here because the custom verifier pins on the key, not
    // the name, but rustls still requires a syntactically valid name.
    let name = ServerName::try_from("cairn-node").context("building server name")?;
    let mut tls = connector.connect(name, tcp).await.context("mTLS handshake (server pin)")?;

    // Task 10: pull the full set.
    let req = Request::NodeEventsAfter { after_id: None };
    write_frame(&mut tls, &serde_json::to_vec(&req)?).await.context("sending request")?;

    let db = db::connect(&cfg.db_conn).await.context("pull: connecting to DB")?;
    let mut stats = PullStats::default();
    while let Some(frame) = read_frame(&mut tls).await.context("reading a response frame")? {
        stats.received += 1;
        match db.execute("SELECT apply_remote_node_event($1)", &[&frame]).await {
            Ok(_) => stats.admitted += 1,
            Err(e) => {
                // Non-fatal: the admission gate refused this event (un-trusted
                // author / malformed / fail-closed). Log the legible reason, keep going.
                stats.rejected += 1;
                eprintln!("pull: node_event rejected (non-fatal): {e}");
            }
        }
    }
    Ok(stats)
}

// ---------------------------------------------------------------------------
// run — unattended serve + periodic pull (mirrors `cairn-sync run`).
// ---------------------------------------------------------------------------

/// Serve in the background and pull from `peer` every `interval` seconds, surviving
/// connect errors (a sustained outage is logged as a partition and the loop keeps
/// going — availability over consistency). Runs until cancelled.
pub async fn run(
    listen: SocketAddr,
    peer: SocketAddr,
    db_conn: &str,
    sk: &SigningKey,
    interval_secs: u64,
) -> anyhow::Result<()> {
    // ONE live trust set, shared by the inbound serve verifier AND every outbound
    // pull. Re-snapshotting it each cycle (below) makes peer.added / peer.revoked
    // take effect on BOTH paths with no process restart: the rustls verifier
    // closures read this set live, so the already-built serve `ServerConfig` and
    // pull `ClientConfig` honour a revocation on the very next handshake. (Earlier
    // this froze the serve-side set for the process lifetime — PR #28 review,
    // finding 1.)
    let trust_set: TrustSet = Arc::new(RwLock::new(HashSet::new()));
    let boot_db = db::connect(db_conn).await.context("run: connecting boot DB")?;
    refresh_trust_set(&boot_db, &trust_set)
        .await
        .context("run: initial trust snapshot")?;

    let (addr, serve_cfg) =
        bind_serve(listen, db_conn, sk, trust_store_from_set(trust_set.clone())).await?;
    eprintln!("run: serving on {addr}, pulling {peer} every {interval_secs}s");
    let serve_handle = tokio::spawn(serve(serve_cfg));

    // The pull side reads the SAME live set, so its TLS config is also built once.
    let client_tls = transport::client_config(sk, trust_store_from_set(trust_set.clone()))?;
    let db_conn = db_conn.to_string();
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs.max(1)));
    loop {
        ticker.tick().await;
        // Re-snapshot the live set so peering changes since the last cycle apply to
        // serve AND pull. A failed refresh is non-fatal: the last-known set stays in
        // force and we still attempt the pull. During a DB outage a pending
        // revocation therefore lands only once the DB is reachable again — the
        // deliberate availability-over-consistency trade (we never halt federation
        // on a transient DB blip), and the still-pinned mTLS + in-DB admission gate
        // remain the hard floor regardless.
        match db::connect(&db_conn).await {
            Ok(c) => {
                if let Err(e) = refresh_trust_set(&c, &trust_set).await {
                    eprintln!("run: trust refresh failed, serving last-known set: {e}");
                }
            }
            Err(e) => {
                eprintln!("run: DB unreachable for trust refresh, serving last-known set: {e}")
            }
        }
        let cfg = PullConfig { tls: client_tls.clone(), db_conn: db_conn.clone() };
        match pull_once(peer, cfg).await {
            Ok(s) => eprintln!(
                "run: pull {peer}: received={} admitted={} rejected={}",
                s.received, s.admitted, s.rejected
            ),
            // A sustained outage = a partition. Logged, never fatal.
            Err(e) => eprintln!("run: PARTITION pulling {peer}: {e}"),
        }
        if serve_handle.is_finished() {
            anyhow::bail!("run: serve task exited unexpectedly");
        }
    }
}
