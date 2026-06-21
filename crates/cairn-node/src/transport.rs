//! Built-in mTLS transport, pinned to the trust set (Task 9).
//!
//! Cairn nodes have no CA. A node's TLS identity *is* its Ed25519 signing key:
//! [`node_cert`] mints a self-signed cert whose SubjectPublicKeyInfo Ed25519 key
//! equals `sk.verifying_key()`, so the out-of-band fingerprint (which covers that
//! key) pins the TLS identity. The custom verifiers below replace CA-chain
//! validation entirely: a peer is admitted **iff** the hex of its presented
//! cert's Ed25519 SPKI key is `active` in the trust set. There is deliberately no
//! fallback to a WebPKI verifier — pinning is the whole decision (§9, §12).

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, DistinguishedName, Error, ServerConfig,
    SignatureScheme,
};

use cairn_event::SigningKey;

/// Looks up a peer's hex-encoded Ed25519 public key in the trust set, returning
/// `true` iff that key is `active`. Backed in production by
/// `SELECT 1 FROM trust_peer WHERE peer_pubkey=$1 AND status='active'`.
pub type TrustStore = Arc<dyn Fn(&str /*pubkey_hex*/) -> bool + Send + Sync>;

/// RFC 8410 PKCS#8 v1 prefix for an Ed25519 private key: the fixed ASN.1 framing
/// that precedes the 32-byte raw seed. Wrapping the seed this way lets rcgen
/// adopt the *node's existing* key rather than minting a fresh one.
const ED25519_PKCS8_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

/// Mint a self-signed Ed25519 cert whose SPKI public key IS `sk.verifying_key()`.
pub fn node_cert(
    sk: &SigningKey,
) -> anyhow::Result<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    // Wrap the node's 32-byte seed in PKCS#8 so rcgen reuses this exact keypair.
    let mut pkcs8 = ED25519_PKCS8_PREFIX.to_vec();
    pkcs8.extend_from_slice(&sk.to_bytes());
    let key_pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(
        &PrivatePkcs8KeyDer::from(pkcs8.clone()),
        &rcgen::PKCS_ED25519,
    )?;

    let params = rcgen::CertificateParams::new(vec!["cairn-node".to_string()])?;
    let cert = params.self_signed(&key_pair)?;

    let cert_der = cert.der().clone();
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8));
    Ok((cert_der, key_der))
}

/// Pull the 32 raw Ed25519 bytes out of a presented cert's SPKI and hex-encode
/// them — the same string the trust set keys on (== `verifying_key` hex).
fn peer_pubkey_hex(cert: &CertificateDer<'_>) -> Result<String, Error> {
    let (_, parsed) = x509_parser::parse_x509_certificate(cert.as_ref())
        .map_err(|_| Error::InvalidCertificate(CertificateError::BadEncoding))?;
    let spki = parsed.public_key().subject_public_key.as_ref();
    if spki.len() != 32 {
        return Err(Error::InvalidCertificate(CertificateError::BadEncoding));
    }
    Ok(hex::encode(spki))
}

/// The one error every reject path returns: an application-level verification
/// failure (not a chain/expiry error — there is no chain).
fn rejected() -> Error {
    Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure)
}

/// Pins a peer cert: success iff `trust(pubkey_hex)`. Shared by both verifiers.
fn pinned(cert: &CertificateDer<'_>, trust: &TrustStore) -> Result<(), Error> {
    let hex = peer_pubkey_hex(cert)?;
    if trust(&hex) {
        Ok(())
    } else {
        Err(rejected())
    }
}

/// Verifier installed on the **client**: pins the server's key. (`TrustStore` is
/// a closure, so `Debug` is written by hand rather than derived.)
struct PinnedServerVerifier {
    trust: TrustStore,
    provider: Arc<CryptoProvider>,
}

impl std::fmt::Debug for PinnedServerVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PinnedServerVerifier")
    }
}

impl ServerCertVerifier for PinnedServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        pinned(end_entity, &self.trust).map(|()| ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// Verifier installed on the **server**: pins the client's key (mTLS).
struct PinnedClientVerifier {
    trust: TrustStore,
    provider: Arc<CryptoProvider>,
}

impl std::fmt::Debug for PinnedClientVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PinnedClientVerifier")
    }
}

impl ClientCertVerifier for PinnedClientVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        pinned(end_entity, &self.trust).map(|()| ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// The process-wide aws-lc-rs crypto provider, installed once.
fn provider() -> Arc<CryptoProvider> {
    // Idempotent: ignore the "already installed" error so repeated calls (and
    // other modules) coexist.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    CryptoProvider::get_default()
        .expect("aws-lc-rs default provider installed")
        .clone()
}

/// Server side of mTLS: present the node cert, require + pin the client cert.
pub fn server_config(sk: &SigningKey, trust: TrustStore) -> anyhow::Result<Arc<ServerConfig>> {
    let provider = provider();
    let (cert, key) = node_cert(sk)?;
    let verifier = Arc::new(PinnedClientVerifier {
        trust,
        provider: provider.clone(),
    });
    let mut cfg = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![cert], key)?;
    // Pinning is a per-handshake decision against the LIVE trust set. A *resumed*
    // TLS session skips client-cert re-verification and reuses the original
    // session's identity — so a peer revoked between sessions could keep a foothold
    // by resuming an old ticket. Disable resumption: every handshake re-pins against
    // the current trust set, which is what makes `run`'s live revocation real
    // (PR #28 review, finding 1).
    cfg.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    cfg.send_tls13_tickets = 0;
    Ok(Arc::new(cfg))
}

/// Client side of mTLS: present the node cert, pin the server cert.
pub fn client_config(sk: &SigningKey, trust: TrustStore) -> anyhow::Result<Arc<ClientConfig>> {
    let provider = provider();
    let (cert, key) = node_cert(sk)?;
    let verifier = Arc::new(PinnedServerVerifier {
        trust,
        provider: provider.clone(),
    });
    let mut cfg = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(vec![cert], key)?;
    // Symmetric to the server: don't resume sessions, so the client re-pins the
    // server's key against the live trust set on every connection (a peer we
    // revoked is re-checked, never silently resumed). (PR #28 review, finding 1.)
    cfg.resumption = rustls::client::Resumption::disabled();
    Ok(Arc::new(cfg))
}

/// Test helper: run a `server`/`client` config through an in-memory duplex pipe
/// and report whether the pinned mTLS handshake completed. `Ok(())` iff both
/// peers passed pinning; `Err` if either verifier rejected.
///
/// Note: with TLS 1.3, client authentication is carried in the client's first
/// *application-data* flight, so the server only verifies (and can only reject)
/// the client cert once bytes flow. A bare `accept`/`connect` therefore can't
/// surface a client-pin rejection — so we drive a one-byte round-trip; the
/// reject manifests as a read/write error on the failing side.
pub async fn test_handshake(
    server: Arc<ServerConfig>,
    client: Arc<ClientConfig>,
) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::rustls::pki_types::ServerName;
    use tokio_rustls::{TlsAcceptor, TlsConnector};

    let (client_io, server_io) = tokio::io::duplex(16 * 1024);

    let acceptor = TlsAcceptor::from(server);
    let server_task = tokio::spawn(async move {
        let mut tls = acceptor.accept(server_io).await?;
        let mut buf = [0u8; 4];
        let _read = tls.read(&mut buf).await?; // forces client-cert verification
        tls.write_all(b"pong").await?;
        tls.flush().await?;
        Ok::<(), std::io::Error>(())
    });

    let connector = TlsConnector::from(client);
    let name = ServerName::try_from("cairn-node")?;
    let client_res = async {
        let mut tls = connector.connect(name, client_io).await?;
        tls.write_all(b"ping").await?;
        tls.flush().await?;
        let mut buf = [0u8; 4];
        let _read = tls.read(&mut buf).await?; // forces server-cert verification
        Ok::<(), std::io::Error>(())
    }
    .await;

    let server_res = server_task.await?;
    client_res?;
    server_res?;
    Ok(())
}
