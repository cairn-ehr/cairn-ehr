//! Task 10 — `node_event` set-union sync over the Task 9 mTLS transport.
//!
//! Proves ONE direction end-to-end: node B, having mutually peered with node A,
//! pulls A's events over a pinned mTLS session and admits A's genesis enroll. The
//! full bidirectional convergence (both directions, watermarks) is Task 12.
//!
//! mTLS is mutual, so BOTH nodes must peer with each other before B can pull:
//!   * A's server pins connecting clients to A's `trust_peer` → A must hold peer(B).
//!   * B's client pins A's server cert                       → B must hold peer(A).
//! The test therefore establishes mutual peering (each node authors `peer.added`
//! for the other, using the other's real node_id + pubkey + fingerprint) BEFORE
//! the pull.
//!
//! Skips unless BOTH `CAIRN_TEST_PG` (node A) and `CAIRN_TEST_PG2` (node B) are set.

use cairn_node::{db, identity, keystore, sync};
use std::net::SocketAddr;

fn cs_a() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }
fn cs_b() -> Option<String> { std::env::var("CAIRN_TEST_PG2").ok() }

/// Hand-build a `PairingBundle` for `peer` (node X) so node Y can author
/// `peer.added(X)` from X's real node_id + pubkey + fingerprint.
fn bundle_for(node_id_hex: &str, pubkey_hex: &str, address: &str) -> cairn_event::PairingBundle {
    cairn_event::PairingBundle {
        node_id_hex: node_id_hex.into(),
        pubkey_hex: pubkey_hex.into(),
        address: address.into(),
        fingerprint: cairn_event::short_fingerprint(pubkey_hex).unwrap(),
        nonce: "n".into(),
        hlc: cairn_event::Hlc { wall: 0, counter: 0, node_origin: node_id_hex.into() },
    }
}

#[tokio::test]
async fn b_pulls_and_admits_a_genesis_over_mtls() {
    let (Some(base_a), Some(base_b)) = (cs_a(), cs_b()) else {
        eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
        return;
    };

    // --- provision both nodes in their own fresh DBs ---
    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    a.batch_execute("TRUNCATE node_event, local_node").await.ok();
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    b.batch_execute("TRUNCATE node_event, local_node").await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk_a, kid_a) = keystore::generate_and_seal(&tmp.path().join("a.key"), None).unwrap();
    let (sk_b, kid_b) = keystore::generate_and_seal(&tmp.path().join("b.key"), None).unwrap();
    identity::provision(&a, &sk_a, &kid_a, "A", "127.0.0.1:7800").await.unwrap();
    identity::provision(&b, &sk_b, &kid_b, "B", "127.0.0.1:7801").await.unwrap();

    let id_a = identity::load_local(&a).await.unwrap();
    let id_b = identity::load_local(&b).await.unwrap();

    // --- mutual peering (mTLS is mutual) ---
    // A authors peer.added(B); B authors peer.added(A).
    let bundle_b = bundle_for(&id_b.node_id_hex, &id_b.pubkey_hex, &id_b.address);
    identity::author_peer(&a, &sk_a, &kid_a, &id_a.node_id_hex, &bundle_b, Some("peer"))
        .await.unwrap();
    let bundle_a = bundle_for(&id_a.node_id_hex, &id_a.pubkey_hex, &id_a.address);
    identity::author_peer(&b, &sk_b, &kid_b, &id_b.node_id_hex, &bundle_a, Some("peer"))
        .await.unwrap();

    // --- build TrustStores from each DB's active peer set (snapshot) ---
    let trust_a = sync::trust_store_from_db(&a).await.unwrap();
    let trust_b = sync::trust_store_from_db(&b).await.unwrap();
    // Sanity: A trusts B's key, B trusts A's key.
    assert!(trust_a(&kid_b), "A must pin B's key");
    assert!(trust_b(&kid_a), "B must pin A's key");

    // --- stand up A's serve task on an ephemeral port ---
    let listen: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (addr, serve_cfg) =
        sync::bind_serve(listen, &base_a, &sk_a, trust_a.clone()).await.unwrap();
    let serve = tokio::spawn(sync::serve(serve_cfg));

    // --- B pulls from A over mTLS ---
    let client_cfg = sync::client_config(&base_b, &sk_b, trust_b).await.unwrap();
    let stats = sync::pull_once(addr, client_cfg).await.unwrap();
    eprintln!("pull stats: {stats:?}");
    assert!(stats.received >= 1, "B must receive at least A's genesis frame");

    // B now holds 2 enroll rows: its own genesis + A's, admitted over mTLS.
    let n: i64 = b
        .query_one("SELECT count(*) FROM node_event WHERE op='enroll'", &[])
        .await.unwrap().get(0);
    assert_eq!(n, 2, "B must hold its own + A's genesis enroll after the pull");

    serve.abort();
}
