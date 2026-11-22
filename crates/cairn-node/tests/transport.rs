use cairn_node::transport;
use std::sync::Arc;

#[tokio::test]
async fn mtls_accepts_pinned_peer_and_rejects_unpinned() {
    let (sk_a, kid_a) = cairn_event::generate_key().unwrap();
    let (sk_b, kid_b) = cairn_event::generate_key().unwrap();
    let (_sk_c, kid_c) = cairn_event::generate_key().unwrap();

    // A trusts B only.
    let kid_b2 = kid_b.clone();
    let trust_a: transport::TrustStore = Arc::new(move |pk: &str| pk == kid_b2);
    // B trusts A only.
    let kid_a2 = kid_a.clone();
    let trust_b: transport::TrustStore = Arc::new(move |pk: &str| pk == kid_a2);

    // A serves; B connects -> handshake succeeds (mutually pinned).
    let server = transport::server_config(&sk_a, trust_a.clone()).unwrap();
    let client_b = transport::client_config(&sk_b, trust_b).unwrap();
    assert!(
        transport::test_handshake(server.clone(), client_b)
            .await
            .is_ok(),
        "mutually-pinned peers must handshake"
    );

    // C (untrusted by A) connects -> A's ClientCertVerifier rejects.
    let trust_c_sees_a: transport::TrustStore = Arc::new(move |pk: &str| pk == kid_a);
    let client_c = transport::client_config(&_sk_c, trust_c_sees_a).unwrap();
    let _ = kid_c;
    let _ = kid_b;
    assert!(
        transport::test_handshake(server, client_c).await.is_err(),
        "an unpinned client must be rejected at the TLS layer"
    );
}

/// A trust store admitting exactly the key `pk`.
fn pin(pk: &str) -> transport::TrustStore {
    let pk = pk.to_string();
    Arc::new(move |p: &str| p == pk)
}

#[tokio::test]
async fn gateway_and_terminal_handshake_when_both_pinned() {
    let (gw, gw_id) = cairn_event::generate_key().unwrap();
    let (term, term_id) = cairn_event::generate_key().unwrap();
    let server = transport::gateway_server_config(&gw, pin(&term_id)).unwrap();
    let client = transport::terminal_client_config(&term, pin(&gw_id)).unwrap();
    transport::test_handshake(server, client).await.unwrap();
}

#[tokio::test]
async fn gateway_refuses_an_unpinned_terminal() {
    let (gw, gw_id) = cairn_event::generate_key().unwrap();
    let (term, _) = cairn_event::generate_key().unwrap();
    let (_, other_id) = cairn_event::generate_key().unwrap();
    let server = transport::gateway_server_config(&gw, pin(&other_id)).unwrap();
    let client = transport::terminal_client_config(&term, pin(&gw_id)).unwrap();
    assert!(transport::test_handshake(server, client).await.is_err());
}

#[tokio::test]
async fn terminal_refuses_an_unpinned_gateway() {
    let (gw, _) = cairn_event::generate_key().unwrap();
    let (term, term_id) = cairn_event::generate_key().unwrap();
    let (_, other_id) = cairn_event::generate_key().unwrap();
    let server = transport::gateway_server_config(&gw, pin(&term_id)).unwrap();
    let client = transport::terminal_client_config(&term, pin(&other_id)).unwrap();
    assert!(transport::test_handshake(server, client).await.is_err());
}

#[tokio::test]
async fn gateway_refuses_a_tls12_only_client() {
    let (gw, gw_id) = cairn_event::generate_key().unwrap();
    let (term, term_id) = cairn_event::generate_key().unwrap();
    let client = transport::tls12_only_client_config_for_tests(&term, pin(&gw_id)).unwrap();
    // Positive control: the same TLS 1.2 client DOES handshake against default
    // versions, so the failure below can only be the gateway's version pin.
    let lax = transport::server_config(&gw, pin(&term_id)).unwrap();
    transport::test_handshake(lax, client.clone())
        .await
        .unwrap();
    let server = transport::gateway_server_config(&gw, pin(&term_id)).unwrap();
    assert!(transport::test_handshake(server, client).await.is_err());
}

/// The test-only TLS 1.2 builder must never gain a production caller.
#[test]
fn tls12_test_builder_has_no_production_caller() {
    fn walk(dir: &std::path::Path, hits: &mut usize) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, hits);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                *hits += text.matches("tls12_only_client_config_for_tests").count();
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut hits = 0;
    for group in ["crates", "cairn-gui"] {
        for krate in std::fs::read_dir(root.join(group))
            .into_iter()
            .flatten()
            .flatten()
        {
            walk(&krate.path().join("src"), &mut hits);
        }
    }
    // Exactly one occurrence: the definition itself.
    assert_eq!(
        hits, 1,
        "tls12_only_client_config_for_tests used outside tests"
    );
}

/// A resumed TLS 1.3 session skips the client-certificate verifier, so the live trust set would
/// never be consulted: a revoked terminal holding an old ticket would get back in. The gateway
/// issues no tickets and caches no sessions, so every handshake pins afresh (review T2).
#[test]
fn the_gateway_never_resumes_a_tls_session() {
    let (gw, _) = cairn_event::generate_key().unwrap();
    let (_, term_id) = cairn_event::generate_key().unwrap();
    let cfg = transport::gateway_server_config(&gw, pin(&term_id)).unwrap();
    assert_eq!(cfg.send_tls13_tickets, 0, "no TLS 1.3 tickets are sent");
    assert!(
        !cfg.session_storage.can_cache(),
        "no session is cached for a stateful resume"
    );
}
