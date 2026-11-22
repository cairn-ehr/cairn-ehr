//! First launch: pin the node from the fingerprint the operator typed (#909 slice B, design D4
//! step 2).
//!
//! The operator reads the node's short fingerprint at the node (`gateway-setup` prints it) and
//! types it into the terminal's first launch. A short fingerprint is too short to pin on (80 bits
//! of a hash), so it is only used to RECOGNISE the node once: the terminal dials the gateway,
//! accepts the TLS server key only if its fingerprint is the typed one, and keeps the node's FULL
//! key as its pin from then on.
//!
//! The key is captured by the pinned verifier while the handshake is still running, but returned
//! only after the handshake completes, because only then has the node signed the handshake with
//! that key (TLS 1.3's CertificateVerify: proof that it holds the key, not merely a copy of the
//! certificate). The terminal presents its own key in the same handshake, so the gateway records
//! the attempt as a pending terminal the operator can then admit.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use cairn_event::{short_fingerprint, SigningKey};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::connector::SERVER_NAME;
use crate::transport::{terminal_client_config, TrustStore};

/// How long the first-launch dial (TCP, `SSLRequest`, TLS handshake) may take in all: the
/// gateway's own deadline before a startup, so a silent host fails the launch instead of hanging it.
const DIAL_WITHIN: Duration = Duration::from_secs(10);

/// How long the farewell read may wait for the gateway to close. The pin is already proven by
/// then; this only bounds a gateway that never hangs up.
const FAREWELL_WITHIN: Duration = Duration::from_secs(10);

/// A typed fingerprint in comparable form: upper-case hex with the dashes and spaces dropped, or
/// `None` unless that is exactly 20 hex digits. Pure.
pub(crate) fn normalise_fingerprint(typed: &str) -> Option<String> {
    let digits: String = typed
        .chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .collect();
    (digits.len() == 20 && digits.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| digits.to_ascii_uppercase())
}

/// Does the full key `pubkey_hex` have the (already normalised) short fingerprint `want`? Pure.
pub(crate) fn has_fingerprint(pubkey_hex: &str, want: &str) -> bool {
    short_fingerprint(pubkey_hex).is_ok_and(|fp| fp.replace('-', "") == want)
}

/// Postgres's `SSLRequest`: length 8, then the request code 80877103, both big-endian.
fn ssl_request() -> [u8; 8] {
    let mut message = [0u8; 8];
    message[..4].copy_from_slice(&8u32.to_be_bytes());
    message[4..].copy_from_slice(&80_877_103u32.to_be_bytes());
    message
}

/// Dial the gateway at `addr` (`host:port`), recognise the node by the `typed` fingerprint, and
/// return the node's full public key (64 hex digits) once the TLS handshake has completed.
///
/// The connection is then closed after one byte, so the gateway logs the attempt; the outcome of
/// that farewell is ignored (an unadmitted terminal is refused there, which is expected).
pub async fn pin_from_fingerprint(
    addr: &str,
    typed: &str,
    terminal_sk: &SigningKey,
) -> anyhow::Result<String> {
    pin_within(addr, typed, terminal_sk, DIAL_WITHIN).await
}

/// [`pin_from_fingerprint`] with the dial's deadline as a parameter, so a test can shorten it.
async fn pin_within(
    addr: &str,
    typed: &str,
    terminal_sk: &SigningKey,
    within: Duration,
) -> anyhow::Result<String> {
    let want = normalise_fingerprint(typed).ok_or_else(|| {
        anyhow!(
            "a node fingerprint is 20 hex digits, as the node prints it \
             (XXXX-XXXX-XXXX-XXXX-XXXX)"
        )
    })?;

    // The verifier admits a key only if its fingerprint is the typed one, and records it.
    let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let slot = captured.clone();
    let recognise: TrustStore = Arc::new(move |presented: &str| {
        let matches = has_fingerprint(presented, &want);
        if matches {
            *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(presented.to_string());
        }
        matches
    });
    let config = terminal_client_config(terminal_sk, recognise)?;

    let dial = async {
        let mut tcp = TcpStream::connect(addr)
            .await
            .with_context(|| format!("reaching the node's gateway at {addr}"))?;
        tcp.write_all(&ssl_request())
            .await
            .with_context(|| format!("asking the gateway at {addr} for TLS"))?;
        let mut answer = [0u8; 1];
        tcp.read_exact(&mut answer)
            .await
            .with_context(|| format!("asking the gateway at {addr} for TLS"))?;
        if answer[0] != b'S' {
            bail!("{addr} did not answer as a Cairn node's gateway (it declined TLS)");
        }
        let name = ServerName::try_from(SERVER_NAME)?;
        tokio_rustls::TlsConnector::from(config)
            .connect(name, tcp)
            .await
            .map_err(|e| {
                let captured = captured.lock().unwrap_or_else(|p| p.into_inner()).is_some();
                anyhow!(handshake_failure(addr, typed, captured, &e))
            })
    };
    let mut tls = tokio::time::timeout(within, dial).await.map_err(|_| {
        anyhow!(
            "the node's gateway at {addr} did not answer within {} s; nothing was pinned. Check \
             the address and port with the node's operator.",
            within.as_secs_f32()
        )
    })??;
    // The handshake completed, so the captured key is the one the node signed it with.
    let pin = captured
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
        .ok_or_else(|| anyhow!("the handshake completed without a recognised key"))?;

    // Farewell: one byte, then close our side and wait (bounded) for the gateway to close its.
    let _ = tls.write_all(&[0]).await;
    let _ = tls.shutdown().await;
    let _ = tokio::time::timeout(FAREWELL_WITHIN, tls.read_to_end(&mut Vec::new())).await;
    Ok(pin)
}

/// The sentence for a TLS handshake that failed during the first-launch dial. Pure.
///
/// `captured` says whether the verifier had already recognised the typed fingerprint. Only a
/// failed signature after that is a failure to PROVE the key (a server holding the certificate
/// but not the key); a connection lost or refused after the certificate is just an incomplete
/// handshake, and is not called impersonation.
fn handshake_failure(addr: &str, typed: &str, captured: bool, e: &std::io::Error) -> String {
    let bad_signature = matches!(
        super::connector::rustls_error_in(e),
        Some(rustls::Error::InvalidCertificate(
            rustls::CertificateError::BadSignature
        ))
    );
    match (captured, bad_signature) {
        (false, _) => format!(
            "the node at {addr} did not present a key with the fingerprint {typed} ({e}). Check \
             the fingerprint with the node's operator."
        ),
        (true, true) => format!(
            "the node at {addr} presented the fingerprint {typed} but did not prove it holds that \
             key ({e}); nothing was pinned. Tell the node's operator."
        ),
        (true, false) => format!(
            "the TLS handshake with the node at {addr} did not complete ({e}); nothing was \
             pinned. Try again, or check the address with the node's operator."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host that accepts TCP and never answers must not hang the first launch.
    #[tokio::test]
    async fn a_silent_server_is_given_up_on_at_the_deadline() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let _hold = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(60)).await;
            drop(tcp);
        });
        let (terminal, _) = cairn_event::generate_key().unwrap();
        let (_, node_kid) = cairn_event::generate_key().unwrap();
        let typed = short_fingerprint(&node_kid).unwrap();
        let dial = pin_within(&addr, &typed, &terminal, Duration::from_millis(200));
        let err = tokio::time::timeout(Duration::from_secs(5), dial)
            .await
            .expect("the dial gave up by its own deadline")
            .expect_err("nothing answered, so nothing is pinned");
        assert!(format!("{err:#}").contains("did not answer"), "{err:#}");
    }

    /// Only a failed signature is called a failure to prove the key; a handshake cut short after
    /// the certificate arrived is said to be just that.
    #[test]
    fn a_failed_handshake_says_what_failed() {
        use rustls::CertificateError;
        let io = |e: rustls::Error| std::io::Error::new(std::io::ErrorKind::InvalidData, e);
        let fp = "ABCD-ABCD-ABCD-ABCD-ABCD";
        let bad_sig = io(rustls::Error::InvalidCertificate(
            CertificateError::BadSignature,
        ));
        let text = handshake_failure("n:1", fp, true, &bad_sig);
        assert!(text.contains("did not prove"), "{text}");
        let cut = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "early eof");
        let text = handshake_failure("n:1", fp, true, &cut);
        assert!(text.contains("did not complete"), "{text}");
        assert!(text.contains("nothing was pinned"), "{text}");
        assert!(!text.contains("did not prove"), "{text}");
        let text = handshake_failure("n:1", fp, false, &cut);
        assert!(text.contains("did not present a key"), "{text}");
    }

    #[test]
    fn a_fingerprint_is_normalised_to_twenty_upper_case_digits() {
        let fp = "abcd-0123-EF45-6789-aaaa";
        assert_eq!(
            normalise_fingerprint(fp).as_deref(),
            Some("ABCD0123EF456789AAAA")
        );
        assert_eq!(
            normalise_fingerprint(" abcd0123ef456789aaaa ").as_deref(),
            Some("ABCD0123EF456789AAAA")
        );
        assert_eq!(normalise_fingerprint("abcd-0123-ef45-6789-aaa"), None);
        assert_eq!(normalise_fingerprint("abcd-0123-ef45-6789-aaaaa"), None);
        assert_eq!(normalise_fingerprint("abcd-0123-ef45-6789-aaag"), None);
    }

    #[test]
    fn the_ssl_request_is_postgres_s() {
        assert_eq!(ssl_request(), [0, 0, 0, 8, 0x04, 0xd2, 0x16, 0x2f]);
    }

    #[test]
    fn a_key_has_its_own_fingerprint_and_no_other() {
        let (_sk, kid) = cairn_event::generate_key().unwrap();
        let (_other, other_kid) = cairn_event::generate_key().unwrap();
        let want = normalise_fingerprint(&short_fingerprint(&kid).unwrap()).unwrap();
        assert!(has_fingerprint(&kid, &want));
        assert!(!has_fingerprint(&other_kid, &want));
    }
}
