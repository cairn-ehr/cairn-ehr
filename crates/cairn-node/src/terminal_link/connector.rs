//! The pinned TLS connector for tokio-postgres, and the [`Link`] the window opens through.
//!
//! tokio-postgres opens TLS by itself once given a connector: it sends Postgres's `SSLRequest`,
//! waits for `S`, then hands the socket to [`PinnedTls`], which runs the mutual TLS 1.3 handshake
//! with the gateway (`transport::terminal_client_config`: the terminal's key as the client
//! certificate, the node's full pinned key as the only server key accepted). SCRAM and every
//! query then run inside that session, end to end with the node's Postgres.
//!
//! The one thing added on top is the wording of the two refusals the link itself produces (see
//! `PinnedTls::refusal`): both must reach the window's reopen as REFUSALS, so it names the
//! operator instead of retrying as if the node were down.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use cairn_event::{short_fingerprint, SigningKey};
use rustls::pki_types::ServerName;
use rustls::{AlertDescription, ClientConfig};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_postgres::config::{SslMode, SslNegotiation};
use tokio_postgres::tls::{ChannelBinding, MakeTlsConnect, TlsConnect, TlsStream};
use tokio_postgres::{Client, Config, Socket};

use crate::db::{self, ConnectFailed};
use crate::transport::{terminal_client_config, TrustStore};

/// The TLS server name the terminal asks for. The pinned verifier ignores names (the key is the
/// identity), but rustls needs one; it is the name `transport::node_cert` puts in the cert.
pub(crate) const SERVER_NAME: &str = "cairn-node";

/// The terminal's TLS connector: its own key and the node's pinned key, in one rustls config.
#[derive(Clone)]
pub struct PinnedTls {
    config: Arc<ClientConfig>,
    /// This terminal's short fingerprint, for the not-admitted refusal's next command.
    terminal_fingerprint: String,
}

/// Why the link itself refused. Each is a refusal (an operator must act), never an outage.
#[derive(Debug, PartialEq, Eq)]
enum LinkRefusal {
    /// The gateway's TLS server rejected this terminal's key (it is not `active` there).
    NotAdmitted,
    /// The server did not prove it holds the pinned key.
    PinMismatch,
}

/// Classify a rustls error seen by the terminal. Pure.
///
/// * `AlertReceived(AccessDenied)`: the gateway's client verifier returns
///   `ApplicationVerificationFailure` for a key not in its trust set, and rustls sends that as
///   the `access_denied` alert. With TLS 1.3 the client's handshake has already finished when
///   the server judges its certificate, so the alert arrives at the client's FIRST READ (inside
///   tokio-postgres's startup), not in `connect`.
/// * `InvalidCertificate(_)`: only this terminal's own pinned verifier judges certificates, so
///   this is the server presenting another key (or a non-Ed25519 cert), or failing to sign with
///   the pinned one. Either way it is not the node this terminal pinned.
fn link_refusal(e: &rustls::Error) -> Option<LinkRefusal> {
    match e {
        rustls::Error::AlertReceived(AlertDescription::AccessDenied) => {
            Some(LinkRefusal::NotAdmitted)
        }
        rustls::Error::InvalidCertificate(_) => Some(LinkRefusal::PinMismatch),
        _ => None,
    }
}

/// The rustls error somewhere inside `e`, if there is one.
///
/// tokio-rustls hands rustls errors back as `io::Error`s wrapping them, and tokio-postgres wraps
/// those again (as a TLS error from the handshake, or an I/O error from a later read). An
/// `io::Error`'s `source()` SKIPS its wrapped error, so each level is unwrapped with `get_ref`.
fn rustls_error_in<'e>(e: &'e (dyn std::error::Error + 'static)) -> Option<&'e rustls::Error> {
    let mut level = Some(e);
    while let Some(err) = level {
        if let Some(r) = err.downcast_ref::<rustls::Error>() {
            return Some(r);
        }
        let wrapped = err
            .downcast_ref::<io::Error>()
            .and_then(io::Error::get_ref)
            .and_then(|inner| inner.downcast_ref::<rustls::Error>());
        if wrapped.is_some() {
            return wrapped;
        }
        level = err.source();
    }
    None
}

impl PinnedTls {
    /// A connector that presents `terminal_sk` and accepts only `node_pin_hex` (64 hex digits).
    pub fn new(terminal_sk: &SigningKey, node_pin_hex: &str) -> anyhow::Result<Self> {
        let pin = node_pin_hex.to_ascii_lowercase();
        let trust: TrustStore = Arc::new(move |presented: &str| presented == pin);
        let terminal_kid = hex::encode(terminal_sk.verifying_key().to_bytes());
        Ok(Self {
            config: terminal_client_config(terminal_sk, trust)?,
            terminal_fingerprint: short_fingerprint(&terminal_kid)?,
        })
    }

    /// The [`ConnectFailed`] for a failed pinned connect: one of the link's own refusals, or else
    /// exactly what the local link would have said.
    fn refusal(&self, e: &tokio_postgres::Error) -> ConnectFailed {
        match rustls_error_in(e).and_then(link_refusal) {
            Some(LinkRefusal::NotAdmitted) => ConnectFailed::refused_by_link(format!(
                "this terminal is not admitted by the node: at the node, run cairn-node \
                 admit-terminal {} --label \"<where this terminal is>\", then start again.",
                self.terminal_fingerprint
            )),
            Some(LinkRefusal::PinMismatch) => ConnectFailed::refused_by_link(
                "the node presented a different key than this terminal pinned (or could not prove \
                 it holds the pinned one), so nothing was sent to it. If the node's key really \
                 changed, ask the node's operator before deleting this terminal's node.pin to pin \
                 it again."
                    .to_string(),
            ),
            None => ConnectFailed::from(e),
        }
    }
}

impl MakeTlsConnect<Socket> for PinnedTls {
    type Stream = PinnedStream;
    type TlsConnect = PinnedTls;
    type Error = io::Error;

    /// The host name tokio-postgres passes is ignored: the pin, not a name, is the identity.
    fn make_tls_connect(&mut self, _domain: &str) -> io::Result<PinnedTls> {
        Ok(self.clone())
    }
}

impl TlsConnect<Socket> for PinnedTls {
    type Stream = PinnedStream;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = io::Result<PinnedStream>> + Send>>;

    fn connect(self, stream: Socket) -> Self::Future {
        Box::pin(async move {
            let name = ServerName::try_from(SERVER_NAME).map_err(io::Error::other)?;
            let tls = tokio_rustls::TlsConnector::from(self.config)
                .connect(name, stream)
                .await?;
            Ok(PinnedStream(tls))
        })
    }
}

/// The TLS session tokio-postgres reads and writes. A newtype only because tokio-postgres's
/// `TlsStream` trait and tokio-rustls's stream are both foreign to this crate.
pub struct PinnedStream(tokio_rustls::client::TlsStream<Socket>);

impl TlsStream for PinnedStream {
    /// None: the TLS session ends at the gateway, and Postgres sees a loopback connection, so
    /// there is no TLS session in common to bind SCRAM to.
    fn channel_binding(&self) -> ChannelBinding {
        ChannelBinding::none()
    }
}

impl AsyncRead for PinnedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
    }
}

impl AsyncWrite for PinnedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
    }
}

/// How the window reaches the node's database: directly on this machine, or through the node's
/// gateway with the pinned TLS link. [`super::prepare`] decides which.
pub enum Link {
    Local,
    Pinned(PinnedTls),
}

impl Link {
    /// [`db::connect_with_task`] over this link. Every error is a [`ConnectFailed`], as the local
    /// door's is, so the window's reopen classifies both alike.
    pub async fn connect_with_task(
        &self,
        conn: &str,
    ) -> anyhow::Result<(Client, tokio::task::AbortHandle)> {
        match self {
            Link::Local => db::connect_with_task(conn).await,
            Link::Pinned(tls) => {
                let legible =
                    |e: tokio_postgres::Error| anyhow::Error::new(ConnectFailed::from(&e));
                let mut cfg: Config = conn.parse().map_err(legible)?;
                // Forced whatever the string says: `sslmode=disable` would send the startup in
                // clear text, and `sslnegotiation=direct` would skip the `SSLRequest`. The gateway
                // closes either without a reply, which the window would retry as an outage.
                cfg.ssl_mode(SslMode::Require)
                    .ssl_negotiation(SslNegotiation::Postgres);
                db::open(cfg, tls.clone())
                    .await
                    .map_err(|e| anyhow::Error::new(tls.refusal(&e)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_access_denied_and_a_rejected_certificate_are_the_links_refusals() {
        use rustls::CertificateError;
        assert_eq!(
            link_refusal(&rustls::Error::AlertReceived(
                AlertDescription::AccessDenied
            )),
            Some(LinkRefusal::NotAdmitted)
        );
        assert_eq!(
            link_refusal(&rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure
            )),
            Some(LinkRefusal::PinMismatch)
        );
        // A handshake that fails for any other reason (a version or cipher mismatch, a hang-up)
        // is not this terminal's or this node's identity being refused.
        for other in [
            rustls::Error::AlertReceived(AlertDescription::HandshakeFailure),
            rustls::Error::AlertReceived(AlertDescription::ProtocolVersion),
            rustls::Error::HandshakeNotComplete,
        ] {
            assert_eq!(link_refusal(&other), None, "{other:?}");
        }
    }

    #[test]
    fn the_rustls_error_is_found_through_io_error_wrapping() {
        let inner = rustls::Error::AlertReceived(AlertDescription::AccessDenied);
        let io = io::Error::new(io::ErrorKind::InvalidData, inner.clone());
        assert_eq!(rustls_error_in(&io), Some(&inner));
        let plain = io::Error::other("no tls here");
        assert_eq!(rustls_error_in(&plain), None);
    }
}
