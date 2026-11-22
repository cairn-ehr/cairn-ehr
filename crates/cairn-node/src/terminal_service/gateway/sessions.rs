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
//!
//! WHEN A RUN ENDS (final review I1). Only the run whose tick owns this table sweeps it. A run
//! that ends (a panic, which `run::supervise` restarts) would leave its sessions piping with no
//! sweep at all, so a later revoke could never end them. So the run holds a [`CloseOnDrop`]:
//! dropping the run aborts every session in the table and sends each one's cancel, as a revoke
//! does (a statement left running would hold its locks until it next wrote, beyond any later
//! revoke: final review R1). A session admitted after that (a connection task still walking the
//! steps under the old run) is aborted the moment it opens, before its backend runs any statement.
//!
//! WHY EACH SESSION ENDED is recorded by whoever aborts it, at that moment ([`REVOKED`] or
//! [`RUN_ENDED`]), so the closing log line says the cause that really ended it (R2).
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tokio::task::AbortHandle;

use super::pipe::send_cancel;
use super::upstream::Upstream;

/// The closing line's cause for a session the sweep ended because its key is no longer active.
pub const REVOKED: &str = "ended: the terminal is no longer admitted";
/// The closing line's cause for a session its gateway run's end ended.
pub const RUN_ENDED: &str = "ended: the gateway's run ended (it is restarted)";

/// Why a session was aborted, set once by whoever aborted it.
pub type EndedBy = Arc<OnceLock<&'static str>>;

/// One open session: whose it is, how to stop it, and how to cancel its backend's statement.
struct Entry {
    key: String,
    abort: AbortHandle,
    cancel: Option<Vec<u8>>,
    why: EndedBy,
}

type Table = Arc<Mutex<HashMap<u64, Entry>>>;

/// Every open session. Cheap to clone: clones share one table.
#[derive(Clone, Default)]
pub struct Sessions {
    table: Table,
    next_id: Arc<AtomicU64>,
    /// Set by [`Sessions::close`]; read and written only under the table's lock, so no
    /// session can slip in between the close and its sweep.
    closed: Arc<AtomicBool>,
}

/// Closes its [`Sessions`] when dropped, then sends their cancels to `upstream`: what the
/// gateway's run holds for its whole life.
pub struct CloseOnDrop {
    pub sessions: Sessions,
    pub upstream: Upstream,
}

impl Drop for CloseOnDrop {
    /// Never blocks and never panics (it may run during a panic's unwind): the cancels go on a
    /// spawned task, best-effort. With no runtime, nothing can send the cancel; the statement
    /// runs until it next writes, as a vanished terminal's does.
    fn drop(&mut self) {
        let cancels = self.sessions.close();
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if cancels.is_empty() {
            return;
        }
        let upstream = self.upstream.clone();
        rt.spawn(async move {
            let mut failed = 0;
            let mut last = None;
            for c in cancels {
                if let Err(e) = send_cancel(&upstream, &c).await {
                    failed += 1;
                    last = Some(e);
                }
            }
            if let Some(e) = last {
                eprintln!(
                    "terminal gateway: {failed} statements of a run that ended could not be \
                     cancelled (the last: {e})"
                );
            }
        });
    }
}

/// A session's registration. Dropping it (the pipe ended, however) forgets the session.
pub struct SessionGuard {
    table: Table,
    id: u64,
    why: EndedBy,
}

impl Sessions {
    /// Register a session of terminal `key`, piped by the task `handle` aborts. On a closed
    /// table (its run has ended) the task is aborted at once instead.
    pub fn open(&self, key: &str, handle: AbortHandle) -> SessionGuard {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let why = EndedBy::default();
        let mut table = lock(&self.table);
        if self.closed.load(Ordering::SeqCst) {
            let _ = why.set(RUN_ENDED);
            handle.abort();
        } else {
            let entry = Entry {
                key: key.to_string(),
                abort: handle,
                cancel: None,
                why: why.clone(),
            };
            table.insert(id, entry);
        }
        SessionGuard {
            table: self.table.clone(),
            id,
            why,
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
            .filter_map(|e| end(e, REVOKED))
            .collect()
    }

    /// Abort every session and refuse all later ones: the run that sweeps this table is over.
    /// Returns their cancel requests, which [`CloseOnDrop`] sends: a statement left running
    /// would keep its backend and its locks until it next wrote.
    pub fn close(&self) -> Vec<Vec<u8>> {
        let mut table = lock(&self.table);
        self.closed.store(true, Ordering::SeqCst);
        table
            .drain()
            .filter_map(|(_, e)| end(e, RUN_ENDED))
            .collect()
    }
}

/// Record why `e` ended (unless already recorded), abort it, and hand back its cancel request.
fn end(e: Entry, why: &'static str) -> Option<Vec<u8>> {
    let _ = e.why.set(why);
    e.abort.abort();
    e.cancel
}

impl SessionGuard {
    /// Where the session's cause of ending will be recorded: the piping task reads it once the
    /// pipe is aborted. Take it before handing the guard to the pipe.
    pub fn ended_by(&self) -> EndedBy {
        self.why.clone()
    }

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
        assert_eq!(b.ended_by().get(), Some(&REVOKED));
        assert!(gone.await.unwrap_err().is_cancelled());
        assert!(gone_quiet.await.unwrap_err().is_cancelled());
        tokio::task::yield_now().await;
        assert!(!kept.is_finished(), "an active key's session runs on");
        // Forgotten once closed: the next sweep sends nothing again.
        assert!(sessions.close_all_but(&set(&["aa"])).is_empty());
        // The run ends just after the revoke: the revoked session's cause stays the revoke (R2).
        sessions.close();
        assert_eq!(b.ended_by().get(), Some(&REVOKED));
        assert!(
            kept.await.unwrap_err().is_cancelled(),
            "the run's end aborts the rest"
        );
    }

    #[tokio::test]
    async fn a_closed_table_aborts_its_sessions_and_every_later_one() {
        let sessions = Sessions::default();
        let (open, late) = (forever(), forever());
        let a = sessions.open("aa", open.abort_handle());
        a.set_cancel(vec![7]);
        assert_eq!(
            sessions.close(),
            vec![vec![7]],
            "the cancel is handed back to be sent"
        );
        assert!(open.await.unwrap_err().is_cancelled());
        assert_eq!(a.ended_by().get(), Some(&RUN_ENDED));
        let b = sessions.open("aa", late.abort_handle());
        assert_eq!(b.ended_by().get(), Some(&RUN_ENDED));
        let late = tokio::time::timeout(std::time::Duration::from_secs(1), late).await;
        assert!(late
            .expect("aborted as it opens")
            .unwrap_err()
            .is_cancelled());
        assert!(sessions.close_all_but(&set(&[])).is_empty());
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
