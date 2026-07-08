//! Finding 1 (PR #28 review): the trust set the mTLS verifier consults must be
//! LIVE. Revoking a peer — removing its pubkey from the set — has to take effect
//! on an *already-built* `ServerConfig`/`ClientConfig`, with no rebind or process
//! restart. This is the exact property `sync::run` now relies on to apply
//! `peer.revoked` (and `peer.added`) to BOTH the inbound serve path and the
//! outbound pull on the next cycle. No DB needed: this tests the seam directly.

use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use cairn_event::generate_key;
use cairn_node::sync::{trust_store_from_set, TrustSet};
use cairn_node::transport::{client_config, server_config, test_handshake};

#[tokio::test]
async fn revoking_a_peer_in_the_live_set_rejects_the_next_handshake() {
    let (sk_a, kid_a) = generate_key().unwrap(); // server node A
    let (sk_b, kid_b) = generate_key().unwrap(); // client node B

    // A trusts B; B trusts A. One live set per node, shared into the verifiers.
    let set_a: TrustSet = Arc::new(RwLock::new(HashSet::from([kid_b.clone()])));
    let set_b: TrustSet = Arc::new(RwLock::new(HashSet::from([kid_a.clone()])));

    // Build BOTH configs ONCE. The verifier closures read the live sets on every
    // handshake — these configs are never rebuilt below.
    let server = server_config(&sk_a, trust_store_from_set(set_a.clone())).unwrap();
    let client = client_config(&sk_b, trust_store_from_set(set_b.clone())).unwrap();

    // While B is an active peer, the pinned-mTLS handshake completes.
    test_handshake(server.clone(), client.clone())
        .await
        .expect("handshake must succeed while B is an active peer");

    // Revoke B by removing its key from A's LIVE set — `server` is NOT rebuilt.
    set_a.write().unwrap().remove(&kid_b);

    // The SAME server config must now reject B's client cert.
    assert!(
        test_handshake(server.clone(), client.clone())
            .await
            .is_err(),
        "a revoked peer must be rejected by the already-built serve config (no restart)"
    );

    // Re-adding B to the live set restores acceptance on the same config — proving
    // peer.added also applies without a rebuild (the symmetric half of finding 1).
    set_a.write().unwrap().insert(kid_b.clone());
    test_handshake(server, client)
        .await
        .expect("re-added peer must be accepted again with no rebuild");
}
