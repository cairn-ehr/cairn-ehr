//! Task 10 — `node_event` set-union sync over the Task 9 mTLS transport.
//!
//! Proves ONE direction end-to-end: node B, having mutually peered with node A,
//! pulls A's events over a pinned mTLS session and admits A's genesis enroll. The
//! full bidirectional convergence (both directions, watermarks) is Task 12.
//!
//! mTLS is mutual, so BOTH nodes must peer with each other before B can pull:
//!   * A's server pins connecting clients to A's `trust_peer` → A must hold peer(B).
//!   * B's client pins A's server cert                       → B must hold peer(A).
//!
//! The test therefore establishes mutual peering (each node authors `peer.added`
//! for the other, using the other's real node_id + pubkey + fingerprint) BEFORE
//! the pull.
//!
//! Skips unless BOTH `CAIRN_TEST_PG` (node A) and `CAIRN_TEST_PG2` (node B) are set.

use cairn_node::{db, identity, keystore, sync};
use std::net::SocketAddr;

fn cs_a() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }
fn cs_b() -> Option<String> { std::env::var("CAIRN_TEST_PG2").ok() }
fn cs_c() -> Option<String> { std::env::var("CAIRN_TEST_PG3").ok() }

// Both DB-gated tests in this file share node A's (`CAIRN_TEST_PG`) database (and
// B's/C's) and each begins by TRUNCATEing, so they must not interleave with each
// other OR with the other shared-DB test binaries. `db::test_serial_guard` (a
// Postgres advisory lock on CAIRN_TEST_PG) serializes them cluster-wide, so the run
// command needs no `--test-threads=1`.

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
    let _guard = db::test_serial_guard(&base_a).await.unwrap(); // serialize shared-DB tests

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

/// Count node_event rows by `op` (enroll / peer / revoke) on one node's DB.
async fn count_op(db: &tokio_postgres::Client, op: &str) -> i64 {
    db.query_one("SELECT count(*) FROM node_event WHERE op=$1", &[&op])
        .await.unwrap().get(0)
}

/// THE SLICE ACCEPTANCE GATE.
///
/// One end-to-end pass over the whole federation slice:
///   * two mutually-peered nodes A and B converge by bidirectional `pull_once`
///     (set-union: each DB ends holding both genesis enrolls + both peer events),
///   * a third node C that nobody peered with is rejected — B's pull against C
///     admits nothing (mTLS pin fails or zero rows transfer), B's counts unchanged,
///   * after A `author_unpeer(B)`, a FRESH B-signed peer event arriving at A is
///     denied by the in-DB admission gate (B is no longer an active peer in A).
///
/// Every assertion is on the RECEIVING node's DB, after TRUNCATE, so an admitted
/// row can only have crossed the wire — no vacuous passes.
///
/// Skips unless all three of CAIRN_TEST_PG / CAIRN_TEST_PG2 / CAIRN_TEST_PG3 point
/// at fresh throwaway databases, each with `cairn_pgx` installed.
#[tokio::test]
async fn two_nodes_converge_then_unpeer_and_a_stranger_is_rejected() {
    let (Some(base_a), Some(base_b), Some(base_c)) = (cs_a(), cs_b(), cs_c()) else {
        eprintln!("skipped: set CAIRN_TEST_PG, CAIRN_TEST_PG2 and CAIRN_TEST_PG3");
        return;
    };
    let _guard = db::test_serial_guard(&base_a).await.unwrap(); // serialize shared-DB tests

    // --- 1. provision A, B, C in their own fresh DBs ---
    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    a.batch_execute("TRUNCATE node_event, local_node").await.ok();
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    b.batch_execute("TRUNCATE node_event, local_node").await.ok();
    let c = db::connect_and_load_schema(&base_c).await.unwrap();
    c.batch_execute("TRUNCATE node_event, local_node").await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk_a, kid_a) = keystore::generate_and_seal(&tmp.path().join("a.key"), None).unwrap();
    let (sk_b, kid_b) = keystore::generate_and_seal(&tmp.path().join("b.key"), None).unwrap();
    let (sk_c, kid_c) = keystore::generate_and_seal(&tmp.path().join("c.key"), None).unwrap();
    identity::provision(&a, &sk_a, &kid_a, "A", "127.0.0.1:7810").await.unwrap();
    identity::provision(&b, &sk_b, &kid_b, "B", "127.0.0.1:7811").await.unwrap();
    identity::provision(&c, &sk_c, &kid_c, "C", "127.0.0.1:7812").await.unwrap();

    let id_a = identity::load_local(&a).await.unwrap();
    let id_b = identity::load_local(&b).await.unwrap();

    // --- 2. MUTUAL peering between A and B (mTLS is mutual; fingerprints are
    //        confirmed in-test by passing the real bundle, bypassing the prompt). ---
    let bundle_b = bundle_for(&id_b.node_id_hex, &id_b.pubkey_hex, &id_b.address);
    identity::author_peer(&a, &sk_a, &kid_a, &id_a.node_id_hex, &bundle_b, Some("peer"))
        .await.unwrap();
    let bundle_a = bundle_for(&id_a.node_id_hex, &id_a.pubkey_hex, &id_a.address);
    identity::author_peer(&b, &sk_b, &kid_b, &id_b.node_id_hex, &bundle_a, Some("peer"))
        .await.unwrap();

    // C is provisioned but NOBODY peered with C, and C peers with nobody.

    // Trust snapshots from each DB's active peer set.
    let trust_a = sync::trust_store_from_db(&a).await.unwrap();
    let trust_b = sync::trust_store_from_db(&b).await.unwrap();
    let trust_c = sync::trust_store_from_db(&c).await.unwrap();
    assert!(trust_a(&kid_b), "A must pin B's key");
    assert!(trust_b(&kid_a), "B must pin A's key");
    assert!(!trust_c(&kid_a) && !trust_c(&kid_b), "C trusts nobody");

    // --- stand up serve tasks for A, B, C on ephemeral ports ---
    let listen: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (addr_a, serve_cfg_a) =
        sync::bind_serve(listen, &base_a, &sk_a, trust_a.clone()).await.unwrap();
    let (addr_b, serve_cfg_b) =
        sync::bind_serve(listen, &base_b, &sk_b, trust_b.clone()).await.unwrap();
    let (addr_c, serve_cfg_c) =
        sync::bind_serve(listen, &base_c, &sk_c, trust_c.clone()).await.unwrap();
    let serve_a = tokio::spawn(sync::serve(serve_cfg_a));
    let serve_b = tokio::spawn(sync::serve(serve_cfg_b));
    let serve_c = tokio::spawn(sync::serve(serve_cfg_c));

    // --- 3. bidirectional pull: A←B and B←A ---
    // B pulls from A.
    let cfg = sync::client_config(&base_b, &sk_b, trust_b.clone()).await.unwrap();
    let s_ba = sync::pull_once(addr_a, cfg).await.unwrap();
    eprintln!("B<-A pull: {s_ba:?}");
    // A pulls from B.
    let cfg = sync::client_config(&base_a, &sk_a, trust_a.clone()).await.unwrap();
    let s_ab = sync::pull_once(addr_b, cfg).await.unwrap();
    eprintln!("A<-B pull: {s_ab:?}");

    // Convergence: BOTH DBs hold 2 enroll + 2 peer rows (set-union). Assert on each
    // RECEIVING node after TRUNCATE, so admitted rows can only have crossed the wire.
    assert_eq!(count_op(&a, "enroll").await, 2, "A must hold both genesis enrolls");
    assert_eq!(count_op(&a, "peer").await,   2, "A must hold both peer.added events");
    assert_eq!(count_op(&b, "enroll").await, 2, "B must hold both genesis enrolls");
    assert_eq!(count_op(&b, "peer").await,   2, "B must hold both peer.added events");

    // list_peers on each shows the other as `active`.
    let peers_a = identity::list_peers(&a).await.unwrap();
    let peers_b = identity::list_peers(&b).await.unwrap();
    assert!(
        peers_a.iter().any(|p| p.peer_node_id_hex == id_b.node_id_hex && p.status == "active"),
        "A's trust_peer must show B active"
    );
    assert!(
        peers_b.iter().any(|p| p.peer_node_id_hex == id_a.node_id_hex && p.status == "active"),
        "B's trust_peer must show A active"
    );

    // --- 4. THE STRANGER: B pulls from C (nobody peered with C). ---
    // mTLS must fail (C's server pins its empty trust set; B's client doesn't trust
    // C). The pull returns Err OR transfers zero admitted rows; either way B's row
    // counts are UNCHANGED from step 3. Snapshot first, then wrap the expected
    // failure so the test asserts post-state rather than aborting.
    let b_enroll_before = count_op(&b, "enroll").await;
    let b_peer_before   = count_op(&b, "peer").await;

    let cfg = sync::client_config(&base_b, &sk_b, trust_b.clone()).await.unwrap();
    match sync::pull_once(addr_c, cfg).await {
        Ok(s) => {
            eprintln!("B<-C pull (stranger) unexpectedly returned Ok: {s:?}");
            // If the handshake somehow completed, the in-DB gate must still admit
            // nothing from an un-peered author.
            assert_eq!(s.admitted, 0, "B must admit ZERO rows from the stranger C");
        }
        Err(e) => eprintln!("B<-C pull (stranger) rejected at mTLS as expected: {e}"),
    }
    assert_eq!(count_op(&b, "enroll").await, b_enroll_before, "stranger C must not add enroll rows to B");
    assert_eq!(count_op(&b, "peer").await,   b_peer_before,   "stranger C must not add peer rows to B");

    // --- 5. UNPEER: A revokes B, then a FRESH B-signed peer event must be denied. ---
    identity::author_unpeer(&a, &sk_a, &kid_a, &id_a.node_id_hex, &id_b.node_id_hex).await.unwrap();
    let peers_a = identity::list_peers(&a).await.unwrap();
    assert!(
        peers_a.iter().any(|p| p.peer_node_id_hex == id_b.node_id_hex && p.status == "revoked"),
        "A's trust_peer must show B revoked after author_unpeer"
    );

    // B authors a NEW peer event (peer.added for a synthetic third bundle D). This is
    // a fresh B-signed node_event A has never seen. After the unpeer, B's author node
    // is no longer active in A's trust set, so A's admission gate must reject it.
    let bundle_d = bundle_for(
        // a plausible-but-unknown 32-byte content-address + key; never trusted by A.
        &"dd".repeat(34),
        &"ee".repeat(32),
        "127.0.0.1:7899",
    );
    identity::author_peer(&b, &sk_b, &kid_b, &id_b.node_id_hex, &bundle_d, Some("peer"))
        .await.unwrap();

    // A re-pulls from B. The handshake still succeeds (B never unpeered A, so B's
    // server still pins A; A's client still pins B's server cert — TLS pinning is a
    // snapshot from step 2 and is symmetric here). The REJECTION is at A's in-DB
    // admission gate, which is exactly what we want to prove: trust is enforced in
    // the DB, not only at the transport.
    let a_peer_before = count_op(&a, "peer").await;
    let cfg = sync::client_config(&base_a, &sk_a, trust_a.clone()).await.unwrap();
    let s_unpeer = sync::pull_once(addr_b, cfg).await.unwrap();
    eprintln!("A<-B post-unpeer pull: {s_unpeer:?}");
    assert!(
        s_unpeer.rejected >= 1,
        "A must REJECT at least the new B-authored peer event after unpeering B (rejected={})",
        s_unpeer.rejected
    );
    // No new peer row landed for that event: A's peer count is unchanged.
    assert_eq!(
        count_op(&a, "peer").await, a_peer_before,
        "no new peer row may land on A from an un-trusted (unpeered) author"
    );

    serve_a.abort();
    serve_b.abort();
    serve_c.abort();
}
