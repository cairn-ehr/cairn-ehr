//! The terminal's side of the pinned link (#909 slice B, Task 6), against in-test gateways.
//!
//! No database: each test stands up a TCP listener that answers the Postgres `SSLRequest` and then
//! runs the node's real TLS 1.3 server config (`transport::gateway_server_config`), so the
//! handshake, the alerts and the bytes on the wire are the ones a real gateway produces. Keys are
//! generated at runtime (house rule 6).

use std::sync::{Arc, Mutex};

use cairn_event::{generate_key, short_fingerprint};
use cairn_node::db::ConnectFailed;
use cairn_node::terminal_link::enrol::pin_from_fingerprint;
use cairn_node::terminal_link::{Link, PinnedTls};
use cairn_node::transport::{gateway_server_config, node_cert, TrustStore};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// The 8 bytes of a Postgres `SSLRequest`: length 8, code 80877103.
const SSL_REQUEST: [u8; 8] = [0, 0, 0, 8, 0x04, 0xd2, 0x16, 0x2f];

/// A trust store that admits `admitted` (if any) and records every key it is asked about, as the
/// gateway records an unknown terminal's attempt as a pending row.
fn recording_trust(admitted: Option<String>) -> (TrustStore, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let trust: TrustStore = Arc::new(move |hex: &str| {
        log.lock().unwrap().push(hex.to_string());
        admitted.as_deref() == Some(hex)
    });
    (trust, seen)
}

/// A one-connection gateway: answer the `SSLRequest` with `S`, run TLS with `cfg`, read the
/// terminal's first message, then hang up. Returns its address and its task.
async fn gateway(cfg: Arc<rustls::ServerConfig>) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let task = tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        let mut first8 = [0u8; 8];
        tcp.read_exact(&mut first8).await.unwrap();
        assert_eq!(first8, SSL_REQUEST);
        tcp.write_all(b"S").await.unwrap();
        let accepted = tokio_rustls::TlsAcceptor::from(cfg)
            .accept(tcp)
            .into_fallible()
            .await;
        let mut tls = match accepted {
            Ok(tls) => tls,
            // A refused terminal: rustls has sent its alert. Close gently (our side first, then
            // drain), because closing with the terminal's startup still unread sends a TCP
            // reset, and a reset can discard the alert before the terminal reads it.
            Err((_, mut tcp)) => {
                let _ = tcp.shutdown().await;
                let mut rest = Vec::new();
                let drain = tcp.read_to_end(&mut rest);
                let _ = tokio::time::timeout(std::time::Duration::from_secs(5), drain).await;
                return;
            }
        };
        // Read what the terminal sends first (a startup, or the enrolment's farewell byte),
        // then hang up, as a gateway does when its upstream is gone.
        let mut buf = [0u8; 512];
        let _ = tls.read(&mut buf).await;
    });
    (addr, task)
}

fn conn_to(addr: &str) -> String {
    let (host, port) = addr.rsplit_once(':').unwrap();
    format!("host={host} port={port} user=u dbname=d connect_timeout=5")
}

/// The error a pinned connect ended with, as the [`ConnectFailed`] the window's reopen reads.
async fn pinned_connect_error(tls: PinnedTls, conn: &str) -> (String, bool, Option<String>) {
    let err = match Link::Pinned(tls).connect_with_task(conn).await {
        Ok(_) => panic!("the connect should fail"),
        Err(e) => e,
    };
    let cf = err
        .downcast_ref::<ConnectFailed>()
        .unwrap_or_else(|| panic!("every error is a ConnectFailed: {err:#}"));
    (
        cf.to_string(),
        cf.refused_by_client(),
        cf.sqlstate().map(str::to_string),
    )
}

#[tokio::test]
async fn pin_from_fingerprint_captures_the_key_only_after_the_handshake() {
    let (node, node_kid) = generate_key().unwrap();
    let (terminal, terminal_kid) = generate_key().unwrap();
    let typed = short_fingerprint(&node_kid).unwrap();

    // A real gateway that does not (yet) admit the terminal: the pin is still captured, and the
    // gateway saw the terminal's key, so it can record the attempt.
    let (trust, seen) = recording_trust(None);
    let (addr, task) = gateway(gateway_server_config(&node, trust).unwrap()).await;
    let pin = pin_from_fingerprint(&addr, &typed, &terminal)
        .await
        .unwrap();
    assert_eq!(pin, node_kid);
    task.await.unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![terminal_kid]);

    // A server that presents the node's certificate but cannot sign with the node's key (it
    // holds another): the fingerprint matches, but the handshake fails, so nothing is pinned.
    let (impostor, _) = generate_key().unwrap();
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let (cert, _) = node_cert(&node).unwrap();
    let (_, wrong_key) = node_cert(&impostor).unwrap();
    let signer = provider.key_provider.load_private_key(wrong_key).unwrap();
    let certified = rustls::sign::CertifiedKey::new(vec![cert], signer);
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(rustls::sign::SingleCertAndKey::from(certified)));
    let (addr, _task) = gateway(Arc::new(cfg)).await;
    let err = pin_from_fingerprint(&addr, &typed, &terminal)
        .await
        .expect_err("a server that cannot prove the key is never pinned");
    assert!(!format!("{err:#}").contains(&node_kid), "{err:#}");
    assert!(format!("{err:#}").contains("did not prove"), "{err:#}");
}

#[tokio::test]
async fn pin_from_fingerprint_refuses_a_wrong_fingerprint() {
    let (node, _) = generate_key().unwrap();
    let (terminal, _) = generate_key().unwrap();
    let (_, other_kid) = generate_key().unwrap();
    let (trust, _seen) = recording_trust(None);
    let (addr, _task) = gateway(gateway_server_config(&node, trust).unwrap()).await;
    // Typed lower-case without dashes: normalised before it is compared, and still wrong.
    let typed = short_fingerprint(&other_kid)
        .unwrap()
        .replace('-', "")
        .to_ascii_lowercase();
    let err = pin_from_fingerprint(&addr, &typed, &terminal)
        .await
        .expect_err("a different node is never pinned");
    assert!(format!("{err:#}").contains("fingerprint"), "{err:#}");
}

#[tokio::test]
async fn a_pinned_link_maps_access_denied_to_a_refusal() {
    let (node, node_kid) = generate_key().unwrap();
    let (terminal, terminal_kid) = generate_key().unwrap();
    let (trust, _seen) = recording_trust(None);
    let (addr, _task) = gateway(gateway_server_config(&node, trust).unwrap()).await;
    let tls = PinnedTls::new(&terminal, &node_kid).unwrap();
    let (text, refused, sqlstate) = pinned_connect_error(tls, &conn_to(&addr)).await;
    assert!(text.contains("not admitted"), "{text}");
    let fp = short_fingerprint(&terminal_kid).unwrap();
    assert!(
        text.contains(&fp),
        "names this terminal's fingerprint: {text}"
    );
    // A revoked terminal gets the same alert, so the text covers both remedies.
    assert!(text.contains("operator"), "{text}");
    assert!(text.contains("cairn-node admit-terminal"), "{text}");
    assert!(text.contains("revoked"), "{text}");
    assert!(text.contains("new terminal key"), "{text}");
    assert!(
        refused,
        "a refusal, so the reopen names the operator: {text}"
    );
    assert_eq!(sqlstate, None);
}

#[tokio::test]
async fn a_pin_mismatch_is_a_refusal() {
    let (node, _) = generate_key().unwrap();
    let (terminal, terminal_kid) = generate_key().unwrap();
    let (_, other_kid) = generate_key().unwrap();
    let (trust, _seen) = recording_trust(Some(terminal_kid));
    let (addr, _task) = gateway(gateway_server_config(&node, trust).unwrap()).await;
    let tls = PinnedTls::new(&terminal, &other_kid).unwrap();
    let (text, refused, sqlstate) = pinned_connect_error(tls, &conn_to(&addr)).await;
    assert!(text.contains("different key"), "{text}");
    assert!(refused, "{text}");
    assert_eq!(sqlstate, None);
}

/// A server that answers `S` but presents a non-Ed25519 certificate (Postgres's own TLS port,
/// for one) is named as the wrong service, not as the node having changed its key.
#[tokio::test]
async fn a_server_that_is_not_a_gateway_is_named_as_such() {
    let key_pair = rcgen::KeyPair::generate().unwrap(); // ECDSA P-256 by default
    let cert = rcgen::CertificateParams::new(vec!["cairn-node".to_string()])
        .unwrap()
        .self_signed(&key_pair)
        .unwrap();
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(key_pair.serialize_der().into());
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], key)
        .unwrap();
    let (addr, _task) = gateway(Arc::new(cfg)).await;
    let (terminal, _) = generate_key().unwrap();
    let (_, node_kid) = generate_key().unwrap();
    let tls = PinnedTls::new(&terminal, &node_kid).unwrap();
    let (text, refused, sqlstate) = pinned_connect_error(tls, &conn_to(&addr)).await;
    assert!(
        text.contains("not a Cairn node's terminal gateway"),
        "{text}"
    );
    assert!(text.contains("port"), "{text}");
    assert!(refused, "{text}");
    assert_eq!(sqlstate, None);
}

/// The control: an admitted terminal whose gateway then hangs up is an OUTAGE, not a refusal,
/// so the mapping above is not a blanket "every TLS-path error is a refusal".
#[tokio::test]
async fn an_admitted_terminal_whose_gateway_hangs_up_is_an_outage() {
    let (node, node_kid) = generate_key().unwrap();
    let (terminal, terminal_kid) = generate_key().unwrap();
    let (trust, _seen) = recording_trust(Some(terminal_kid));
    let (addr, _task) = gateway(gateway_server_config(&node, trust).unwrap()).await;
    let tls = PinnedTls::new(&terminal, &node_kid).unwrap();
    let (text, refused, _) = pinned_connect_error(tls, &conn_to(&addr)).await;
    assert!(!refused, "{text}");
}

#[tokio::test]
async fn a_pinned_link_overrides_sslmode_and_sslnegotiation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    // A LAN impostor that DECLINES TLS. Under `sslmode=prefer` the client would carry on and send
    // its startup in clear; only `require` stops dead (review T1: an SSLRequest alone cannot tell
    // the two apart).
    let seen = tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        let mut first8 = [0u8; 8];
        tcp.read_exact(&mut first8).await.unwrap();
        tcp.write_all(b"N").await.unwrap();
        let mut rest = Vec::new();
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            tcp.read_to_end(&mut rest),
        )
        .await;
        (first8, rest)
    });
    let (terminal, _) = generate_key().unwrap();
    let (_, node_kid) = generate_key().unwrap();
    let tls = PinnedTls::new(&terminal, &node_kid).unwrap();
    let conn = format!("{} sslmode=disable sslnegotiation=direct", conn_to(&addr));
    let got = Link::Pinned(tls).connect_with_task(&conn).await;
    assert!(got.is_err(), "a server that declines TLS is never used");
    let (first8, rest) = seen.await.unwrap();
    assert_eq!(first8, SSL_REQUEST, "an SSLRequest went first");
    assert!(
        rest.is_empty(),
        "nothing followed the declined SSLRequest (a clear-text startup would): {rest:?}"
    );
}
