//! Issue #482, end to end — the three failures where the PEER answered and its answer was
//! unusable must not be logged as link downtime.
//!
//! # Why these are driven through the real code path, not constructed
//!
//! `tests/pull_failure_class.rs` pins the classifier itself over hand-built errors. That is
//! necessary and not sufficient: it would stay green if a production site were reverted to
//! an `anyhow::bail!` or if the handshake stopped carrying its `io::Error`, because the
//! fixtures there are built by the test rather than by `pull_into`. A pin that survives the
//! revert it names is the #387 species this repo has already paid for once (PR #486 review).
//!
//! So each test here makes a REAL pull fail the real way and asks
//! [`pull_failure_class`] what an operator would be told.
//!
//! * the **pin mismatch** needs no stub at all — a client whose trust store denies every
//!   key is exactly the revoked/rotated-peer case, against the node's own real `serve`;
//! * the two **protocol** cases need a peer that answers badly, which no honest `serve`
//!   will do, so they use a minimal hostile stub that completes a genuine pinned mTLS
//!   handshake (the client must get PAST the handshake for the frame checks to be the
//!   thing under test) and then writes one deliberately malformed frame.
//!
//! DB-gated: `pull_into` reads this node's cursor and quarantine floor before it touches
//! the network, so it needs `CAIRN_TEST_PG`.

use std::net::SocketAddr;
use std::sync::Arc;

use cairn_node::sync::{pull_failure_class, pull_into, PullFailureClass};
use cairn_node::transport::TrustStore;
use cairn_node::{db, identity, keystore, sync, transport};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

#[path = "common/db_gate.rs"]
mod db_gate;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The node-plane frame cap `read_frame` enforces (`sync::MAX_FRAME_BYTES`, private).
/// Restated rather than exported: a test that needs a length OVER the cap only needs a
/// number the cap is below, and exporting a constant to let a test read it back is how a
/// guard ends up defined over the thing it guards.
const COMFORTABLY_OVER_THE_NODE_FRAME_CAP: u32 = 64 * 1024 * 1024;

/// A provisioned, self-peered node with its real `serve` running: the same fixture the
/// quarantine suites use, reduced to what these tests need.
struct Node {
    db: tokio_postgres::Client,
    addr: SocketAddr,
    sk: cairn_event::SigningKey,
    trust: TrustStore,
    _tmp: tempfile::TempDir,
}

async fn node(base: &str) -> Node {
    let db = db::connect_and_load_schema(base).await.unwrap();
    db::reset_node_federation_tables(&db).await.ok();
    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    // The listen address recorded in the identity is cosmetic here — every test below
    // dials the address its own listener actually bound.
    identity::provision(&db, &sk, &kid, "A", "127.0.0.1:0")
        .await
        .unwrap();
    let id = identity::load_local(&db).await.unwrap();
    // Self-peer, so this node's own key is `active` in its own trust set and a pinned
    // handshake against itself (or against the stub, which reuses the key) succeeds.
    let bundle = cairn_event::PairingBundle {
        node_id_hex: id.node_id_hex.clone(),
        pubkey_hex: id.pubkey_hex.clone(),
        address: "127.0.0.1:0".into(),
        fingerprint: cairn_event::short_fingerprint(&id.pubkey_hex).unwrap(),
        nonce: "n".into(),
        hlc: cairn_event::Hlc {
            wall: 0,
            counter: 0,
            node_origin: id.node_id_hex.clone(),
        },
    };
    identity::author_peer(&db, &sk, &kid, &id.node_id_hex, &bundle, Some("peer"))
        .await
        .unwrap();
    let trust = sync::trust_store_from_db(&db).await.unwrap();
    let listen: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (addr, serve_cfg) = sync::bind_serve(listen, base, &sk, trust.clone())
        .await
        .unwrap();
    tokio::spawn(sync::serve(serve_cfg));
    Node {
        db,
        addr,
        sk,
        trust,
        _tmp: tmp,
    }
}

/// A peer that completes a real pinned handshake and then answers with `frame` verbatim.
///
/// `frame` is the RAW bytes on the wire — prefix included — so a caller can write a
/// length prefix that no honest `write_frame` would ever produce. The stub reads the
/// request frame first: skipping that read would close the socket under the client's
/// write and turn every one of these tests into an ordinary partition.
async fn hostile_peer(
    sk: &cairn_event::SigningKey,
    trust: TrustStore,
    frame: Vec<u8>,
) -> SocketAddr {
    let tls = transport::server_config(sk, trust).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let Ok((tcp, _)) = listener.accept().await else {
            return;
        };
        let Ok(mut s) = TlsAcceptor::from(tls).accept(tcp).await else {
            return;
        };
        // Drain the request frame (4-byte BE length prefix, then that many bytes).
        let mut len = [0u8; 4];
        if s.read_exact(&mut len).await.is_err() {
            return;
        }
        let mut body = vec![0u8; u32::from_be_bytes(len) as usize];
        if s.read_exact(&mut body).await.is_err() {
            return;
        }
        let _ = s.write_all(&frame).await;
        let _ = s.flush().await;
        // Hold the connection open briefly: dropping it immediately can race the
        // client's read and deliver an EOF (a partition) instead of the bytes.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    });
    addr
}

/// **The sharp case.** A peer key that no longer pins — rotated, revoked, or a different
/// node answering at that address — produced a line indistinguishable from a satellite
/// outage, and the availability figure was charged for it.
///
/// A deny-all trust store IS the revoked-peer case: `trust_peer` no longer has the key
/// `active`, so the client's verifier refuses the server's cert. TCP connect succeeded a
/// moment earlier, which is what makes "go and look at the link" the wrong instruction.
#[tokio::test]
async fn a_pin_mismatch_is_the_peer_not_the_link() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = node(&base).await;

    let deny_all: TrustStore = Arc::new(|_: &str| false);
    let tls = transport::client_config(&n.sk, deny_all).unwrap();
    let err = pull_into(n.addr, tls, &n.db, true)
        .await
        .expect_err("a client that trusts no key cannot pin this server");

    assert_eq!(
        pull_failure_class(&err),
        PullFailureClass::Integrity,
        "a revoked/rotated peer key is a security event, not link downtime: {err:#}"
    );
}

/// A response frame too short to carry its own 8-byte seq prefix: the peer is running
/// incompatible code. The bytes arrived intact, so there is no `io::Error` here at all —
/// this is the case `PeerIntegrityError` exists for, and an `anyhow::bail!` would classify
/// it as a partition by elimination.
#[tokio::test]
async fn a_short_response_frame_is_the_peer_not_the_link() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = node(&base).await;

    // A well-formed 3-byte frame: the prefix is honest, the payload is too short to hold
    // the 8-byte seq the node plane puts in front of every event.
    let mut frame = 3u32.to_be_bytes().to_vec();
    frame.extend_from_slice(b"abc");
    let addr = hostile_peer(&n.sk, n.trust.clone(), frame).await;
    let tls = transport::client_config(&n.sk, n.trust.clone()).unwrap();

    let err = pull_into(addr, tls, &n.db, true)
        .await
        .expect_err("a 3-byte frame cannot carry an 8-byte seq prefix");

    assert_eq!(
        pull_failure_class(&err),
        PullFailureClass::Integrity,
        "the peer answered; its wire format is the problem: {err:#}"
    );
}

/// A length prefix over the node plane's frame cap. `read_frame` refuses it BEFORE
/// allocating (issue #212's rule 1) and returns `InvalidData`, which is the kind
/// `tokio-rustls` also uses for a failed pin — one recogniser covers both.
#[tokio::test]
async fn an_oversized_frame_prefix_is_the_peer_not_the_link() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = node(&base).await;

    // Prefix only, and deliberately no payload: the client must refuse on the number
    // alone. Sending 64 MiB of bytes to prove that would be the opposite of the point.
    let frame = COMFORTABLY_OVER_THE_NODE_FRAME_CAP.to_be_bytes().to_vec();
    let addr = hostile_peer(&n.sk, n.trust.clone(), frame).await;
    let tls = transport::client_config(&n.sk, n.trust.clone()).unwrap();

    let err = pull_into(addr, tls, &n.db, true)
        .await
        .expect_err("an over-cap length prefix is refused before allocating");

    assert_eq!(
        pull_failure_class(&err),
        PullFailureClass::Integrity,
        "an oversized frame is the peer serving garbage, not the link dropping: {err:#}"
    );
}

/// The other direction, and the one that keeps the availability figure honest: a peer that
/// simply is not there must still be a PARTITION. Widening the peer class until it
/// swallowed real outages would be this fix's own mirror-image defect.
#[tokio::test]
async fn a_peer_that_is_not_listening_is_still_a_partition() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = node(&base).await;

    // Bind, read the port, drop the listener: nothing is listening there now.
    let dead: SocketAddr = {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        l.local_addr().unwrap()
    };
    let tls = transport::client_config(&n.sk, n.trust.clone()).unwrap();

    let err = pull_into(dead, tls, &n.db, true)
        .await
        .expect_err("nothing is listening on a closed port");

    assert_eq!(
        pull_failure_class(&err),
        PullFailureClass::Partition,
        "a refused connect is exactly what the partition class is for: {err:#}"
    );
}
