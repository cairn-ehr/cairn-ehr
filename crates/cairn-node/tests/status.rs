use cairn_event::PairingBundle;
use cairn_node::{db, identity, keystore};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

#[tokio::test]
async fn status_reports_peers_and_keystore_health() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap(); // serialize shared-DB tests
    let db = db::connect_and_load_schema(&base).await.unwrap();
    // Re-runnable: truncate before provisioning.
    db.batch_execute("TRUNCATE node_event, local_node")
        .await
        .ok();

    let tmp = tempfile::tempdir().unwrap();
    let key_path = tmp.path().join("node.key");

    // Provision node A with a real keystore file.
    let (sk_a, kid_a) = keystore::generate_plaintext(&key_path).unwrap();
    identity::provision(&db, &sk_a, &kid_a, "A", "127.0.0.1:7900")
        .await
        .unwrap();
    let id_a = identity::load_local(&db).await.unwrap();

    // Add one active peer (B).
    let (sk_b, kid_b) = cairn_event::generate_key().unwrap();
    let b_node_id = hex::encode(cairn_event::event_address(b"B-genesis-status-test"));
    let bundle_b = PairingBundle {
        node_id_hex: b_node_id.clone(),
        pubkey_hex: kid_b.clone(),
        address: "127.0.0.1:7901".into(),
        fingerprint: cairn_event::short_fingerprint(&kid_b).unwrap(),
        nonce: "n1".into(),
        hlc: cairn_event::Hlc {
            wall: 0,
            counter: 0,
            node_origin: b_node_id.clone(),
        },
    };
    identity::author_peer(
        &db,
        &sk_a,
        &kid_a,
        &id_a.node_id_hex,
        &bundle_b,
        Some("peer"),
    )
    .await
    .unwrap();

    // Add one more peer (C) and immediately revoke it.
    let (sk_c, kid_c) = cairn_event::generate_key().unwrap();
    let c_node_id = hex::encode(cairn_event::event_address(b"C-genesis-status-test"));
    let bundle_c = PairingBundle {
        node_id_hex: c_node_id.clone(),
        pubkey_hex: kid_c.clone(),
        address: "127.0.0.1:7902".into(),
        fingerprint: cairn_event::short_fingerprint(&kid_c).unwrap(),
        nonce: "n2".into(),
        hlc: cairn_event::Hlc {
            wall: 0,
            counter: 0,
            node_origin: c_node_id.clone(),
        },
    };
    identity::author_peer(
        &db,
        &sk_a,
        &kid_a,
        &id_a.node_id_hex,
        &bundle_c,
        Some("peer"),
    )
    .await
    .unwrap();
    identity::author_unpeer(&db, &sk_a, &kid_a, &id_a.node_id_hex, &c_node_id)
        .await
        .unwrap();

    // --- Happy path: keystore file exists and loads fine.
    let st = identity::status(&db, &key_path).await.unwrap();
    eprintln!("status (ok key): {:?}", st);

    // Peer counts: 1 active (B), 1 revoked (C).
    assert_eq!(st.peers_active, 1, "expected 1 active peer");
    assert_eq!(st.peers_revoked, 1, "expected 1 revoked peer");
    assert!(st.keystore_ok, "keystore must be ok when key file exists");
    assert!(
        st.dr_escrow.contains("STUBBED"),
        "dr_escrow must surface the ADR-0026 stub, got: {:?}",
        st.dr_escrow
    );

    // Finding 3 (review): key-at-rest posture is surfaced and honest about v1 plaintext.
    assert!(
        st.key_at_rest.contains("PLAINTEXT"),
        "key_at_rest must surface v1 plaintext, got: {:?}",
        st.key_at_rest
    );
    assert!(!st.recovery_escrow, "plaintext key has no recovery escrow");
    // Finding 2 (review): the in-DB floor self-check is populated. Tests connect as a
    // superuser, so the floor is present-but-bypassable here (can raw-INSERT) — assert
    // that exact honest reading rather than pretending the gate binds this connection.
    assert!(
        !st.runtime_role.is_empty(),
        "runtime_role must be populated"
    );
    assert!(
        !st.db_floor_enforced,
        "a superuser test connection must report the floor BYPASSABLE (role {:?})",
        st.runtime_role
    );

    // --- Degraded path: missing key file must NOT error; just flags keystore_ok=false.
    let missing = tmp.path().join("does_not_exist.key");
    let st2 = identity::status(&db, &missing).await.unwrap();
    eprintln!("status (missing key): {:?}", st2);
    assert!(
        !st2.keystore_ok,
        "keystore_ok must be false when key file is missing"
    );
    // Peer counts should still be correct even with a missing key.
    assert_eq!(st2.peers_active, 1);
    assert_eq!(st2.peers_revoked, 1);

    // Suppress unused-variable warnings from generate_key calls.
    let _ = sk_b;
    let _ = sk_c;
}

/// Issue #216 / ADR-0058 — `status` surfaces `cairn_clock_health()`'s honest read of
/// whether this node's local clock has fallen behind its own HLC (e.g. a dead RTC that
/// has nonetheless synced a later event from a peer — HLC merge only ever ratchets
/// `hlc_state.hlc_wall` forward, so this is the one direction the DB alone cannot
/// self-correct). Forcing `hlc_state.hlc_wall` an hour ahead of the real clock
/// simulates exactly that condition; `status()` must surface it in `clock_health`
/// rather than silently treating `hlc_floor` as trustworthy (the ADR-0027 §7
/// honest-assembly fact this task exists to surface).
#[tokio::test]
async fn status_flags_a_clock_behind_its_own_hlc() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let db = db::connect_and_load_schema(&base).await.unwrap();
    db.batch_execute("TRUNCATE node_event, local_node")
        .await
        .ok();

    let tmp = tempfile::tempdir().unwrap();
    let key_path = tmp.path().join("node.key");
    let (sk, kid) = keystore::generate_plaintext(&key_path).unwrap();
    identity::provision(&db, &sk, &kid, "A", "127.0.0.1:7910")
        .await
        .unwrap();

    // Force hlc_state.hlc_wall an hour ahead of the real clock — hlc_state is a
    // singleton row (db/007_node_federation.sql, `WHERE id` selects the sole TRUE row).
    // The test connection loaded the schema (owner-level), so it can write directly
    // here even though the runtime `cairn_node` role cannot (db/007's door-only rule).
    db.execute(
        "UPDATE hlc_state SET hlc_wall = (extract(epoch FROM clock_timestamp()) * 1000)::bigint + 3_600_000 WHERE id",
        &[],
    )
    .await
    .unwrap();

    let st = identity::status(&db, &key_path).await.unwrap();
    eprintln!("status (clock behind): {:?}", st);
    assert!(
        st.clock_health.contains("BEHIND"),
        "a clock an hour behind its own HLC must report BEHIND, got: {:?}",
        st.clock_health
    );

    // Restore the baseline (PR #285 review): hlc_state survives TRUNCATEs (the db/007
    // seed is first-load-only) and the suite shares one serialized DB, so leaving the
    // wall an hour in the future would leak into whichever test file runs next — safe
    // today (#187's drift bound is 24h and measures against clock_timestamp()), but a
    // latent pollution trap. Mirror the companion test's explicit reset.
    db.execute(
        "UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0 WHERE id",
        &[],
    )
    .await
    .unwrap();
}

/// The companion clean case: a clock that has NOT fallen behind its own HLC must
/// report an honest "ok" reading, never the BEHIND warning — the same false-positive
/// concern every honest-degradation field in `Status` is held to elsewhere in this file.
#[tokio::test]
async fn status_reports_clock_ok_when_not_behind() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let db = db::connect_and_load_schema(&base).await.unwrap();
    db.batch_execute("TRUNCATE node_event, local_node")
        .await
        .ok();
    // hlc_state resets to (TRUE, 0, 0) via db/007's ON CONFLICT DO NOTHING seed only on
    // first load; TRUNCATE does not touch it, so explicitly reset it here for a
    // deterministic "not behind" baseline regardless of what earlier tests left behind.
    db.execute(
        "UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0 WHERE id",
        &[],
    )
    .await
    .unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let key_path = tmp.path().join("node.key");
    let (sk, kid) = keystore::generate_plaintext(&key_path).unwrap();
    identity::provision(&db, &sk, &kid, "A", "127.0.0.1:7911")
        .await
        .unwrap();

    let st = identity::status(&db, &key_path).await.unwrap();
    eprintln!("status (clock ok): {:?}", st);
    assert!(
        !st.clock_health.contains("BEHIND"),
        "a clock at/behind wall-clock time must not report BEHIND, got: {:?}",
        st.clock_health
    );
    assert!(
        !st.clock_health.is_empty(),
        "clock_health must be populated"
    );
}

/// `status` must NOT crash when run before `init` (no `local_node` row yet).
/// An operator inspecting a freshly-created-but-unprovisioned node should get an
/// honest "uninitialized" reading, not a `query_one` "expected one row" error —
/// the same honest-degradation contract `keystore_ok` already follows. (HANDOVER:
/// "status crashes if run before init".)
#[tokio::test]
async fn status_before_init_degrades_gracefully() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let db = db::connect_and_load_schema(&base).await.unwrap();
    // Un-provisioned node: schema loaded, but no genesis enrollment.
    db.batch_execute("TRUNCATE node_event, local_node")
        .await
        .ok();

    let tmp = tempfile::tempdir().unwrap();
    let key_path = tmp.path().join("node.key");

    // Must return Ok, not error.
    let st = identity::status(&db, &key_path)
        .await
        .expect("status before init must not error");
    eprintln!("status (uninitialized): {:?}", st);

    assert!(
        !st.initialized,
        "an un-provisioned node must report initialized=false"
    );
    assert_eq!(st.peers_active, 0, "no peers before init");
    assert_eq!(st.peers_revoked, 0, "no peers before init");
    // The floor self-check does not depend on local_node, so it must still populate.
    assert!(
        !st.runtime_role.is_empty(),
        "runtime_role must populate even uninitialized"
    );
}
