//! The window's one node connection, reopened when it is lost (#807).
//!
//! The reference window opens ONE connection at launch and shares it between the chart
//! commands and the funnel (see [`crate::LiveData`]'s doc). Before #807 that connection was a
//! bare `Mutex<Client>`: after a Postgres restart, or any dropped connection, every later read
//! and write failed until the window itself was restarted.
//!
//! # How it works
//!
//! Every command takes the connection through [`NodeConnection::lock`], exactly as it took the
//! old mutex. `lock` checks whether the client has CLOSED: its connection task ended because
//! the server hung up (a restart's FATAL) or the socket was reset. Only then does it open a
//! fresh connection and put it in the old one's place, so every holder of the shared `Arc` gets
//! the new one.
//!
//! - **Never on a refusal.** A floor refusal, a constraint, a syntax error: each is an ANSWER on
//!   a live connection, which stays open, so `lock` never reopens for one.
//! - **Never mid-command.** The check runs only when the lock is taken, before the command does
//!   anything; no transaction can be open on a client nobody holds. A write whose commit got no
//!   answer is NOT retried: the next command once the node answers again gets a working
//!   connection, so the re-read that follows it can say what landed.
//! - **Backoff.** A failed reopen is not tried again for [`NodeConnection::RETRY_AFTER`], so a
//!   node that is still down costs one attempt per press, not one per lock a gesture takes. An
//!   attempt that gets no answer gives up after [`NodeConnection::REOPEN_WITHIN`], so an
//!   unreachable host cannot hold the lock for a TCP timeout.
//! - **If reopening fails**, `lock` hands back the closed client. Its next query fails with
//!   "connection closed", which every caller already reports as an outage it may retry. The
//!   CAUSE (a refused socket, a rejected password) reaches only the log, not the screen (#876).
//!
//! Not detected: a socket that dies SILENTLY (a pulled cable, a powered-off remote host) never
//! closes the client, so the next query hangs, holding the lock, until TCP gives up (#875).
//!
//! The reopen uses `cairn_node::db::connect`, not `connect_and_load_schema`: the schema was loaded
//! at launch, and replaying every `db/*.sql` file takes locks a read never needs (#726). It also
//! skips the deferred-event re-adjudication `connect_and_load_schema` runs; a window restart, or
//! any other node process connecting, still does that.
use std::time::{Duration, Instant};
use tokio::sync::{MappedMutexGuard, Mutex, MutexGuard};
use tokio_postgres::Client;

/// One node connection, shared through an `Arc` and taken with [`NodeConnection::lock`].
pub struct NodeConnection {
    held: Mutex<Held>,
    /// Where to reopen from. `None` for [`NodeConnection::fixed`]: that one is never replaced.
    reopen_from: Option<String>,
}

/// What the lock guards: the client, and when the last reopen of it failed. Kept under ONE
/// tokio mutex, so the failure stamp needs no lock of its own.
struct Held {
    client: Client,
    last_failed: Option<Instant>,
}

impl NodeConnection {
    /// After a failed reopen, how long before the next attempt.
    pub const RETRY_AFTER: Duration = Duration::from_secs(2);

    /// How long one reopen attempt may take before it counts as failed.
    pub const REOPEN_WITHIN: Duration = Duration::from_secs(5);

    /// A connection that is reopened, from `conn`, when it is lost. The window's.
    pub fn reopening(client: Client, conn: String) -> Self {
        Self::new(client, Some(conn))
    }

    /// A connection that is never replaced. For a caller that made the client itself and may
    /// have set session state on it (a `SET`) that a fresh connection would silently lack.
    pub fn fixed(client: Client) -> Self {
        Self::new(client, None)
    }

    fn new(client: Client, reopen_from: Option<String>) -> Self {
        Self {
            held: Mutex::new(Held {
                client,
                last_failed: None,
            }),
            reopen_from,
        }
    }

    /// Take the connection, reopening it first if it has closed (see the module doc).
    pub async fn lock(&self) -> MappedMutexGuard<'_, Client> {
        let mut held = self.held.lock().await;
        if held.client.is_closed() {
            if let Some(conn) = &self.reopen_from {
                held.reopen(conn).await;
            }
        }
        MutexGuard::map(held, |held| &mut held.client)
    }
}

impl Held {
    /// Put a fresh connection in the closed client's place, unless an attempt failed too
    /// recently. On failure the closed client stays. Logged either way: the loss itself is
    /// logged by `db::connect`'s connection task, the outcome of reopening only here.
    async fn reopen(&mut self, conn: &str) {
        if !may_retry(self.last_failed, Instant::now()) {
            return;
        }
        let attempt = cairn_node::db::connect(conn);
        let cause = match tokio::time::timeout(NodeConnection::REOPEN_WITHIN, attempt).await {
            Ok(Ok(fresh)) => {
                self.client = fresh;
                self.last_failed = None;
                eprintln!("the database connection was lost; reopened it");
                return;
            }
            Ok(Err(e)) => format!("{e:#}"),
            Err(_) => format!("no answer within {:?}", NodeConnection::REOPEN_WITHIN),
        };
        self.last_failed = Some(Instant::now());
        eprintln!("the database connection was lost and could not be reopened: {cause}");
    }
}

/// May a reopen be attempted `now`, given when the last attempt failed? Pure.
fn may_retry(last_failed: Option<Instant>, now: Instant) -> bool {
    last_failed.is_none_or(|t| now.saturating_duration_since(t) >= NodeConnection::RETRY_AFTER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reopen_waits_out_the_backoff_after_a_failure() {
        let retry_after = NodeConnection::RETRY_AFTER;
        let failed = Instant::now();
        assert!(may_retry(None, failed), "no failure yet: try at once");
        assert!(!may_retry(Some(failed), failed), "just failed: wait");
        assert!(!may_retry(
            Some(failed),
            failed + retry_after - Duration::from_millis(1)
        ));
        assert!(may_retry(Some(failed), failed + retry_after));
    }

    /// Every Tauri command holds the guard across `.await`s, so its future must be `Send`.
    /// Pinned here, where the guard is made; otherwise only cairn-gui-tauri's build would
    /// catch a change (a `std` mutex, say) that takes `Send` away.
    #[allow(dead_code)]
    fn the_guard_may_be_held_across_an_await(db: &NodeConnection) -> impl Send + '_ {
        async move {
            let client = db.lock().await;
            std::future::ready(()).await;
            drop(client);
        }
    }
}
