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
//! old mutex. `lock` checks whether the client has CLOSED (its connection task ended: the server
//! hung up, or the socket died). Only then does it open a fresh connection and put it in the old
//! one's place, so every holder of the shared `Arc` gets the new one.
//!
//! - **Never on a refusal.** A floor refusal, a constraint, a syntax error: each is an ANSWER on
//!   a live connection, which stays open, so `lock` never reopens for one.
//! - **Never mid-command.** The check runs only when the lock is taken, before the command does
//!   anything; no transaction can be open on a client nobody holds. A write whose commit got no
//!   answer is NOT retried: the next command simply gets a working connection, so the re-read
//!   that follows it can say what landed.
//! - **Backoff.** A failed reopen is not tried again for [`RETRY_AFTER`], so a node that is
//!   still down costs one attempt per press, not one per lock a gesture takes. An attempt that
//!   gets no answer gives up after [`REOPEN_WITHIN`], so an unreachable host cannot hold the lock
//!   for a TCP timeout.
//! - **If reopening fails**, `lock` hands back the closed client. Its next query fails with
//!   "connection closed", which every caller already reports as an outage it may retry.
//!
//! The reopen uses `cairn_node::db::connect`, not `connect_and_load_schema`: the schema was loaded
//! at launch, and replaying every `db/*.sql` file takes locks a read never needs (#726).
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, MutexGuard};
use tokio_postgres::Client;

/// After a failed reopen, how long before the next attempt.
pub const RETRY_AFTER: Duration = Duration::from_secs(2);

/// How long one reopen attempt may take before it counts as failed.
pub const REOPEN_WITHIN: Duration = Duration::from_secs(5);

/// One node connection, shared through an `Arc` and taken with [`NodeConnection::lock`].
pub struct NodeConnection {
    client: Mutex<Client>,
    /// `None` for [`NodeConnection::fixed`]: that connection is never replaced.
    reopen: Option<Reopen>,
}

/// What reopening needs: where to connect, and when the last attempt failed.
struct Reopen {
    conn: String,
    /// Only touched while `client` is held, through [`Reopen::last_failed`].
    last_failed: std::sync::Mutex<Option<Instant>>,
}

impl NodeConnection {
    /// A connection that is reopened, from `conn`, when it is lost. The window's.
    pub fn reopening(client: Client, conn: String) -> Self {
        Self {
            client: Mutex::new(client),
            reopen: Some(Reopen {
                conn,
                last_failed: std::sync::Mutex::new(None),
            }),
        }
    }

    /// A connection that is never replaced. For a caller that made the client itself and may
    /// have set session state on it (a `SET`) that a fresh connection would silently lack.
    pub fn fixed(client: Client) -> Self {
        Self {
            client: Mutex::new(client),
            reopen: None,
        }
    }

    /// Take the connection, reopening it first if it has closed (see the module doc).
    pub async fn lock(&self) -> MutexGuard<'_, Client> {
        let mut client = self.client.lock().await;
        if client.is_closed() {
            if let Some(reopen) = &self.reopen {
                reopen.replace(&mut client).await;
            }
        }
        client
    }
}

impl Reopen {
    /// Put a fresh connection in `client`'s place, unless an attempt failed too recently. On
    /// failure `client` is left as it was, closed. Logged either way: this is the only place an
    /// operator can see the window lost and regained its node.
    async fn replace(&self, client: &mut Client) {
        if !may_retry(*self.last_failed(), Instant::now()) {
            return;
        }
        match tokio::time::timeout(REOPEN_WITHIN, cairn_node::db::connect(&self.conn)).await {
            Ok(Ok(fresh)) => {
                *client = fresh;
                *self.last_failed() = None;
                eprintln!("the database connection was lost; reopened it");
            }
            Ok(Err(e)) => {
                *self.last_failed() = Some(Instant::now());
                eprintln!("the database connection was lost and could not be reopened: {e:#}");
            }
            Err(_) => {
                *self.last_failed() = Some(Instant::now());
                eprintln!(
                    "the database connection was lost; reopening it got no answer within \
                     {REOPEN_WITHIN:?}"
                );
            }
        }
    }

    /// The failure stamp, for one statement. Never bound across an `.await`: a `std` guard
    /// would make [`NodeConnection::lock`]'s future `!Send`, and Tauri commands need `Send`.
    fn last_failed(&self) -> std::sync::MutexGuard<'_, Option<Instant>> {
        self.last_failed.lock().expect("no panic while held")
    }
}

/// May a reopen be attempted `now`, given when the last attempt failed? Pure.
fn may_retry(last_failed: Option<Instant>, now: Instant) -> bool {
    last_failed.is_none_or(|t| now.saturating_duration_since(t) >= RETRY_AFTER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reopen_waits_out_the_backoff_after_a_failure() {
        let failed = Instant::now();
        assert!(may_retry(None, failed), "no failure yet: try at once");
        assert!(!may_retry(Some(failed), failed), "just failed: wait");
        assert!(!may_retry(
            Some(failed),
            failed + RETRY_AFTER - Duration::from_millis(1)
        ));
        assert!(may_retry(Some(failed), failed + RETRY_AFTER));
    }
}
