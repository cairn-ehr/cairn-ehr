//! The quarantine commit as the window SEES it (#909): a draft nobody has typed into for the site's
//! quarantine time goes into the record by itself, unsigned — but the NODE's terminal service
//! does that (`cairn_node::terminal_service::commit`); the window holds no node key and runs no
//! commit. All that is left here is what the pane SAYS of an open draft (`due_words`) and the
//! read of which drafts the service's last try failed (`failed_drafts`, from db/083's fact).
use cairn_node::note::view::instant;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use std::collections::HashSet;
use uuid::Uuid;

/// Said of an overdue draft still on the chart whose last commit did not fail: the node's service
/// has not reached it yet, or a save or Sign held its lock. Either way it is tried again, which is
/// all that is certain (review I1).
const OVERDUE: &str =
    "overdue — not in the record yet; the node's terminal service puts it in unsigned at its next check";
/// Said of an overdue draft whose LAST commit by the node's service failed (the cause is in the
/// service's log). Not "tries again": a failure that repeats every pass would make that a promise
/// nobody keeps, and an abandoned draft has no other way in, so the clinician is told to act.
const FAILED: &str = "overdue — could not go into the record by itself (the node's last try \
     failed; the cause is in the terminal service's log); sign it, or tell the site's operator";
/// Said of a STUCK draft: a writer has no vault key, so every commit rolls it back. Not "sign it":
/// a Sign refuses the same draft (`SignRefusal::NoSuchWriter`), so only re-enrolling them helps.
const STUCK: &str = "overdue, but cannot go into the record by itself — a writer of it is no \
     longer enrolled on this node; it goes in once they are enrolled again";
/// Said of a stuck draft NOT yet due (review I1): it will not go in at its due time either, so it
/// never says when it goes in — only what lets it.
const STUCK_NOT_DUE: &str = "cannot go into the record by itself — a writer of it is no longer \
     enrolled on this node; it can go in only once they are enrolled again";

/// The drafts on `patient`'s chart whose last commit the node's service recorded as failed. A read
/// that fails is logged (draft-free: patient id, SQLSTATE and primary message only, never DETAIL) and answers none, so
/// the notes still read and the pane shows no FAILED rather than no notes.
pub async fn failed_drafts(
    db: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
) -> HashSet<Uuid> {
    match cairn_node::note::commit::drafts_commit_failed(db, patient).await {
        Ok(set) => set,
        Err(e) => {
            eprintln!(
                "could not read which drafts failed to commit on {patient}: {}",
                cairn_node::terminal_service::commit::primary_message(&e)
            );
            HashSet::new()
        }
    }
}

/// When an open draft goes into the record unsigned, in zone `tz` with `today` its date. `stuck`
/// wins whether or not it is due: such a draft never goes in by itself ([`STUCK`] once
/// `overdue`, else [`STUCK_NOT_DUE`]). Otherwise: at its `due` time if nobody types in it; or,
/// `overdue` (the node decided, on its own clock), [`FAILED`] when the service's last commit of
/// it failed, else [`OVERDUE`]. Pure.
pub(crate) fn due_words<Tz: TimeZone>(
    due: DateTime<Utc>,
    overdue: bool,
    stuck: bool,
    failed: bool,
    tz: &Tz,
    today: NaiveDate,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match (overdue, stuck) {
        (true, true) => STUCK.into(),
        (false, true) => STUCK_NOT_DUE.into(),
        (true, false) if failed => FAILED.into(),
        (true, false) => OVERDUE.into(),
        (false, false) => format!(
            "goes into the record unsigned at {} if nobody types in it",
            instant(due, tz, today)
        ),
    }
}

#[cfg(test)]
#[path = "notes_commit_tests.rs"]
mod tests;
