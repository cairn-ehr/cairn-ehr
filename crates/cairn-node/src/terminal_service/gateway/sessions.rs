//! The gateway's open sessions, by terminal key (#909 slice B, Review Focus 1).
//!
//! WHY IT EXISTS. Revoking a terminal must end what it already has open, not only refuse its
//! next handshake. Each piped session registers here once its startup is admitted, with the
//! [`AbortHandle`] of the task that pipes it; the gateway's 1 s tick then calls
//! [`Sessions::close_all_but`] with the freshly loaded active set, which aborts every other
//! key's sessions.
//!
//! Aborting the pipe closes both sockets, but closing the loopback socket alone does not stop a
//! backend that is busy running a statement (Postgres notices a gone client only when it next
//! reads or writes). So each session also keeps the `CancelRequest` for its backend, built from
//! the `BackendKeyData` the gateway saw during startup ([`SessionGuard::set_cancel`]);
//! `close_all_but` hands those back for the caller to send to Postgres. The running statement is
//! cancelled, the backend then finds its client gone and exits, and its locks are released.
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::task::AbortHandle;

/// One open session: whose it is, how to stop it, and how to cancel its backend's statement.
struct Entry {
    key: String,
    abort: AbortHandle,
    cancel: Option<Vec<u8>>,
}

type Table = Arc<Mutex<HashMap<u64, Entry>>>;

/// Every open session. Cheap to clone: clones share one table.
#[derive(Clone, Default)]
pub struct Sessions {
    table: Table,
    next_id: Arc<AtomicU64>,
}

/// A session's registration. Dropping it (the pipe ended, however) forgets the session.
pub struct SessionGuard {
    table: Table,
    id: u64,
}

impl Sessions {
    /// Register a session of terminal `key`, piped by the task `handle` aborts.
    pub fn open(&self, key: &str, handle: AbortHandle) -> SessionGuard {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let entry = Entry {
            key: key.to_string(),
            abort: handle,
            cancel: None,
        };
        lock(&self.table).insert(id, entry);
        SessionGuard {
            table: self.table.clone(),
            id,
        }
    }

    /// Abort every session whose key is not in `active`, forget them, and return their cancel
    /// requests (those whose backend key was seen) for the caller to send upstream.
    pub fn close_all_but(&self, active: &HashSet<String>) -> Vec<Vec<u8>> {
        let mut table = lock(&self.table);
        let gone: Vec<u64> = table
            .iter()
            .filter(|(_, e)| !active.contains(&e.key))
            .map(|(id, _)| *id)
            .collect();
        gone.iter()
            .filter_map(|id| table.remove(id))
            .filter_map(|e| {
                e.abort.abort();
                e.cancel
            })
            .collect()
    }
}

impl SessionGuard {
    /// Remember the `CancelRequest` bytes for this session's backend (once its key is seen).
    pub fn set_cancel(&self, cancel: Vec<u8>) {
        if let Some(e) = lock(&self.table).get_mut(&self.id) {
            e.cancel = Some(cancel);
        }
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        lock(&self.table).remove(&self.id);
    }
}

/// The table, even if a panic poisoned its lock: every critical section above is a single map
/// operation, so a poisoned table is still consistent, and a sweep that stopped working would
/// leave revoked sessions open.
fn lock(table: &Table) -> std::sync::MutexGuard<'_, HashMap<u64, Entry>> {
    table.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A task that runs until aborted, and its handle.
    fn forever() -> tokio::task::JoinHandle<()> {
        tokio::spawn(std::future::pending::<()>())
    }

    fn set(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    #[tokio::test]
    async fn close_all_but_aborts_only_the_revoked_keys_sessions() {
        let sessions = Sessions::default();
        let (kept, gone, gone_quiet) = (forever(), forever(), forever());
        let _a = sessions.open("aa", kept.abort_handle());
        let b = sessions.open("bb", gone.abort_handle());
        b.set_cancel(vec![1, 2, 3]);
        // A second session of the revoked key whose backend key was never seen: aborted, with
        // no cancel to send.
        let _b2 = sessions.open("bb", gone_quiet.abort_handle());

        let cancels = sessions.close_all_but(&set(&["aa"]));
        assert_eq!(cancels, vec![vec![1, 2, 3]]);
        assert!(gone.await.unwrap_err().is_cancelled());
        assert!(gone_quiet.await.unwrap_err().is_cancelled());
        tokio::task::yield_now().await;
        assert!(!kept.is_finished(), "an active key's session runs on");
        // Forgotten once closed: the next sweep sends nothing again.
        assert!(sessions.close_all_but(&set(&["aa"])).is_empty());
        kept.abort();
    }

    #[tokio::test]
    async fn a_dropped_guard_forgets_its_session() {
        let sessions = Sessions::default();
        let task = forever();
        let guard = sessions.open("aa", task.abort_handle());
        guard.set_cancel(vec![9]);
        drop(guard);
        assert!(
            sessions.close_all_but(&set(&[])).is_empty(),
            "a forgotten session has nothing to cancel"
        );
        tokio::task::yield_now().await;
        assert!(!task.is_finished(), "and is not aborted");
        task.abort();
    }
}
