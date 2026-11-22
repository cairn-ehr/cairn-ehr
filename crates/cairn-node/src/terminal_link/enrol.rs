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
    let mut tls = match tokio_rustls::TlsConnector::from(config)
        .connect(name, tcp)
        .await
    {
        Ok(tls) => tls,
        Err(e) if captured.lock().unwrap_or_else(|p| p.into_inner()).is_none() => {
            bail!(
                "the node at {addr} did not present a key with the fingerprint {typed} ({e}). \
                 Check the fingerprint with the node's operator."
            )
        }
        Err(e) => bail!(
            "the node at {addr} presented the fingerprint {typed} but did not prove it holds \
             that key ({e}); nothing was pinned. Tell the node's operator."
        ),
    };
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

#[cfg(test)]
mod tests {
    use super::*;

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
