//! The one seam: everything that can answer a [`Request`].
//!
//! WHY A TRAIT (slice 2b, #500). ADR-0026 decision 2 says clinical events back up "as a cold
//! peer … a configuration of the existing sync daemon". Until this trait existed, `cairn-sync`
//! reached its peer through one free function that opened a TCP socket, and a backup medium is
//! not a socket — so "the medium is a peer" was prose with nothing behind it. With the seam,
//! `do_pull` is transport-agnostic: slice 2d's restore drives the SAME puller, with the SAME
//! cursor, quarantine pen and custody handling, against a file.

use std::error::Error;
use std::fmt;

use crate::wire::Request;

/// Anything that can answer a [`Request`] with one response frame.
///
/// Implementors own their own retries, timeouts and reconnection: a caller sees either a
/// response frame or a [`TransportError`], never a half-finished exchange.
pub trait Transport {
    /// Where this transport actually goes — `"tcp 10.0.0.3:9443"`, `"medium /vol/cairn.b3"`.
    ///
    /// ERROR PROSE ONLY. It is deliberately NOT the peer's NAME: `sync_state` is keyed on
    /// `peer_name`, which `do_pull` keeps as its own parameter, because the cursor must stay
    /// attached to the peer's identity even when the route to it changes.
    fn label(&self) -> &str;

    /// One request, one response frame.
    fn request(&self, req: &Request) -> Result<Vec<u8>, TransportError>;
}

/// Why a request produced no usable response. **Two variants, because they have opposite
/// remedies** — the same reasoning that split `cairn_medium::BackupError` three ways.
#[derive(Debug)]
pub enum TransportError {
    /// The exchange failed: resolve, connect, write, or read. Retrying may help.
    ///
    /// ⚠️ `source` MUST stay a real, reachable cause and must never be flattened into the
    /// label or a message. `cairn-sync`'s `chain_reaches_a_peer_frame_error` walks `source()`
    /// for an `io::Error` of kind `InvalidData` — [`crate::read_frame`]'s refusal of an
    /// over-cap length prefix — to tell a PEER sending garbage from a LINK that went away.
    /// Those are different operator words (#482), and a `String` error has no chain.
    Exchange {
        label: String,
        source: Box<dyn Error + Send + Sync + 'static>,
    },
    /// This transport cannot answer this request at all — a medium asked for a blob slice, or
    /// a pre-CAIRNB3 image asked for clinical events. NOT a link failure: no retry helps, and
    /// a caller that cannot tell the two apart will retry four times for nothing.
    Unsupported { label: String, reason: String },
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // The cause is the SUFFIX, and that placement is load-bearing rather than
            // stylistic: `cairn-sync`'s `operator_chain` drops a layer only when the layer
            // above it ENDS WITH that layer's rendering, so a mid-sentence `{source}` would
            // print the same transport error twice on the `run` path.
            TransportError::Exchange { label, source } => {
                write!(f, "{label}: the exchange failed: {source}")
            }
            TransportError::Unsupported { label, reason } => {
                write!(f, "{label}: cannot answer this request: {reason}")
            }
        }
    }
}

impl Error for TransportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            TransportError::Exchange { source, .. } => Some(source.as_ref()),
            TransportError::Unsupported { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    use std::io;

    /// `cairn-sync`'s `chain_reaches_a_peer_frame_error` decides "this peer sent an over-cap
    /// frame" — an INTEGRITY condition, not a partition — by walking `source()` for an
    /// `io::Error` of kind `InvalidData`. A `TransportError` that formatted its cause into a
    /// string instead of keeping it as `source` would silently reclassify that peer as link
    /// downtime, and every test on the far side would stay green because those tests build the
    /// error they classify. So the chain is pinned HERE, on the error this crate produces.
    #[test]
    fn exchange_keeps_the_io_error_reachable_as_a_source() {
        let cause = io::Error::new(io::ErrorKind::InvalidData, "frame length 99 exceeds cap");
        let err = TransportError::Exchange {
            label: "tcp 10.0.0.3:9443".into(),
            source: Box::new(cause),
        };
        let found = std::iter::successors(Some(&err as &(dyn Error + 'static)), |e| (*e).source())
            .filter_map(|e| e.downcast_ref::<io::Error>())
            .any(|io| io.kind() == io::ErrorKind::InvalidData);
        assert!(found, "the io::Error must stay reachable through source()");
    }

    /// An unsupported request is not a failure of the link and no retry helps. It must be a
    /// DIFFERENT variant, not a differently-worded Exchange: a caller that cannot tell them
    /// apart will retry a medium four times for a blob it does not have.
    #[test]
    fn unsupported_is_a_distinct_variant_with_no_source() {
        let err = TransportError::Unsupported {
            label: "medium /vol/cairn.b3".into(),
            reason: "this medium carries no byte tier".into(),
        };
        assert!(err.source().is_none());
        assert!(err.to_string().contains("carries no byte tier"), "{err}");
    }
}
