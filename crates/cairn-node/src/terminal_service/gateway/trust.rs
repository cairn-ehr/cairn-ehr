//! The gateway's live terminal trust set (#909 slice B, D2 step 2).
//!
//! WHY IT EXISTS. The TLS handshake asks one question about a terminal: is its key `active` in
//! `terminal_trust` (db/084)? rustls asks it inside the handshake, synchronously, so it cannot
//! wait on the database. [`TerminalTrust`] holds the answer in memory instead: the gateway's
//! 1 s background tick calls [`TerminalTrust::refresh`], and the handshake's closure
//! ([`TerminalTrust::store`]) reads the copy.
//!
//! Three things the closure does for an unknown key, all without blocking the handshake:
//! it refuses it (rustls then sends the `access_denied` alert, which the window reads as "not
//! admitted"), it queues the key on a bounded channel so the tick can record it as a `pending`
//! attempt (an operator then sees it and can admit it), and it bumps a counter the tick logs
//! once a second. A full queue drops the key: a LAN host flooding fresh keys costs the gateway
//! nothing, and db/084 keeps only the 64 newest pending rows anyway. An ACTIVE key is queued too
//! (never counted): the same door only touches a known row's `last_seen`, so `list-terminals`
//! shows when an admitted terminal last connected, not when it was last refused (final review M2).
//!
//! Two states are kept apart on purpose (controller ruling R2):
//! * NEVER LOADED (`None`): the closure accepts no key and records nothing, because "the
//!   gateway does not know yet" is not "this terminal is unknown". (The gateway does not
//!   accept connections before its first load, so this is defence in depth.)
//! * A failed refresh KEEPS the last loaded set: a stale set beats an empty one, which would
//!   shut every admitted terminal out over a database hiccup.
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use anyhow::Context;
use tokio::sync::mpsc;
use tokio_postgres::Client;

use crate::transport::TrustStore;

/// How many seen keys may wait to be recorded. Matches db/084's cap on pending rows.
pub const ATTEMPT_QUEUE: usize = 64;

/// The live trust set, the attempt queue's sending end and the refused-handshake counter.
/// Cheap to clone: every clone shares the same set, queue and counter.
#[derive(Clone)]
pub struct TerminalTrust {
    active: Arc<RwLock<Option<HashSet<String>>>>,
    attempts: mpsc::Sender<String>,
    refused: Arc<AtomicU64>,
}

impl TerminalTrust {
    /// An empty, never-loaded trust set, and the receiving end of its attempt queue (the
    /// gateway's tick drains it into `cairn_terminal_attempt`).
    pub fn new() -> (TerminalTrust, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel(ATTEMPT_QUEUE);
        let trust = TerminalTrust {
            active: Arc::new(RwLock::new(None)),
            attempts: tx,
            refused: Arc::new(AtomicU64::new(0)),
        };
        (trust, rx)
    }

    /// The closure the gateway's TLS server config pins client keys with: `true` for an active
    /// key. Every key is queued (if there is room) for its `last_seen`; any other key is also
    /// counted, then refused. A poisoned
    /// lock fails closed, as `sync::trust_store_from_set` does.
    pub fn store(&self) -> TrustStore {
        let me = self.clone();
        Arc::new(move |key: &str| {
            let verdict = me
                .active
                .read()
                .map(|s| s.as_ref().map(|s| s.contains(key)));
            match verdict {
                Ok(Some(true)) => {
                    let _ = me.attempts.try_send(key.to_string());
                    true
                }
                Ok(Some(false)) => {
                    // `try_send`, never `send`: the handshake must not wait on the queue.
                    let _ = me.attempts.try_send(key.to_string());
                    me.refused.fetch_add(1, Ordering::Relaxed);
                    false
                }
                // Never loaded (or poisoned): refuse, and record nothing.
                _ => false,
            }
        })
    }

    /// Re-read the active keys from `terminal_trust` and replace the set with them; returns the
    /// new set. On an error the set is left as it was (stale beats empty).
    pub async fn refresh(&self, db: &Client) -> anyhow::Result<HashSet<String>> {
        let rows = db
            .query(
                "SELECT terminal_pubkey FROM terminal_trust WHERE status = 'active'",
                &[],
            )
            .await
            .context("reading the active terminal keys")?;
        let fresh: HashSet<String> = rows.iter().map(|r| r.get(0)).collect();
        self.replace(fresh.clone());
        Ok(fresh)
    }

    /// The refused handshakes since the last call, resetting the count (for the 1 s summary).
    pub fn take_refused(&self) -> u64 {
        self.refused.swap(0, Ordering::Relaxed)
    }

    fn replace(&self, set: HashSet<String>) {
        if let Ok(mut s) = self.active.write() {
            *s = Some(set);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn an_unknown_key_is_refused_queued_and_counted() {
        let (trust, mut rx) = TerminalTrust::new();
        trust.replace(set(&["aa"]));
        let store = trust.store();
        assert!(store("aa"), "an active key passes");
        // Queued too, so its last_seen is touched (list-terminals, final review M2), but it is
        // not a refused handshake.
        assert_eq!(rx.try_recv().unwrap(), "aa");
        assert_eq!(
            trust.take_refused(),
            0,
            "an active key is not counted as refused"
        );
        assert!(!store("bb"), "an unknown key is refused");
        assert_eq!(rx.try_recv().unwrap(), "bb");
        assert_eq!(trust.take_refused(), 1);
        assert_eq!(trust.take_refused(), 0, "the count resets");
    }

    #[test]
    fn a_full_attempt_channel_drops_without_blocking_the_handshake() {
        let (trust, mut rx) = TerminalTrust::new();
        trust.replace(set(&[]));
        let store = trust.store();
        for i in 0..100 {
            assert!(!store(&format!("k{i}")));
        }
        let mut queued = 0;
        while rx.try_recv().is_ok() {
            queued += 1;
        }
        assert_eq!(queued, ATTEMPT_QUEUE, "the queue keeps what fits");
        assert_eq!(trust.take_refused(), 100, "every refusal is counted");
    }

    #[test]
    fn before_the_first_load_no_key_passes_and_none_is_recorded() {
        let (trust, mut rx) = TerminalTrust::new();
        assert!(!trust.store()("aa"));
        assert!(rx.try_recv().is_err(), "not recorded as an attempt");
        assert_eq!(
            trust.take_refused(),
            0,
            "not counted as an unadmitted handshake"
        );
        trust.replace(set(&["aa"]));
        assert!(trust.store()("aa"), "after a load, the same key passes");
    }
}
