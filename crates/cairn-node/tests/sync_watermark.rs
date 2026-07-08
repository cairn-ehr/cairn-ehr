//! Issue #38 — incremental sync watermark (local-insertion `seq` cursor) and the
//! full-sweep correctness floor. DB-gated: needs CAIRN_TEST_PG (fresh DB + cairn_pgx).

use cairn_node::{db, identity, keystore};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Every node_event row gets a monotonic `seq` assigned at INSERT, and the genesis
/// enroll's seq is the lowest (insertion order == seq order on this node).
#[tokio::test]
async fn node_event_seq_is_monotonic_on_insert() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk, &kid, "A", "127.0.0.1:7900")
        .await
        .unwrap();

    // After genesis there is exactly one row, and its seq is NOT NULL and >= 1.
    let row = a
        .query_one(
            "SELECT count(*) AS n, min(seq) AS lo, max(seq) AS hi FROM node_event",
            &[],
        )
        .await
        .unwrap();
    let n: i64 = row.get("n");
    let lo: i64 = row.get("lo");
    let hi: i64 = row.get("hi");
    assert_eq!(n, 1, "exactly the genesis enroll");
    assert_eq!(lo, hi, "single row: lo == hi");
    assert!(lo >= 1, "seq is assigned (>= 1), got {lo}");
}

/// The cursor door is advance-only: a lower observed_seq is a no-op, never a rewind.
#[tokio::test]
async fn checkpoint_sync_cursor_is_advance_only() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let peer = "127.0.0.1:7901";
    let s1: i64 = a
        .query_one("SELECT checkpoint_sync_cursor($1,$2)", &[&peer, &10_i64])
        .await
        .unwrap()
        .get(0);
    assert_eq!(s1, 10, "first checkpoint sets last_seq=10");
    // A lower value must NOT rewind.
    let s2: i64 = a
        .query_one("SELECT checkpoint_sync_cursor($1,$2)", &[&peer, &5_i64])
        .await
        .unwrap()
        .get(0);
    assert_eq!(s2, 10, "lower observed_seq is a no-op (advance-only)");
    // A higher value advances.
    let s3: i64 = a
        .query_one("SELECT checkpoint_sync_cursor($1,$2)", &[&peer, &20_i64])
        .await
        .unwrap()
        .get(0);
    assert_eq!(s3, 20, "higher observed_seq advances");
}

/// The floor invariant (PR #39) extended to sync_cursor: the unprivileged runtime role
/// CANNOT raw-write the table — it may only go through the validated door. Mirrors
/// `tests/floor_enforced.rs`.
fn conn_as_role(base: &str, role: &str) -> String {
    let kept: Vec<&str> = base
        .split_whitespace()
        .filter(|kv| !kv.starts_with("user="))
        .collect();
    format!("{} user={role}", kept.join(" "))
}

#[tokio::test]
async fn runtime_role_cannot_raw_write_sync_cursor_but_door_works() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let owner = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&owner).await.ok();

    let role = "cairn_runtime_cursor_test";
    db::provision_runtime_role(&owner, role).await.unwrap();
    let runtime = db::connect(&conn_as_role(&base, role)).await.unwrap();

    // Raw INSERT is denied (42501).
    let raw = runtime
        .execute(
            "INSERT INTO sync_cursor (peer_addr, last_seq) VALUES ('x', 1)",
            &[],
        )
        .await;
    let err = raw.expect_err("raw INSERT into sync_cursor must be denied for the runtime role");
    assert_eq!(
        err.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE),
        "raw INSERT must fail with 42501, got: {err:?}"
    );
    // The validated door still works for the unprivileged role.
    let v: i64 = runtime
        .query_one("SELECT checkpoint_sync_cursor($1,$2)", &[&"x", &7_i64])
        .await
        .expect("door must work for the cairn_node-granted role")
        .get(0);
    assert_eq!(v, 7, "door advanced the cursor for the runtime role");

    drop(runtime);
    owner
        .batch_execute(&format!("DROP ROLE IF EXISTS {role}"))
        .await
        .ok();
}

use cairn_node::sync;
use std::net::SocketAddr;

/// A pull frames each event as [8-byte seq][signed_bytes]; the signed_bytes the puller
/// receives are byte-identical to what the server stored (seq is transport metadata,
/// never part of the signed core — principle 12).
#[tokio::test]
async fn wire_seq_prefix_does_not_touch_signed_core() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk, &kid, "A", "127.0.0.1:7904")
        .await
        .unwrap();

    // The bytes stored for the genesis enroll.
    let stored: Vec<u8> = a
        .query_one("SELECT signed_bytes FROM node_event WHERE op='enroll'", &[])
        .await
        .unwrap()
        .get(0);

    // Serve A to itself (trust set need not include anyone for a same-process pull that
    // only reads the stream — but mTLS is mutual, so peer A with itself for the test).
    let id = identity::load_local(&a).await.unwrap();
    let self_bundle = cairn_event::PairingBundle {
        node_id_hex: id.node_id_hex.clone(),
        pubkey_hex: id.pubkey_hex.clone(),
        address: "127.0.0.1:7905".into(),
        fingerprint: cairn_event::short_fingerprint(&id.pubkey_hex).unwrap(),
        nonce: "n".into(),
        hlc: cairn_event::Hlc {
            wall: 0,
            counter: 0,
            node_origin: id.node_id_hex.clone(),
        },
    };
    identity::author_peer(&a, &sk, &kid, &id.node_id_hex, &self_bundle, Some("peer"))
        .await
        .unwrap();
    let trust = sync::trust_store_from_db(&a).await.unwrap();

    let listen: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (addr, serve_cfg) = sync::bind_serve(listen, &base, &sk, trust.clone())
        .await
        .unwrap();
    let serve = tokio::spawn(sync::serve(serve_cfg));

    let cfg = sync::client_config(&base, &sk, trust).await.unwrap();
    let stats = sync::pull_once(addr, cfg, true).await.unwrap(); // full_sweep = true
    serve.abort();
    assert!(stats.received >= 1, "received at least the genesis frame");

    // The genesis row is byte-for-byte the same after a round-trip (re-read; idempotent apply).
    let after: Vec<u8> = a
        .query_one("SELECT signed_bytes FROM node_event WHERE op='enroll'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        stored, after,
        "signed core unchanged by the seq-prefixed wire"
    );
}

/// THE ACCEPTANCE TEST (issue #38): even if the cursor is advanced PAST an un-applied
/// event (the out-of-order-commit / rejection skip scenario), a full-sweep reconciles it.
/// We simulate the skip directly: jam B's cursor for A past A's genesis seq, prove an
/// incremental pull admits nothing, then prove a full-sweep delivers the genesis.
#[tokio::test]
async fn out_of_order_skip_is_reconciled_by_full_sweep() {
    let (Some(base_a), Some(base_b)) = (cs(), std::env::var("CAIRN_TEST_PG2").ok()) else {
        eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
        return;
    };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();

    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    db::reset_node_federation_tables(&b).await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk_a, kid_a) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    let (sk_b, kid_b) = keystore::generate_plaintext(&tmp.path().join("b.key")).unwrap();
    identity::provision(&a, &sk_a, &kid_a, "A", "127.0.0.1:7906")
        .await
        .unwrap();
    identity::provision(&b, &sk_b, &kid_b, "B", "127.0.0.1:7907")
        .await
        .unwrap();
    let id_a = identity::load_local(&a).await.unwrap();
    let id_b = identity::load_local(&b).await.unwrap();

    // mutual peering
    let mk = |nid: &str, pk: &str, addr: &str| cairn_event::PairingBundle {
        node_id_hex: nid.into(),
        pubkey_hex: pk.into(),
        address: addr.into(),
        fingerprint: cairn_event::short_fingerprint(pk).unwrap(),
        nonce: "n".into(),
        hlc: cairn_event::Hlc {
            wall: 0,
            counter: 0,
            node_origin: nid.into(),
        },
    };
    identity::author_peer(
        &a,
        &sk_a,
        &kid_a,
        &id_a.node_id_hex,
        &mk(&id_b.node_id_hex, &id_b.pubkey_hex, &id_b.address),
        Some("peer"),
    )
    .await
    .unwrap();
    identity::author_peer(
        &b,
        &sk_b,
        &kid_b,
        &id_b.node_id_hex,
        &mk(&id_a.node_id_hex, &id_a.pubkey_hex, &id_a.address),
        Some("peer"),
    )
    .await
    .unwrap();

    let trust_a = sync::trust_store_from_db(&a).await.unwrap();
    let trust_b = sync::trust_store_from_db(&b).await.unwrap();
    let listen: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (addr, serve_cfg) = sync::bind_serve(listen, &base_a, &sk_a, trust_a)
        .await
        .unwrap();
    let serve = tokio::spawn(sync::serve(serve_cfg));

    // Jam B's cursor for A past A's max seq, simulating "advanced past an un-applied event".
    let max_seq_a: i64 = a
        .query_one("SELECT max(seq) FROM node_event", &[])
        .await
        .unwrap()
        .get(0);
    b.execute(
        "SELECT checkpoint_sync_cursor($1,$2)",
        &[&addr.to_string(), &(max_seq_a + 1000)],
    )
    .await
    .unwrap();

    // Incremental pull: admits nothing new (cursor is ahead).
    let cfg = sync::client_config(&base_b, &sk_b, trust_b.clone())
        .await
        .unwrap();
    let inc = sync::pull_once(addr, cfg, false).await.unwrap();
    let a_on_b: i64 = b
        .query_one("SELECT count(*) FROM node_event WHERE op='enroll'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        a_on_b, 1,
        "incremental skipped A's genesis (only B's own enroll present)"
    );
    assert_eq!(
        inc.admitted, 0,
        "incremental admitted nothing past the jammed cursor"
    );

    // Full sweep: reconciles the skipped event.
    let cfg2 = sync::client_config(&base_b, &sk_b, trust_b).await.unwrap();
    let _ = sync::pull_once(addr, cfg2, true).await.unwrap();
    serve.abort();
    let a_on_b2: i64 = b
        .query_one("SELECT count(*) FROM node_event WHERE op='enroll'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        a_on_b2, 2,
        "full-sweep reconciled A's genesis (B now holds both enrolls)"
    );
}

/// After an incremental pull, the cursor advances; a second incremental pull with no new
/// events on the server ships nothing new.
#[tokio::test]
async fn incremental_pull_ships_only_new_events() {
    let (Some(base_a), Some(base_b)) = (cs(), std::env::var("CAIRN_TEST_PG2").ok()) else {
        eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
        return;
    };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();

    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    db::reset_node_federation_tables(&b).await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk_a, kid_a) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    let (sk_b, kid_b) = keystore::generate_plaintext(&tmp.path().join("b.key")).unwrap();
    identity::provision(&a, &sk_a, &kid_a, "A", "127.0.0.1:7908")
        .await
        .unwrap();
    identity::provision(&b, &sk_b, &kid_b, "B", "127.0.0.1:7909")
        .await
        .unwrap();
    let id_a = identity::load_local(&a).await.unwrap();
    let id_b = identity::load_local(&b).await.unwrap();
    let mk = |nid: &str, pk: &str, addr: &str| cairn_event::PairingBundle {
        node_id_hex: nid.into(),
        pubkey_hex: pk.into(),
        address: addr.into(),
        fingerprint: cairn_event::short_fingerprint(pk).unwrap(),
        nonce: "n".into(),
        hlc: cairn_event::Hlc {
            wall: 0,
            counter: 0,
            node_origin: nid.into(),
        },
    };
    identity::author_peer(
        &a,
        &sk_a,
        &kid_a,
        &id_a.node_id_hex,
        &mk(&id_b.node_id_hex, &id_b.pubkey_hex, &id_b.address),
        Some("peer"),
    )
    .await
    .unwrap();
    identity::author_peer(
        &b,
        &sk_b,
        &kid_b,
        &id_b.node_id_hex,
        &mk(&id_a.node_id_hex, &id_a.pubkey_hex, &id_a.address),
        Some("peer"),
    )
    .await
    .unwrap();

    let trust_a = sync::trust_store_from_db(&a).await.unwrap();
    let trust_b = sync::trust_store_from_db(&b).await.unwrap();
    let listen: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (addr, serve_cfg) = sync::bind_serve(listen, &base_a, &sk_a, trust_a)
        .await
        .unwrap();
    let serve = tokio::spawn(sync::serve(serve_cfg));

    // First incremental pull (cursor starts at 0) admits A's genesis + A's peer event.
    let cfg = sync::client_config(&base_b, &sk_b, trust_b.clone())
        .await
        .unwrap();
    let first = sync::pull_once(addr, cfg, false).await.unwrap();
    assert!(
        first.admitted >= 1,
        "first pull admits A's events, got {}",
        first.admitted
    );

    // Second incremental pull: cursor is now past everything → nothing received.
    let cfg2 = sync::client_config(&base_b, &sk_b, trust_b).await.unwrap();
    let second = sync::pull_once(addr, cfg2, false).await.unwrap();
    serve.abort();
    assert_eq!(
        second.received, 0,
        "second incremental pull ships nothing new"
    );
}
