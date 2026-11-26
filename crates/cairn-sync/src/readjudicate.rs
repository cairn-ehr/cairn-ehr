//! The sync daemon's routine re-adjudication of deferred events (#912).
//!
//! WHAT IT IS FOR. A peer's door admits an event it cannot yet interpret, or one whose target has
//! not arrived, as DEFERRED: stored, but powerless (ADR-0056). `cairn_readjudicate_deferred()`
//! (db/043) promotes it once what it waited for is here. Until #912 only the schema loader ran that
//! pass, so on a running node a deferred event stayed powerless until an operator happened to run a
//! loader verb. `cairn-sync run` is the process deferred events ARRIVE through, so its pull loop now
//! runs the pass too.
//!
//! WHEN. [`due`]: after a cycle that applied new events (one of them may be what a deferred event
//! waited for), and on the loop's full-sweep cycle, which catches what arrives some other way — an
//! actor vouch on the node plane, a code-plane upgrade that classified a type. On a healthy node
//! `event_deferred` is empty and the pass is one cheap query.
//!
//! NEVER STALLS SYNC. [`run`] takes its own transaction under a [`LOCK_TIMEOUT`]: a promotion that
//! has to wait behind a long reader gives up with `55P03` (db/043 re-raises it since #1175) instead
//! of holding up clinical sync, the availability floor. The caller logs [`failure_line`] and the
//! next due cycle tries again; no failure here ends the loop.
use postgres::error::SqlState;
use postgres::Client;

/// How long one pass may wait for any lock before it gives up until the next due cycle. The same
/// 2 s the loader's replay uses (ADR-0111).
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

/// The operator's line for a pass that failed. **Pure.** A lock wait is expected under load and
/// says so plainly; anything else carries the database's own words (`legible_db_error`).
pub fn failure_line(e: &postgres::Error) -> String {
    if e.code() == Some(&SqlState::LOCK_NOT_AVAILABLE) {
        format!(
            "re-adjudicating deferred events waited more than {LOCK_TIMEOUT} for a lock; it runs \
             again on the next cycle that applies events, or the next full sweep"
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
    /// node. Before #912 this was `permission denied`.
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
    /// the pull loop behind a long reader.
    #[test]
    fn a_pass_behind_a_held_lock_gives_up_and_says_why() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let mut holder = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        let mut held = holder.transaction().unwrap();
        held.batch_execute("LOCK TABLE event_deferred IN ACCESS EXCLUSIVE MODE")
            .unwrap();

        let started = std::time::Instant::now();
        let ran = run(&mut c);
        let waited = started.elapsed();
        held.rollback().unwrap();

        let e = ran.expect_err("the pass must give up, not wait for the lock");
        assert_eq!(e.code(), Some(&SqlState::LOCK_NOT_AVAILABLE), "{e:?}");
        assert!(
            waited < std::time::Duration::from_secs(10),
            "gave up in {waited:?}"
        );
        assert!(
            failure_line(&e).contains("waited more than 2s for a lock"),
            "{}",
            failure_line(&e)
        );
    }
}
