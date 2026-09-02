//! A network peer over plain TCP. Today's `cairn-sync` behaviour, moved verbatim.
//!
//! NoTls is intentional on this plane: the link is WireGuard, which is the transport and the
//! perimeter (Spike 0001's assumption). The node plane is the one with mTLS pinned to the
//! trust set (`cairn-node/src/transport.rs`); this is the walking-skeleton clinical plane.

use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::framing::{read_frame, write_frame};
use crate::transport::{Transport, TransportError};
use crate::wire::Request;

/// A peer reachable at `host:port`.
pub struct TcpTransport {
    peer: String,
    label: String,
}

impl TcpTransport {
    pub fn new(peer: impl Into<String>) -> Self {
        let peer = peer.into();
        let label = format!("tcp {peer}");
        Self { peer, label }
    }

    /// ONE attempt, no backoff.
    ///
    /// The byte tier wants this rather than [`Transport::request`]: a blob swarm round-robins
    /// across many peers, so failing over to the next source immediately beats spending four
    /// backoff attempts on a source that is down.
    pub fn try_once(&self, req: &Request) -> Result<Vec<u8>, TransportError> {
        self.exchange(req)
            .map_err(|source| TransportError::Exchange {
                label: self.label.clone(),
                source,
            })
    }

    /// The raw exchange, with its cause UNBOXED into the error type by the callers above.
    /// Kept separate so both the single-attempt and the retrying entry points build the same
    /// `Exchange` error, with the same reachable `source`.
    fn exchange(&self, req: &Request) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        // Bounded connect so a dead link fails fast instead of hanging for minutes.
        let addr = self.peer.to_socket_addrs()?.next().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                "could not resolve peer address",
            )
        })?;
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10))?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;
        write_frame(&mut stream, &serde_json::to_vec(req)?)?;
        Ok(read_frame(&mut stream)?)
    }
}

impl Transport for TcpTransport {
    fn label(&self) -> &str {
        &self.label
    }

    /// Retry with exponential backoff. A Starlink link drops constantly; a transient failure
    /// must not fail the whole pull — it retries, and only a sustained outage surfaces as an
    /// error (which the `run` loop logs as a partition).
    fn request(&self, req: &Request) -> Result<Vec<u8>, TransportError> {
        let mut delay = Duration::from_millis(250);
        let mut last = None;
        for attempt in 0..4 {
            match self.exchange(req) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last = Some(e);
                    if attempt < 3 {
                        std::thread::sleep(delay);
                        delay *= 2;
                    }
                }
            }
        }
        Err(TransportError::Exchange {
            label: self.label.clone(),
            // Unwrap is unreachable: the loop runs four times and every arm that does not
            // return sets `last`.
            source: last.expect("four attempts always record a failure"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Request;

    #[test]
    fn the_label_names_the_transport_and_the_address() {
        let t = TcpTransport::new("10.0.0.3:9443");
        assert_eq!(t.label(), "tcp 10.0.0.3:9443");
    }

    /// An unresolvable address must surface as `Exchange` — the link class — not as a panic
    /// and not as `Unsupported`. Uses a syntactically valid but unroutable address so the
    /// test needs no network and no listener.
    #[test]
    fn an_unreachable_peer_is_an_exchange_failure() {
        let t = TcpTransport::new("127.0.0.1:1");
        let err = t
            .try_once(&Request::EventsAfterSeq {
                after_seq: 0,
                unwrap_cert: None,
            })
            .expect_err("nothing listens on port 1");
        assert!(matches!(err, TransportError::Exchange { .. }), "{err}");
    }
}
