//! The upgrade replay WAITS for its locks instead of stalling readers (#909 slice C1, design
//! §4.6).
//!
//! WHY THIS EXISTS. PostgreSQL grants lock requests in queue order. When the schema replay's
//! `ALTER TABLE` (ACCESS EXCLUSIVE) queues behind one long reader, every LATER reader of that
//! table queues behind the ALTER — so a single slow report could freeze charting for as long
//! as it stays open. [`crate::db::connect_and_load_schema`] therefore sets
//! [`REPLAY_LOCK_TIMEOUT`] once it holds the loader-against-loader advisory lock, and runs
//! each replayed file, the re-adjudication, the heal and the stamp through this module. A wait
//! longer than the timeout fails with SQLSTATE `55P03`; the queued readers go through; we roll
//! back, name the sessions that may be holding it, sleep a [`backoff`], and try the WHOLE step
//! again — indefinitely, so the owner can decide what to do about a blocker the log names.
//!
//! WHY RE-RUNNING A WHOLE FILE IS SAFE. A file's earlier transactions may already have
//! committed when a later one times out (db/033, db/034, db/079 carry several). That is fine:
//! every file is already replayed whole on every changed connect, so each must be idempotent.
//!
//! `lock_timeout` bounds the WAIT for a lock, never how long a statement runs.
use std::future::Future;
use std::time::Duration;

use tokio_postgres::Client;

use crate::db_diagnosis::legible_db_error;

/// How long one replay step may wait for ONE lock before it steps aside and retries. Short
/// enough that a reader queued behind that wait is held ~2 s by it; locks the step already holds
/// stay held through its later waits and its own run. The figure is the builder's.
pub const REPLAY_LOCK_TIMEOUT: &str = "2s";

/// Bound this session's lock waits for the replay. [`crate::db::connect_and_load_schema`] calls
/// it right AFTER taking its advisory load lock, so loader-against-loader waits stay blocking.
/// Its own batch, outside any transaction, so a replayed file's ROLLBACK cannot undo it.
pub async fn arm(client: &Client) -> anyhow::Result<()> {
    client
        .batch_execute(&format!("SET lock_timeout = '{REPLAY_LOCK_TIMEOUT}'"))
        .await
        .map_err(|e| anyhow::anyhow!("setting the replay lock timeout: {}", legible_db_error(&e)))
}

/// Undo [`arm`] once the stamp is written: the loader's client lives on as the daemon's
/// connection. RESET returns to the session's own default, so a `lock_timeout` given in the
/// connection string survives.
pub async fn disarm(client: &Client) -> anyhow::Result<()> {
    client
        .batch_execute("RESET lock_timeout")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "resetting the replay lock timeout: {}",
                legible_db_error(&e)
            )
        })
}

/// `55P03` (`lock_not_available`) is what a `lock_timeout` raises. Only that one is retried:
/// any other failure — a deadlock, a statement timeout, a broken file — is reported at once.
pub fn is_lock_timeout(sqlstate: Option<&str>) -> bool {
    sqlstate == Some("55P03")
}

/// The pause before retry number `attempt + 1`: 250 ms, doubling, capped at 5 s.
pub fn backoff(attempt: u32) -> Duration {
    // min(5) keeps the shift far from overflow; 250 << 5 = 8 s, which the cap cuts to 5 s.
    Duration::from_millis((250u64 << attempt.min(5)).min(5_000))
}

/// Replay one file body (`sql`), waiting out lock timeouts. Any other error comes back with
/// exactly the text [`crate::db::load_migration`] gives it (issue #467's acceptance criterion:
/// it names the migration, the SQLSTATE and the server's message).
pub async fn run_waiting(client: &Client, what: &str, sql: &str) -> anyhow::Result<()> {
    retry_on_lock_timeout(client, what, || client.batch_execute(sql))
        .await
        .map_err(|e| anyhow::anyhow!("loading {what}: {}", legible_db_error(&e)))
}

/// The one retry loop: run `attempt`; on `55P03` roll back, name the blockers, sleep, repeat.
/// Generic over the attempt so callers that need the rows back (the re-adjudication) or a row
/// count (the heal) share it with the file replay. Any other error is returned unchanged, so
/// each caller keeps its own error wording.
pub(crate) async fn retry_on_lock_timeout<T, F, Fut>(
    client: &Client,
    what: &str,
    mut attempt: F,
) -> Result<T, tokio_postgres::Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, tokio_postgres::Error>>,
{
    let mut tries: u32 = 0;
    loop {
        match attempt().await {
            Err(e) if is_lock_timeout(e.code().map(|c| c.code())) => {
                // A timeout inside the file's own BEGIN leaves the session in an aborted
                // transaction (a bare retry would fail 25P02). Outside one, ROLLBACK is a
                // harmless warning. Its own error is ignored: the retry reports anything real.
                let _ = client.batch_execute("ROLLBACK").await;
                let pause = backoff(tries);
                eprintln!(
                    "schema replay: {what} is waiting for a lock (attempt {}); {}; \
                     retrying in {} ms",
                    tries + 1,
                    blockers(client).await,
                    pause.as_millis()
                );
                tokio::time::sleep(pause).await;
                tries = tries.saturating_add(1);
            }
            other => return other,
        }
    }
}

/// One line naming the sessions that MAY be holding the lock we gave up on — candidates to
/// look at, never a verdict: an operator must not end a session on this line alone.
///
/// Why not `pg_blocking_pids(pg_backend_pid())`: it lists who blocks a wait IN PROGRESS, and
/// once the timeout has cancelled our statement we wait on nothing, so it is always empty
/// (checked on PG 18). With only this one connection, the honest substitute is the sessions in
/// this database that hold a table or advisory lock inside a transaction open at least as long
/// as the timeout — a blocker must have held its lock that long. Advisory, because the
/// re-adjudication pass waits on custody locks and on its own (db/043, #1175). Oldest first, at
/// most five. Public for `tests/replay_lock_timeout.rs`, which pins the advisory case.
pub async fn blockers(client: &Client) -> String {
    let rows = client
        .query(
            "SELECT DISTINCT a.pid, coalesce(nullif(a.application_name, ''), '(unnamed)'), \
                    date_trunc('second', now() - a.xact_start)::text, a.xact_start \
               FROM pg_locks l JOIN pg_stat_activity a ON a.pid = l.pid \
              WHERE l.granted AND l.locktype IN ('relation', 'advisory') \
                AND a.datname = current_database() AND a.pid <> pg_backend_pid() \
                AND a.xact_start <= now() - current_setting('lock_timeout')::interval \
              ORDER BY a.xact_start LIMIT 5",
            &[],
        )
        .await;
    match rows {
        Ok(rows) if rows.is_empty() => {
            "no long-open session found holding a table or advisory lock".into()
        }
        Ok(rows) => {
            let named: Vec<String> = rows
                .iter()
                .map(|r| {
                    let pid: i32 = r.get(0);
                    let app: String = r.get(1);
                    let open_for: String = r.get(2);
                    format!("{app} (pid {pid}, transaction open {open_for})")
                })
                .collect();
            format!("these sessions may be holding it: {}", named.join(", "))
        }
        Err(e) => format!("could not list the blockers: {}", legible_db_error(&e)),
    }
}

#[cfg(test)]
#[path = "replay_wait_tests.rs"]
mod tests;
