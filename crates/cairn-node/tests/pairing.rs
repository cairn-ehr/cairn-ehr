use cairn_node::{db, identity, keystore, pairing};

fn conn_str() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

#[tokio::test]
async fn pairing_records_an_active_peer_and_unpeer_revokes_it() {
    let Some(cs) = conn_str() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&cs).await.unwrap(); // serialize shared-DB tests
                                                            // Node A in this DB; "node B" is just a second keypair + a hand-built offer.
    let a = db::connect_and_load_schema(&cs).await.unwrap();
    // Re-runnable: truncate before provisioning.
    a.batch_execute("TRUNCATE node_event, local_node")
        .await
        .ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk_a, kid_a) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk_a, &kid_a, "A", "127.0.0.1:7800")
        .await
        .unwrap();
    let id_a = identity::load_local(&a).await.unwrap();

    // Build B's offer (B's genesis node_id is the content-address of ITS genesis;
    // for the test we only need a stable hex + B's pubkey + matching fingerprint).
    let (sk_b, kid_b) = cairn_event::generate_key().unwrap();
    let b_node_id = hex::encode(cairn_event::event_address(b"B-genesis"));
    // The pairing nonce is DERIVED FROM RUNTIME KEY MATERIAL — house rule 6 (#146).
    //
    // An earlier version wrote `format!("nonce-{}", "B")` under a comment asserting it was
    // "built at runtime, never a hard-coded literal". It was not: every input is a literal,
    // so the whole expression is constant-foldable and CodeQL's dataflow still saw a
    // hard-coded value reaching a nonce. Alert #24
    // (`rust/hard-coded-cryptographic-value`, CRITICAL) stayed open on main for exactly
    // that reason — the mitigation was cosmetic, and a comment claiming a property the code
    // does not have is worse than no comment at all.
    //
    // `kid_b` is the hex of a keypair generated one line above, so this genuinely varies
    // per run and breaks the constant chain. It is B's PUBLIC key id, so no secret enters
    // the nonce. The value itself is arbitrary here — only that it is a stable, non-empty
    // freshness token matters, and nothing asserts on it.
    let nonce = format!("nonce-{}", &kid_b[..16]);
    let offer =
        pairing::make_offer_for(&b_node_id, &kid_b, "127.0.0.1:7801", &nonce, &sk_b).unwrap();
    let bundle = pairing::read_offer(&offer).unwrap();
    assert_eq!(
        bundle.fingerprint,
        cairn_event::short_fingerprint(&kid_b).unwrap()
    );

    identity::author_peer(
        &a,
        &sk_a,
        &kid_a,
        &id_a.node_id_hex,
        &bundle,
        Some("downstream"),
    )
    .await
    .unwrap();
    let peers = identity::list_peers(&a).await.unwrap();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].status, "active");
    assert_eq!(peers[0].peer_node_id_hex, b_node_id);

    identity::author_unpeer(&a, &sk_a, &kid_a, &id_a.node_id_hex, &b_node_id)
        .await
        .unwrap();
    let peers = identity::list_peers(&a).await.unwrap();
    assert_eq!(peers[0].status, "revoked");
}
