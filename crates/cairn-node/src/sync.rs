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
use cairn_event::SigningKey;

use crate::db;
use crate::transport::{self, TrustStore};

/// Full-sweep cadence: the puller does an incremental `seq`-cursor pull each cycle and a
/// full sweep (cursor reset to 0) every `FULL_SWEEP_EVERY` cycles. The sweep is the
/// correctness floor (issue #38): it reconciles any event a residual hazard (commit-order
/// race, a rejected-then-later-trusted author, an address remap) caused incremental to
/// skip. `node_event` is low-volume, so a frequent sweep is cheap.
const FULL_SWEEP_EVERY: u64 = 10;

/// Stall bound for any single network step (connect, handshake, one frame, one write).
/// This bounds STALL, not transfer size: a long full sweep is fine as long as frames keep
/// arriving inside the window. Without it a pinned-but-hung peer (compromised,
/// half-crashed, black-holed) parks `pull_into` on a read forever — which in `run` also
/// freezes the per-cycle trust refresh, so a `peer.revoked` never takes effect on the
/// running daemon: a peer you are cutting off could keep itself trusted by stalling your
/// pull (review finding A7b).
const IO_TIMEOUT_SECS: u64 = 30;

/// `tokio::time::timeout` wrapper that turns an elapsed timer into a legible io::Error,
/// so every caller keeps its existing `?`/`context` error plumbing.
async fn with_io_timeout<T>(
    what: &str,
    fut: impl std::future::Future<Output = std::io::Result<T>>,
) -> std::io::Result<T> {
    match tokio::time::timeout(std::time::Duration::from_secs(IO_TIMEOUT_SECS), fut).await {
        Ok(r) => r,
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("{what}: no progress within {IO_TIMEOUT_SECS}s (stalled peer)"),
        )),
    }
}

/// A request on the clinical-federation plane. JSON, one per connection.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "op")]
pub enum Request {
    /// Every `node_event` whose serving-node `seq` is strictly greater than
    /// `after_seq`, in `seq` order. `after_seq = 0` returns the full set (the
    /// full-sweep path). `seq` is the server's LOCAL insertion order — the only
    /// ordering where newly-learned events always sort above a puller's cursor, so
    /// incremental can never silently skip (issue #38). This enum is versioned;
    /// future changes are additive (principle 12).
    NodeEventsAfterSeq { after_seq: i64 },
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
/// Cap on concurrent serve sessions. Without it an UNAUTHENTICATED client can open
/// connections and stall mid-handshake, each parking a task + FD indefinitely
/// (slowloris); with the cap + the handshake timeout in `serve_conn`, a stalled
/// session is bounded in both number and lifetime (review finding A7b).
const MAX_SERVE_SESSIONS: usize = 64;

pub async fn serve(cfg: ServeConfig) -> anyhow::Result<()> {
    let acceptor = TlsAcceptor::from(cfg.tls);
    let sessions = Arc::new(tokio::sync::Semaphore::new(MAX_SERVE_SESSIONS));
    loop {
        let (tcp, peer) = match cfg.listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("serve: accept error: {e}");
                continue;
            }
        };
        // At capacity: shed the newest connection rather than queueing unbounded work.
        // Legitimate peers simply retry next pull cycle; a flood is bounded here.
        let Ok(permit) = sessions.clone().try_acquire_owned() else {
            eprintln!("serve: at {MAX_SERVE_SESSIONS}-session capacity, dropping {peer}");
            continue;
        };
        let acceptor = acceptor.clone();
        let db_conn = cfg.db_conn.clone();
        tokio::spawn(async move {
            let _permit = permit; // held for the session's lifetime
            if let Err(e) = serve_conn(acceptor, tcp, &db_conn).await {
                eprintln!("serve: session with {peer} ended: {e}");
            }
        });
    }
}

/// Handle one accepted connection: complete the mTLS handshake (which pins the
/// client), read one request frame, and stream the requested `node_event` bytes.
async fn serve_conn(acceptor: TlsAcceptor, tcp: TcpStream, db_conn: &str) -> anyhow::Result<()> {
    // Both the handshake and the request read are stall-bounded: an unauthenticated
    // client that connects and goes silent is disconnected, not parked forever.
    let mut tls = with_io_timeout("serve handshake", acceptor.accept(tcp))
        .await
        .context("mTLS handshake (client pin)")?;

    let req_bytes = match with_io_timeout("serve request read", read_frame(&mut tls)).await? {
        Some(b) => b,
        None => return Ok(()), // client connected and hung up without a request
    };
    let req: Request = serde_json::from_slice(&req_bytes).context("decoding request frame")?;

    let db = db::connect(db_conn).await.context("serve: connecting to DB")?;
    match req {
        Request::NodeEventsAfterSeq { after_seq } => {
            stream_node_events(&mut tls, &db, after_seq).await?;
        }
    }
    // Closing the stream is the EOF the puller reads as "no more frames".
    tls.shutdown().await.ok();
    Ok(())
}

/// Stream every `node_event` with `seq > after_seq`, ordered by `seq` (the serving
/// node's local insertion order). Each frame is `[8-byte big-endian seq][signed_bytes]`
/// so the puller can checkpoint its per-peer cursor. The seq prefix is transport
/// metadata only; the signed_bytes are the untouched signed core (principle 12).
/// `after_seq = 0` selects everything (the full-sweep path).
async fn stream_node_events<S: AsyncWriteExt + Unpin>(
    tls: &mut S,
    db: &Client,
    after_seq: i64,
) -> anyhow::Result<()> {
    let rows = db
        .query(
            "SELECT seq, signed_bytes FROM node_event WHERE seq > $1 ORDER BY seq",
            &[&after_seq],
        )
        .await
        .context("selecting node_event bytes to stream")?;
    for row in &rows {
        let seq: i64 = row.get(0);
        let bytes: Vec<u8> = row.get(1);
        // Write-side counterpart of the read-side MAX_FRAME_BYTES cap (review finding
        // A7a): an oversized stored event would be REFUSED by every puller's read cap,
        // and because the puller aborts mid-stream without checkpointing, one such event
        // would wedge the link at this seq forever. The DB doors now reject oversized
        // events at admission, so this fires only for a legacy/hand-inserted row — skip
        // it LOUDLY rather than poisoning the stream for everything after it.
        if 8 + bytes.len() > MAX_FRAME_BYTES {
            eprintln!(
                "serve: node_event seq {seq} is {} bytes, over the {MAX_FRAME_BYTES}-byte \
                 frame cap — skipping (unreplicable; investigate how it was admitted)",
                bytes.len()
            );
            continue;
        }
        // Frame payload = 8-byte BE seq ++ signed_bytes.
        let mut framed = Vec::with_capacity(8 + bytes.len());
        framed.extend_from_slice(&seq.to_be_bytes());
        framed.extend_from_slice(&bytes);
        with_io_timeout("node_event frame write", write_frame(tls, &framed))
            .await
            .context("writing a node_event frame")?;
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

/// Connect to `peer` over pinned mTLS, request node events after this node's cursor for
/// `peer` (or the full set when `full_sweep`), and apply each via the in-DB gate. Opens
/// its own short-lived DB connection; `run` uses [`pull_into`] with its cycle connection.
pub async fn pull_once(peer: SocketAddr, cfg: PullConfig, full_sweep: bool) -> anyhow::Result<PullStats> {
    let db = db::connect(&cfg.db_conn).await.context("pull: connecting to DB")?;
    pull_into(peer, cfg.tls, &db, full_sweep).await
}

/// The pull itself, applying admitted events into an already-open `db`. Reads the
/// per-peer cursor (keyed by the peer ADDRESS — known before connecting, so no protocol
/// round-trip), requests `seq > cursor` (or `> 0` on a full sweep), parses the 8-byte seq
/// prefix from each frame, applies the signed bytes via the unchanged admission gate, and
/// — only at a CLEAN EOF — checkpoints the highest seq received through the advance-only
/// door. A mid-stream failure returns early WITHOUT checkpointing, so the next cycle
/// re-pulls from the last committed cursor and no event is lost (idempotent apply).
pub async fn pull_into(
    peer: SocketAddr,
    tls: Arc<ClientConfig>,
    db: &Client,
    full_sweep: bool,
) -> anyhow::Result<PullStats> {
    let peer_key = peer.to_string();
    // Cursor: 0 on a full sweep (everything) or when we have never pulled this peer.
    let after_seq: i64 = if full_sweep {
        0
    } else {
        db.query_one(
            "SELECT coalesce((SELECT last_seq FROM sync_cursor WHERE peer_addr = $1), 0)",
            &[&peer_key],
        )
        .await
        .context("reading sync cursor")?
        .get(0)
    };

    let connector = TlsConnector::from(tls);
    // Every network step is stall-bounded (review finding A7b): a black-holed address, a
    // peer that accepts TCP then never completes the handshake, or one that goes silent
    // mid-stream all surface as a timeout error — the pull fails LOUDLY (and, in `run`,
    // the loop continues: trust refresh and the next cycle still happen) instead of
    // parking the daemon on a read forever.
    let tcp = with_io_timeout("connect", TcpStream::connect(peer))
        .await
        .with_context(|| format!("connecting to {peer}"))?;
    // The pinned server cert's SAN is "cairn-node" (see transport::node_cert); the
    // ServerName is cosmetic here because the custom verifier pins on the key, not
    // the name, but rustls still requires a syntactically valid name.
    let name = ServerName::try_from("cairn-node").context("building server name")?;
    let mut tls = with_io_timeout("pull handshake", connector.connect(name, tcp))
        .await
        .context("mTLS handshake (server pin)")?;

    let req = Request::NodeEventsAfterSeq { after_seq };
    with_io_timeout("request write", write_frame(&mut tls, &serde_json::to_vec(&req)?))
        .await
        .context("sending request")?;

    let mut stats = PullStats::default();
    let mut max_seq = after_seq;
    while let Some(frame) = with_io_timeout("response frame read", read_frame(&mut tls))
        .await
        .context("reading a response frame")?
    {
        stats.received += 1;
        // Frame = [8-byte BE seq][signed_bytes]. A short frame is a protocol error.
        if frame.len() < 8 {
            anyhow::bail!("pull: response frame shorter than the 8-byte seq prefix");
        }
        let seq = i64::from_be_bytes(frame[..8].try_into().expect("8 bytes"));
        let signed = &frame[8..];
        match db.execute("SELECT apply_remote_node_event($1)", &[&signed]).await {
            Ok(_) => stats.admitted += 1,
            Err(e) => {
                // Non-fatal: the admission gate refused this event (un-trusted
                // author / malformed / fail-closed). Log the legible reason, keep going.
                stats.rejected += 1;
                eprintln!("pull: node_event rejected (non-fatal): {e}");
            }
        }
        // Advance over RECEIVED events (stream is seq-ordered); rejections are re-tried
        // on the next full sweep. Tracking the max — not the last — is robust to any
        // server-side reordering.
        if seq > max_seq { max_seq = seq; }
    }
    // Clean EOF reached: checkpoint through the advance-only door. Only now — a mid-stream
    // error returned above without advancing the cursor.
    if max_seq > after_seq || full_sweep {
        db.execute("SELECT checkpoint_sync_cursor($1,$2)", &[&peer_key, &max_seq])
            .await
            .context("checkpointing sync cursor")?;
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
    let mut cycle: u64 = 0;
    // Snapshot of the trust set as of the previous cycle, to detect peering changes.
    let mut prev_trust: HashSet<String> =
        trust_set.read().map(|s| s.clone()).unwrap_or_default();
    loop {
        ticker.tick().await;
        cycle += 1;
        // ONE DB connection per cycle, used for BOTH the trust refresh and applying
        // the pull's admitted events (previously this opened two short-lived
        // connections per cycle — PR #28 review follow-up). It is dropped at the end
        // of the iteration, so the loop never accumulates connections and a DB
        // restart is picked up by the next cycle's reconnect.
        let cycle_db = match db::connect(&db_conn).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("run: DB unreachable, serving last-known set, skipping pull: {e}");
                if serve_handle.is_finished() {
                    anyhow::bail!("run: serve task exited unexpectedly");
                }
                continue;
            }
        };
        // Re-snapshot the live set so peering changes since the last cycle apply to
        // serve AND pull. A failed refresh is non-fatal: the last-known set stays in
        // force. During a DB outage a pending revocation therefore lands only once
        // the DB is reachable again — the deliberate availability-over-consistency
        // trade (we never halt federation on a transient DB blip); the still-pinned
        // mTLS + in-DB admission gate remain the hard floor regardless.
        if let Err(e) = refresh_trust_set(&cycle_db, &trust_set).await {
            eprintln!("run: trust refresh failed, serving last-known set: {e}");
        }
        // Full sweep on cadence OR whenever the active peer set changed this cycle (so a
        // freshly-peered node's backlog is pulled at once, not after FULL_SWEEP_EVERY).
        let now_trust: HashSet<String> =
            trust_set.read().map(|s| s.clone()).unwrap_or_default();
        let trust_changed = now_trust != prev_trust;
        prev_trust = now_trust;
        // `% == 0` (not `is_multiple_of`, stabilized only in Rust 1.87) keeps this
        // within the workspace MSRV (rust-version = "1.74"); the clippy lint that
        // prefers the newer method is therefore allowed here.
        #[allow(clippy::manual_is_multiple_of)]
        let full_sweep = trust_changed || cycle % FULL_SWEEP_EVERY == 0;

        match pull_into(peer, client_tls.clone(), &cycle_db, full_sweep).await {
            Ok(s) => eprintln!(
                "run: pull {peer}: full_sweep={full_sweep} received={} admitted={} rejected={}",
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
