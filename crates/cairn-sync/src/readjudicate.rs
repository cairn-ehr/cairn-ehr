//! The sync daemon's routine re-adjudication of deferred events (#912).
//!
//! WHAT IT IS FOR. A peer's door admits an event it cannot yet interpret, or one whose target has
//! not arrived, as DEFERRED: stored, but powerless (ADR-0056). `cairn_readjudicate_deferred()`
//! (db/043) promotes it once what it waited for is here. Until #912 only the schema loader ran that
//! pass, so on a running node a deferred event stayed powerless until an operator happened to run a
//! loader verb. `cairn-sync run` is the process most deferred events ARRIVE through, so its pull
//! loop now runs the pass too.
//!
//! WHEN. [`due`]: after a cycle that applied new events (one of them may be what a deferred event
//! waited for), and on the loop's full-sweep cycle, which catches what arrives some other way — an
//! actor vouch on the node plane, a code-plane upgrade that classified a type, a medium import, a
//! requeue. On a healthy node `event_deferred` holds no row of a type this node can classify, and
//! the pass is one cheap query.
//!
//! NEVER PARKS SYNC ON A LOCK. [`run`] takes its own transaction under a [`LOCK_TIMEOUT`]: a
//! promotion that has to wait for another transaction's lock (a late key, a shred, a running heal,
//! another pass) gives up with `55P03` (db/043 re-raises it since #1175) instead of holding up
//! clinical sync, the availability floor. The timeout bounds each ONE wait, not the pass: the pass
//! runs on the pull loop's thread, so the loop pulls nothing while it runs. The caller logs
//! [`failure_line`] and the next due cycle tries again; no failure here ends the loop.
use postgres::error::SqlState;
use postgres::Client;

/// How long the pass may wait for any ONE lock before it gives up until the next due cycle. The
/// loader's `REPLAY_LOCK_TIMEOUT` (ADR-0111 decision 7); a unit test keeps the two equal.
pub const LOCK_TIMEOUT: &str = "2s";

/// Is a pass due this cycle? **Pure.** `applied_new` is the cycle's newly applied event count
/// (0 when the pull failed); `full_sweep` is the loop's every-Nth-cycle sweep.
pub fn due(applied_new: u64, full_sweep: bool) -> bool {
    applied_new > 0 || full_sweep
}

/// Run one pass in its own transaction under [`LOCK_TIMEOUT`]. Returns how many deferred events
/// it promoted. Callable as the runtime role (db/043 grants it to `cairn_node`).
pub fn run(client: &mut Client) -> Result<i64, postgres::Error> {
    let mut tx = client.transaction()?;
    // SET LOCAL: the timeout ends with this transaction, so the pull's own statements are untouched.
    tx.batch_execute(&format!("SET LOCAL lock_timeout = '{LOCK_TIMEOUT}'"))?;
    let promoted: i64 = tx
        .query_one(
            "SELECT COALESCE(sum(promoted_count), 0)::bigint FROM cairn_readjudicate_deferred()",
            &[],
        )?
        .get(0);
    tx.commit()?;
    Ok(promoted)
}

/// Write one pass's outcome into the cycle's JSONL `line` and the operator's `status`. No database
/// access. Never silent: a failure goes to stderr and to `readjudicate_error`; a success is
/// `readjudicated` (its count), and the status line mentions it only when something was promoted.
pub fn record(
    line: &mut serde_json::Value,
    status: &mut String,
    outcome: &Result<i64, postgres::Error>,
) {
    match outcome {
        Ok(n) => {
            if *n > 0 {
                status.push_str(&format!(", {n} deferred promoted"));
            }
            line["readjudicated"] = serde_json::json!(n);
        }
        Err(e) => {
            let why = failure_line(e);
            eprintln!("{why}");
            line["readjudicate_error"] = serde_json::json!(why);
        }
    }
}

/// The operator's line for a pass that failed. **Pure.** A lock wait is expected under load and
/// says so plainly; every line carries the database's own words (`legible_db_error`), which name
/// what was waited on.
pub fn failure_line(e: &postgres::Error) -> String {
    if e.code() == Some(&SqlState::LOCK_NOT_AVAILABLE) {
        format!(
            "re-adjudicating deferred events waited more than {LOCK_TIMEOUT} for a lock; it runs \
             again on the next cycle that applies events, or the next full sweep: {}",
            crate::legible_db_error(e)
        )
    } else {
        format!(
            "re-adjudicating deferred events failed (it runs again on the next due cycle): {}",
            crate::legible_db_error(e)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quarantine_tests::{cs, locked_client};

    #[test]
    fn the_lock_timeout_is_the_loaders() {
        assert_eq!(LOCK_TIMEOUT, cairn_node::replay_wait::REPLAY_LOCK_TIMEOUT);
    }

    /// The `cmd_run` wiring's success half: the count lands in the JSONL, and the status line
    /// names promotions only when there were some.
    #[test]
    fn a_successful_pass_is_recorded() {
        let mut line = serde_json::json!({});
        let mut status = String::from("cycle 1");
        record(&mut line, &mut status, &Ok(0));
        assert_eq!(line["readjudicated"], 0);
        assert_eq!(
            status, "cycle 1",
            "a pass that promoted nothing adds no noise"
        );

        record(&mut line, &mut status, &Ok(3));
        assert_eq!(line["readjudicated"], 3);
        assert_eq!(status, "cycle 1, 3 deferred promoted");
        assert!(line.get("readjudicate_error").is_none());
    }

    #[test]
    fn a_pass_is_due_after_new_events_or_on_the_full_sweep() {
        assert!(!due(0, false), "a quiet cycle runs no pass");
        assert!(
            due(1, false),
            "a cycle that applied events may have unblocked one"
        );
        assert!(
            due(0, true),
            "the full sweep catches arrivals on other paths"
        );
        assert!(due(5, true));
    }

    /// On cairn-sync's own (subset) schema, as a role holding ONLY `cairn_node` — what
    /// `provision-runtime-role` gives the daemon — the pass runs and promotes nothing on an empty
    /// node. Before #912 this was `permission denied`. It pins the GRANT; the empty set never
    /// reaches the owner-only helpers, so the DEFINER half is pinned with a real deferred row by
    /// cairn-node's `tests/deferred_readjudication_runner.rs`.
    #[test]
    fn the_runtime_role_can_run_a_pass() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        c.batch_execute(
            "DO $$ BEGIN \
               IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_912_sync_probe') THEN \
                 CREATE ROLE cairn_912_sync_probe NOLOGIN; \
               END IF; \
             END $$; \
             GRANT cairn_node TO cairn_912_sync_probe;",
        )
        .unwrap();
        c.batch_execute("SET ROLE cairn_912_sync_probe").unwrap();
        let ran = run(&mut c);
        c.batch_execute("RESET ROLE").unwrap();
        assert_eq!(ran.map_err(|e| failure_line(&e)), Ok(0));
    }

    /// A pass that cannot get a lock gives up within [`LOCK_TIMEOUT`] and says so; it never parks
    /// the pull loop behind another transaction's lock. Its timeout ends with its transaction, the
    /// failure is recorded, and the same client runs the next pass.
    #[test]
    fn a_pass_behind_a_held_lock_gives_up_and_says_why() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        // A backstop so a pass that never times out FAILS (57014) instead of hanging the suite:
        // the lock holder below runs on this thread, after `run` returns.
        c.batch_execute("SET statement_timeout = '15s'").unwrap();
        let session_lock_timeout: String = c.query_one("SHOW lock_timeout", &[]).unwrap().get(0);
        let mut holder = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        let mut held = holder.transaction().unwrap();
        held.batch_execute("LOCK TABLE event_deferred IN ACCESS EXCLUSIVE MODE")
            .unwrap();

        let started = std::time::Instant::now();
        let ran = run(&mut c);
        let waited = started.elapsed();
        held.rollback().unwrap();

        let e = ran
            .as_ref()
            .expect_err("the pass must give up, not wait for the lock");
        assert_eq!(e.code(), Some(&SqlState::LOCK_NOT_AVAILABLE), "{e:?}");
        assert!(
            waited >= std::time::Duration::from_millis(1500)
                && waited < std::time::Duration::from_secs(10),
            "gave up after {waited:?}, not after about {LOCK_TIMEOUT}"
        );
        let mut line = serde_json::json!({});
        let mut status = String::new();
        record(&mut line, &mut status, &ran);
        let why = line["readjudicate_error"].as_str().unwrap_or_default();
        assert!(why.contains("waited more than 2s for a lock"), "{why}");
        assert!(line.get("readjudicated").is_none(), "{line}");

        let after: String = c.query_one("SHOW lock_timeout", &[]).unwrap().get(0);
        assert_eq!(
            after, session_lock_timeout,
            "SET LOCAL: the pull's own statements keep the session's lock_timeout"
        );
        assert_eq!(
            run(&mut c).map_err(|e| failure_line(&e)),
            Ok(0),
            "the next due cycle's pass runs on the same client"
        );
        c.batch_execute("RESET statement_timeout").unwrap();
    }
}
