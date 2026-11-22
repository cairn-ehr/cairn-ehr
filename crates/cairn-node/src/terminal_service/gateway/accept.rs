//! The gateway's door before any byte is read (#909 slice B, Review Focus 2): the flood limits,
//! what happens when `accept` itself fails, and the LAN-side keepalive.
//!
//! WHY A FILE OF ITS OWN. These are the pieces an unauthenticated LAN host can lean on, so each is
//! small and tested apart from the accept loop in `serve.rs` that uses them. An `accept` error
//! (`EMFILE`: the process is out of file descriptors; `ENFILE`: the system is) must neither end
//! the gateway nor spin: [`next_connection`] counts it, pauses [`ACCEPT_ERROR_PAUSE`] and tries
//! again, and the 1 s tick logs one summary line naming the last error ([`AcceptErrors::take`]).
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tokio::net::TcpStream;

/// A connection's whole budget before startup, counted from `accept`.
pub const PRE_STARTUP_DEADLINE: Duration = Duration::from_secs(10);
/// Connections walking the pre-startup steps at once.
pub const PRE_STARTUP_ACTIVE: usize = 64;
/// Connections before startup in all (walking or waiting); one more is closed at once.
pub const PRE_STARTUP_TOTAL: usize = 128;
/// The pause after a failed `accept`, so an exhausted descriptor table cannot busy-loop.
pub const ACCEPT_ERROR_PAUSE: Duration = Duration::from_millis(100);

/// Failed `accept`s since the last summary, and the text of the last one.
#[derive(Default)]
pub struct AcceptErrors {
    count: AtomicU64,
    last: Mutex<String>,
}

impl AcceptErrors {
    fn record(&self, e: &std::io::Error) {
        self.count.fetch_add(1, Ordering::Relaxed);
        *self.last.lock().unwrap_or_else(|p| p.into_inner()) = e.to_string();
    }

    /// `(how many, the last error)` since the previous call, or `None` if there were none.
    pub fn take(&self) -> Option<(u64, String)> {
        let n = self.count.swap(0, Ordering::Relaxed);
        let last = self.last.lock().unwrap_or_else(|p| p.into_inner()).clone();
        (n > 0).then_some((n, last))
    }
}

/// The next connection `accept` yields. An error is recorded in `errors` and followed by
/// [`ACCEPT_ERROR_PAUSE`]; it is never returned, so it can never end the accept loop.
pub async fn next_connection<F, Fut, T>(mut accept: F, errors: &AcceptErrors) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::io::Result<T>>,
{
    loop {
        match accept().await {
            Ok(c) => return c,
            Err(e) => {
                errors.record(&e);
                tokio::time::sleep(ACCEPT_ERROR_PAUSE).await;
            }
        }
    }
}

/// LAN-side dead-peer detection, with the numbers the window's own link uses (Global
/// Constraints). Set on the accepted socket before the TLS handshake.
pub fn keepalive(tcp: &TcpStream) -> std::io::Result<()> {
    use cairn_db_keepalive::{KEEPALIVE_IDLE, KEEPALIVE_INTERVAL, KEEPALIVE_RETRIES};
    // shortcut: interval/retries compile on Linux, Android, macOS, BSDs and Windows only; gate
    // them if Cairn ever targets another OS.
    let ka = socket2::TcpKeepalive::new()
        .with_time(KEEPALIVE_IDLE)
        .with_interval(KEEPALIVE_INTERVAL)
        .with_retries(KEEPALIVE_RETRIES);
    let s = socket2::SockRef::from(tcp);
    s.set_tcp_keepalive(&ka)?;
    // What ends a terminal that vanished with a result in flight; Linux (and Android) only.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    s.set_tcp_user_timeout(Some(cairn_db_keepalive::TCP_USER_TIMEOUT))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emfile() -> std::io::Error {
        std::io::Error::from_raw_os_error(24) // EMFILE on Linux and macOS
    }

    /// Three failed accepts, then a connection: each failure is counted and paused on, and the
    /// loop goes on to hand back the connection (it does not end, and it does not spin).
    #[tokio::test]
    async fn an_accept_error_is_counted_paused_on_and_never_ends_the_loop() {
        let errors = AcceptErrors::default();
        let mut calls = 0;
        let start = std::time::Instant::now();
        let got = next_connection(
            || {
                calls += 1;
                let r = if calls <= 3 { Err(emfile()) } else { Ok(calls) };
                async move { r }
            },
            &errors,
        )
        .await;
        assert_eq!(got, 4, "the connection after the errors is returned");
        assert!(
            start.elapsed() >= ACCEPT_ERROR_PAUSE * 3,
            "a pause after each error, not a busy loop: {:?}",
            start.elapsed()
        );
        let (n, last) = errors.take().expect("the errors are counted");
        assert_eq!(n, 3);
        assert_eq!(last, emfile().to_string(), "the summary names the error");
        assert_eq!(errors.take(), None, "the count resets");
    }
}
