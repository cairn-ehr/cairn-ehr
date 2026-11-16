//! The service's quarantine tick: put every overdue note draft into the record, node-signed, and
//! leave the clinician a trace of any draft that would not go in (#909, design D7).
//!
//! WHY HERE. The window used to run this tick with the node's key. With the key gone from the
//! window, the service runs it, and since the window can no longer see a failure for itself the
//! service writes it down: db/083's `cairn_note_draft_commit_failed` (true = it failed, false =
//! it went in or is no longer failing), which the window's notes read turns into a warning.
//!
//! BUILT ON `note::commit`: [`overdue_drafts`] lists the hint, [`commit_overdue`] does one draft
//! in one transaction (the database re-decides "overdue" under the draft's lock), and
//! [`CommitReport::record`] counts the outcome. `commit_overdue_drafts` is NOT used because it
//! cannot interleave the failure write between drafts; this loop is the same shape plus that.
//! "Every chart" is `overdue_drafts(None)`, exactly as the window's tick asked.
use std::collections::HashSet;

use cairn_event::SigningKey;
use tokio_postgres::Client;
use uuid::Uuid;

use crate::note::commit::{commit_overdue, overdue_drafts, CommitOutcome, CommitReport};

/// The drafts in `stuck` not yet in `said`, which now holds them too.
fn newly_stuck(said: &mut HashSet<Uuid>, stuck: Vec<Uuid>) -> Vec<Uuid> {
    stuck.into_iter().filter(|d| said.insert(*d)).collect()
}

/// One stderr line for a draft whose commit failed: its id, the SQLSTATE if the database gave
/// one, and the error's PRIMARY message only. Never DETAIL, HINT or context (Postgres puts row
/// values there), never note text, never a patient name. A non-database error says its outermost
/// sentence only. Pure.
pub fn failure_line(draft: Uuid, e: &anyhow::Error) -> String {
    let db = e
        .chain()
        .find_map(|c| c.downcast_ref::<tokio_postgres::Error>())
        .and_then(|e| e.as_db_error());
    match db {
        Some(db) => format!(
            "draft {draft} is overdue but could not go into the record: {} [{}]",
            db.message(),
            db.code().code()
        ),
        None => format!("draft {draft} is overdue but could not go into the record: {e}"),
    }
}

/// Record on the database whether draft `draft`'s last try failed. A failure to WRITE the fact is
/// logged and never ends the tick: the next draft is still tried.
async fn note_failed(client: &Client, draft: Uuid, failed: bool) {
    let sent = client
        .execute(
            "SELECT cairn_note_draft_commit_failed($1::text::uuid, $2)",
            &[&draft.to_string(), &failed],
        )
        .await;
    if let Err(e) = sent {
        eprintln!(
            "draft {draft}: the failed-commit fact could not be written: {}",
            failure_cause(&e)
        );
    }
}

/// A database error's SQLSTATE and primary message, or its kind. Pure.
fn failure_cause(e: &tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => format!("{} [{}]", db.message(), db.code().code()),
        None => e.to_string(),
    }
}

/// One pass over every chart: commit each overdue draft in its own transaction, then record the
/// outcome. A draft that errs or is stuck (its writer has no key) is recorded failed; one that
/// went in or was not written (not due, locked, gone) is cleared. One draft's
/// failure never stops the others. Only the listing's own failure is `Err`.
///
/// `said` is the caller's set of stuck drafts already said on stderr: the caller owns it for the
/// life of its loop, so each stuck draft is said once per run, not every minute, and nothing
/// here is global state.
pub async fn commit_tick(
    client: &mut Client,
    node_sk: &SigningKey,
    node_origin: &str,
    said: &mut HashSet<Uuid>,
) -> anyhow::Result<CommitReport> {
    let mut report = CommitReport::default();
    for draft in overdue_drafts(client, None).await? {
        let outcome = commit_overdue(client, node_sk, draft, node_origin).await;
        match &outcome {
            Ok(CommitOutcome::Committed(_)) => note_failed(client, draft, false).await,
            // Not written (locked, gone, not due) is not failing: clear any old fact. STUCK
            // still wins in the window's copy: `due_words` matches `(overdue, stuck)` first.
            Ok(CommitOutcome::NothingWritten) => note_failed(client, draft, false).await,
            Ok(CommitOutcome::Stuck) => note_failed(client, draft, true).await,
            Err(e) => {
                eprintln!("{}", failure_line(draft, e));
                note_failed(client, draft, true).await;
            }
        }
        report.record(draft, outcome);
    }
    for draft in newly_stuck(said, report.stuck.clone()) {
        eprintln!(
            "draft {draft} is overdue but cannot go into the record: a writer of it is no \
             longer enrolled on this node"
        );
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stuck_draft_is_said_once() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut said = HashSet::new();
        assert_eq!(newly_stuck(&mut said, vec![a]), vec![a]);
        assert_eq!(newly_stuck(&mut said, vec![a, b]), vec![b]);
        assert!(newly_stuck(&mut said, vec![a, b]).is_empty());
    }
}
