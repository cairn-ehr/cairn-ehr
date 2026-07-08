//! Issue #38 (Gap 4) — genesis/peer HLC must be REAL, not the 0/0 placeholder.
//! DB-gated: needs CAIRN_TEST_PG.

use cairn_event::verify_self_described;
use cairn_node::{db, identity, keystore};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Genesis enroll carries a real, non-zero HLC wall, and a later peer.added carries a
/// strictly greater HLC than the genesis (the local clock advances per authored event).
#[tokio::test]
async fn genesis_hlc_is_nonzero_and_advances() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk, &kid, "A", "127.0.0.1:7902")
        .await
        .unwrap();

    // Genesis HLC stored in node_event must be non-zero.
    let genesis: (i64, i32) = {
        let r = a
            .query_one(
                "SELECT hlc_wall, hlc_counter FROM node_event WHERE op='enroll'",
                &[],
            )
            .await
            .unwrap();
        (r.get("hlc_wall"), r.get("hlc_counter"))
    };
    assert!(
        genesis.0 > 0,
        "genesis hlc_wall must be real, got {}",
        genesis.0
    );

    // Author a peer.added against a hand-built bundle; its HLC must exceed genesis.
    let id = identity::load_local(&a).await.unwrap();
    let bundle = cairn_event::PairingBundle {
        node_id_hex: id.node_id_hex.clone(),
        pubkey_hex: id.pubkey_hex.clone(),
        address: "127.0.0.1:7903".into(),
        fingerprint: cairn_event::short_fingerprint(&id.pubkey_hex).unwrap(),
        nonce: "n".into(),
        hlc: cairn_event::Hlc {
            wall: 0,
            counter: 0,
            node_origin: id.node_id_hex.clone(),
        },
    };
    identity::author_peer(&a, &sk, &kid, &id.node_id_hex, &bundle, Some("peer"))
        .await
        .unwrap();

    let peer_hlc: (i64, i32) = {
        let r = a
            .query_one(
                "SELECT hlc_wall, hlc_counter FROM node_event WHERE op='peer'",
                &[],
            )
            .await
            .unwrap();
        (r.get("hlc_wall"), r.get("hlc_counter"))
    };
    assert!(
        peer_hlc > genesis,
        "peer HLC {peer_hlc:?} must exceed genesis {genesis:?}"
    );

    // And the signed body's HLC matches what was stored (the tick happened before signing).
    let signed: Vec<u8> = a
        .query_one("SELECT signed_bytes FROM node_event WHERE op='enroll'", &[])
        .await
        .unwrap()
        .get(0);
    let body = verify_self_described(&signed).unwrap();
    assert_eq!(body.hlc.wall, genesis.0, "signed genesis HLC == stored");
}
