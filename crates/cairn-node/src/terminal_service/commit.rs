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

/// The most drafts one tick tries (final review F5, ruling R18). Each is its own sealed, signed
/// transaction, and the registration requests wait while the tick runs, so a backlog (a service
/// that was down for a day) is drained at this many a minute rather than in one long pass.
pub const MAX_DRAFTS_PER_TICK: usize = 20;

/// What the service's loop remembers between ticks. Owned by the caller for the life of its
/// loop; nothing here is global state.
#[derive(Debug, Default)]
pub struct CommitMemory {
    /// The stuck drafts already said on stderr, so each is said once per run, not every minute.
    said: HashSet<Uuid>,
    /// The drafts whose last try did not go in (stuck, or an error): tried after the others.
    failing: HashSet<Uuid>,
}

/// The drafts this tick tries, from the listing `listed` (oldest due first): at most
/// [`MAX_DRAFTS_PER_TICK`], with the drafts in `failing` moved after the others. **Pure.**
///
/// WHY FAILING LAST. A stuck draft stays overdue until its writer is enrolled again, and a draft
/// whose commit errs may err on every pass. Oldest first alone would let twenty such drafts,
/// being the oldest, fill every tick for ever while newer drafts never went in. They are still
/// tried whenever the tick has room, so one that recovers goes in.
fn this_tick(listed: Vec<Uuid>, failing: &HashSet<Uuid>) -> Vec<Uuid> {
    let (again, fresh): (Vec<Uuid>, Vec<Uuid>) =
        listed.into_iter().partition(|d| failing.contains(d));
    fresh
        .into_iter()
        .chain(again)
        .take(MAX_DRAFTS_PER_TICK)
        .collect()
}

/// The drafts in `stuck` not yet in `said`, which now holds them too.
fn newly_stuck(said: &mut HashSet<Uuid>, stuck: Vec<Uuid>) -> Vec<Uuid> {
    stuck.into_iter().filter(|d| said.insert(*d)).collect()
}

/// One stderr line for a draft whose commit failed: its id, the SQLSTATE if the database gave
/// one, and the error's PRIMARY message only. Never DETAIL, HINT or context (Postgres puts row
/// values there), never note text, never a patient name. A non-database error says its outermost
/// sentence only. Pure.
pub fn failure_line(draft: Uuid, e: &anyhow::Error) -> String {
    format!(
        "draft {draft} is overdue but could not go into the record: {}",
        primary_message(e)
    )
}

/// An error's PRIMARY message (with the SQLSTATE if it came from the database) and nothing else:
/// never DETAIL, HINT or context, which Postgres fills with row values. The one renderer every
/// log line of the terminal service uses; never `{e:#}` or `{e:?}`. Pure.
pub fn primary_message(e: &anyhow::Error) -> String {
    let db = e
        .chain()
        .find_map(|c| c.downcast_ref::<tokio_postgres::Error>())
        .and_then(|e| e.as_db_error());
    match db {
        Some(db) => format!("{} [{}]", db.message(), db.code().code()),
        None => e.to_string(),
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

/// One pass over every chart: commit each overdue draft in its own transaction (at most
/// [`MAX_DRAFTS_PER_TICK`], the rest wait for the next tick: [`this_tick`]), then record the
/// outcome. A draft that errs or is stuck (its writer has no key) is recorded failed; one that
/// went in or was not written (not due, locked, gone) is cleared. One draft's
/// failure never stops the others. Only the listing's own failure is `Err`.
///
/// `memory` is the caller's [`CommitMemory`], kept across ticks.
pub async fn commit_tick(
    client: &mut Client,
    node_sk: &SigningKey,
    node_origin: &str,
    memory: &mut CommitMemory,
) -> anyhow::Result<CommitReport> {
    let mut report = CommitReport::default();
    let listed = overdue_drafts(client, None).await?;
    for draft in this_tick(listed, &memory.failing) {
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
        let went_in_or_skipped = matches!(
            outcome,
            Ok(CommitOutcome::Committed(_) | CommitOutcome::NothingWritten)
        );
        if went_in_or_skipped {
            memory.failing.remove(&draft);
        } else {
            memory.failing.insert(draft);
        }
        report.record(draft, outcome);
    }
    for draft in newly_stuck(&mut memory.said, report.stuck.clone()) {
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
    fn a_tick_takes_twenty_with_the_failing_last() {
        let listed: Vec<Uuid> = (0..25).map(Uuid::from_u128).collect();
        assert_eq!(this_tick(listed.clone(), &HashSet::new()), listed[..20]);
        // The five oldest failed last time: the twenty after them go first.
        let failing: HashSet<Uuid> = listed[..5].iter().copied().collect();
        assert_eq!(this_tick(listed.clone(), &failing), listed[5..]);
        // With room, the failing are tried too, after the others, in the listing's order.
        let few = listed[..7].to_vec();
        let order: Vec<Uuid> = listed[5..7].iter().chain(&listed[..5]).copied().collect();
        assert_eq!(this_tick(few, &failing), order);
    }

    #[test]
    fn a_stuck_draft_is_said_once() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut said = HashSet::new();
        assert_eq!(newly_stuck(&mut said, vec![a]), vec![a]);
        assert_eq!(newly_stuck(&mut said, vec![a, b]), vec![b]);
        assert!(newly_stuck(&mut said, vec![a, b]).is_empty());
    }
}
