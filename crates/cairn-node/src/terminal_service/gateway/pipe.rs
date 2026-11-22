//! The gateway's byte plumbing once a decision is made (#909 slice B, D2 step 4): the pipe of an
//! admitted session, the gentle close of a refused one, and the out-of-band `CancelRequest`.
//!
//! Split from `serve.rs` (which decides) so that each file stays readable. Nothing here judges a
//! terminal or a login; it moves bytes and, for the first few server messages, LOOKS at them.
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::sessions::SessionGuard;
use super::upstream::Upstream;
use super::wire::{backend_key, cancel_request, frame};

/// How long a refused connection is drained after the gateway stops writing.
pub const DRAIN: Duration = Duration::from_secs(1);
/// More than this buffered without one whole server message, and the session is closed
/// (controller ruling R3: a cap on what the observer holds, not a parser change).
pub const OBSERVE_CAP: usize = 64 * 1024;

/// What one look at the buffered server bytes found. **Pure** result of [`scan`].
#[derive(Debug, PartialEq, Eq)]
pub struct Scan {
    /// A `'Z'` (ReadyForQuery) or `'E'` (ErrorResponse) was seen: the startup phase is over.
    pub done: bool,
    /// The `CancelRequest` for the session's backend, if a `'K'` (BackendKeyData) was seen.
    pub cancel: Option<Vec<u8>>,
}

/// Consume every whole message at the front of `seen` (bytes already forwarded to the client),
/// stopping after the first `'Z'` or `'E'`. **Pure.** `Err` when more than [`OBSERVE_CAP`] bytes
/// are left with no whole message among them.
pub fn scan(seen: &mut Vec<u8>) -> Result<Scan, &'static str> {
    let mut out = Scan {
        done: false,
        cancel: None,
    };
    while let Some((kind, len)) = frame(seen) {
        if kind == b'K' {
            out.cancel = backend_key(&seen[..len]).map(|k| cancel_request(&k));
        }
        seen.drain(..len);
        if kind == b'Z' || kind == b'E' {
            out.done = true;
            return Ok(out);
        }
    }
    if seen.len() > OBSERVE_CAP {
        return Err("the server sent more than 64 KiB without a whole message");
    }
    Ok(out)
}

/// Forward the server's first messages to the client unchanged, handing the backend's cancel
/// key to `guard`, until the startup phase is over (then the caller copies plainly).
async fn observe_first_messages<R, W>(
    server: &mut R,
    client: &mut W,
    guard: &SessionGuard,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut seen = Vec::new();
    let mut chunk = vec![0u8; 8192];
    loop {
        let n = server.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        // Forwarded first, exactly as received: the gateway observes, it never alters.
        client.write_all(&chunk[..n]).await?;
        client.flush().await?;
        seen.extend_from_slice(&chunk[..n]);
        let found = scan(&mut seen).map_err(std::io::Error::other)?;
        if let Some(c) = found.cancel {
            guard.set_cancel(c);
        }
        if found.done {
            return Ok(());
        }
    }
}

/// Pipe an admitted session until either side closes. The client→server half is a plain copy
/// from the start (SCRAM needs both directions at once); the server→client half is observed
/// through the startup phase, then copied plainly too.
pub async fn pipe<C, U>(client: C, upstream: U, guard: &SessionGuard) -> std::io::Result<()>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: AsyncRead + AsyncWrite + Unpin,
{
    let (mut cr, mut cw) = tokio::io::split(client);
    let (mut ur, mut uw) = tokio::io::split(upstream);
    let up = async { tokio::io::copy(&mut cr, &mut uw).await.map(|_| ()) };
    let down = async {
        observe_first_messages(&mut ur, &mut cw, guard).await?;
        tokio::io::copy(&mut ur, &mut cw).await?;
        cw.shutdown().await
    };
    tokio::select! {
        r = up => r,
        r = down => r,
    }
}

/// Close a refused connection gently: stop writing (so a TLS alert or an `ErrorResponse`
/// already written goes out before a FIN), then read and discard what the peer still sends,
/// for at most [`DRAIN`] or until it closes. Dropping a socket with unread data sends a TCP
/// reset, and a reset can discard the alert before the window reads it, which would turn a
/// refusal into what looks like an outage (controller ruling R1).
pub async fn close_gently<S: AsyncRead + AsyncWrite + Unpin>(mut s: S) {
    let _ = s.shutdown().await;
    let mut sink = vec![0u8; 4096];
    let drain = async {
        while let Ok(n) = s.read(&mut sink).await {
            if n == 0 {
                break;
            }
        }
    };
    let _ = tokio::time::timeout(DRAIN, drain).await;
}

/// Send `cancel` (a whole `CancelRequest`) to Postgres on a fresh connection, and wait (briefly)
/// for Postgres to close it, which it does once it has acted. Errors are returned for the log.
pub async fn send_cancel(upstream: &Upstream, cancel: &[u8]) -> std::io::Result<()> {
    let send = async {
        let mut s = upstream.connect().await?;
        s.write_all(cancel).await?;
        s.flush().await?;
        let mut rest = Vec::new();
        s.read_to_end(&mut rest).await.map(|_| ())
    };
    tokio::time::timeout(DRAIN, send)
        .await
        .unwrap_or_else(|_| Err(std::io::ErrorKind::TimedOut.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut m = vec![kind];
        m.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        m.extend_from_slice(body);
        m
    }

    #[test]
    fn scan_hands_over_the_cancel_key_and_stops_at_ready_for_query() {
        let key: Vec<u8> = (0..8).map(|i| i * 3 + 1).collect();
        let mut seen = [
            msg(b'R', &[0, 0, 0, 0]),
            msg(b'K', &key),
            msg(b'Z', b"I"),
            b"after".to_vec(),
        ]
        .concat();
        let s = scan(&mut seen).unwrap();
        assert!(s.done);
        assert_eq!(s.cancel, Some(cancel_request(&key)));
        assert_eq!(seen, b"after", "nothing past the 'Z' is consumed");
    }

    #[test]
    fn scan_waits_for_a_whole_message_and_caps_what_it_holds() {
        let mut seen = msg(b'S', b"client_encoding\0UTF8\0")[..6].to_vec();
        assert_eq!(
            scan(&mut seen).unwrap(),
            Scan {
                done: false,
                cancel: None
            }
        );
        // A corrupt length looks like an endless message: refused once past the cap.
        let mut huge = vec![b'S', 0x7f, 0xff, 0xff, 0xff];
        huge.resize(OBSERVE_CAP + 1, 0);
        assert!(scan(&mut huge).is_err());
    }
}
