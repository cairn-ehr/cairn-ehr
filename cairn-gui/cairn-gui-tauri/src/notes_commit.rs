//! The window as the quarantine commit's RUNNER (sign-in slice 3b-2a, design §7): a draft nobody
//! has typed into for the site's quarantine time goes into the record by itself, unsigned
//! (`cairn_node::note::commit`). Something has to ask the node to do that, and the window is
//! what every site already runs, so it asks in two places:
//!
//! * **a tick** every `COMMIT_TICK`, over every chart (`run_ticks`, spawned once in `main`'s
//!   setup). A Rust task, not a webview timer: WebKit throttles a hidden window's timers, and a
//!   minimised window must still commit;
//! * **before every chart read** (`notes::note_list_impl`), over that chart only, so the pane never
//!   shows a draft the next tick is about to take away.
//!
//! The node, not this file, decides what is overdue: its door re-checks on the database's clock
//! under the draft's lock, so two windows ticking at once, or a tick racing a Sign, write one
//! note. This file only asks, says on stderr what could not go in, and remembers which drafts'
//! last commit failed (`AppState::commit_failed`) so the pane says so. And it never gets in the
//! way: fixture mode (no key, no connection) commits nothing, a commit that fails is
//! never a failed read (Review Focus 5), each draft is its own short connection hold (a
//! clinician's press waits behind one draft at most, never a whole pass), and the tick never
//! reopens a lost connection (the next press does).
//!
//! Also here: the words each open draft carries for when it falls due (`due_words`).
use crate::state::AppState;
use cairn_gui_live::{ConnectionGuard, NodeConnection};
use cairn_node::note::commit::{commit_overdue, overdue_drafts, CommitReport};
use cairn_node::note::view::instant;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use std::collections::HashSet;
use std::time::Duration;
use tauri::Manager;
use uuid::Uuid;

/// How often the window asks the node to commit overdue drafts. A minute: the quarantine is
/// hours, so a draft goes in at most a minute late, and an idle pass is one cheap query.
pub const COMMIT_TICK: Duration = Duration::from_secs(60);

/// Said of an overdue draft still on the chart after the pre-read commit ran, whose last commit did
/// not fail: it fell due since, or a save or Sign held its lock. Either way it is tried again,
/// which is all that is certain (review I1).
const OVERDUE: &str =
    "overdue — not in the record yet; the window tries again at every check to put it in unsigned";
/// Said of an overdue draft whose LAST commit in this window failed (the cause is on stderr). Not
/// "tries again": a failure that repeats every pass would make that a promise nobody keeps, and an
/// abandoned draft has no other way in, so the clinician is told to act (PR #981 review F1).
const FAILED: &str = "overdue — could not go into the record by itself (the window's last try \
     failed; the cause is in its log); sign it, or tell the site's operator";
/// Said of a STUCK draft: a writer has no vault key, so every commit rolls it back. Not "sign it":
/// a Sign refuses the same draft (`SignRefusal::NoSuchWriter`), so only re-enrolling them helps.
const STUCK: &str = "overdue, but cannot go into the record by itself — a writer of it is no \
     longer enrolled on this node; it goes in once they are enrolled again";
/// Said of a stuck draft NOT yet due (review I1): it will not go in at its due time either, so it
/// never says when it goes in — only what lets it.
const STUCK_NOT_DUE: &str = "cannot go into the record by itself — a writer of it is no longer \
     enrolled on this node; it can go in only once they are enrolled again";

/// Commit the overdue drafts on `patient`'s chart (`None` = every chart), and say on stderr each
/// one whose commit failed. `None` when nothing was asked: fixture mode, no node key, or no
/// connection. Never an error to the caller — a read must go on whatever this did.
///
/// One short hold lists the drafts, then each draft is committed under a hold of its own, so the
/// connection is free between drafts and no hold nears the 15 s cut-off (review I2). The guards
/// live only inside this call, so the tick never holds one across its sleep. `may_reopen`: whether
/// a lost connection is reopened (the pre-read: a read reopens anyway) or skipped (the tick).
pub async fn commit_overdue_impl(
    state: &AppState,
    patient: Option<Uuid>,
    may_reopen: bool,
) -> Option<CommitReport> {
    let node_sk = state.node_sk.as_ref()?;
    let db = state.db.as_deref()?;
    let drafts = {
        let held = hold(db, may_reopen).await?;
        match overdue_drafts(&*held, patient).await {
            Ok(drafts) => drafts,
            Err(e) => {
                let e = held.explain(e);
                eprintln!(
                    "the quarantine commit could not list the overdue drafts: {}",
                    cairn_gui_live::cause_of(&e)
                );
                return None;
            }
        }
    };
    let mut report = CommitReport::default();
    for draft in drafts {
        // Lost mid-pass: the rest wait for the next pass.
        let Some(mut held) = hold(db, may_reopen).await else {
            break;
        };
        let outcome = commit_overdue(&mut held, node_sk, draft, &state.node_origin)
            .await
            .map_err(|e| held.explain(e));
        // What the pane says of the draft follows its LAST try: failed, or not.
        let mut failed = state.commit_failed.lock().await;
        if outcome.is_err() {
            failed.insert(draft);
        } else {
            failed.remove(&draft);
        }
        drop(failed);
        report.record(draft, outcome);
    }
    // The draft id and the cause only: never its text or the patient's name (stderr is a log).
    for (draft, cause) in &report.failed {
        eprintln!("draft {draft} is overdue but could not go into the record: {cause}");
    }
    Some(report)
}

/// Take the connection: reopening it if lost when `may_reopen`, else only if it is open.
async fn hold(db: &NodeConnection, may_reopen: bool) -> Option<ConnectionGuard<'_>> {
    if may_reopen {
        db.lock().await.ok()
    } else {
        db.lock_if_open().await
    }
}

/// The tick: forever, wait [`COMMIT_TICK`], then commit every chart's overdue drafts, saying each
/// stuck draft on stderr ONCE per window run (`newly_stuck`), not every minute. Spawned once, in
/// `main`'s setup, on Tauri's runtime. Waits FIRST, so a launch is never slowed by it (the first
/// chart read commits that chart's drafts anyway).
pub async fn run_ticks(app: tauri::AppHandle) {
    let mut said_stuck = HashSet::new();
    let mut paused = false;
    loop {
        tokio::time::sleep(COMMIT_TICK).await;
        let report = commit_overdue_impl(&app.state::<AppState>(), None, false).await;
        if let Some(line) = pause_change(&mut paused, report.is_some()) {
            eprintln!("{line}");
        }
        let Some(report) = report else {
            continue;
        };
        for draft in newly_stuck(&mut said_stuck, report.stuck) {
            eprintln!(
                "draft {draft} is overdue but cannot go into the record: a writer of it is no \
                 longer enrolled on this node"
            );
        }
    }
}

/// The stderr line, if any, for a tick that `ran` (or did not), `paused` the last tick's state,
/// which this updates: one line when the tick stops running and one when it runs again, never
/// one a minute. Without it, a window left open on a lost connection is silently no runner
/// (PR #981 review F2). Pure.
fn pause_change(paused: &mut bool, ran: bool) -> Option<&'static str> {
    let was = std::mem::replace(paused, !ran);
    match (was, ran) {
        (false, false) => Some(
            "the quarantine tick is paused: no open connection, or the overdue drafts could not \
             be listed; a chart read or the next press reopens it",
        ),
        (true, true) => Some("the quarantine tick is running again"),
        _ => None,
    }
}

/// The drafts in `stuck` not yet in `said`, which now holds them too.
fn newly_stuck(said: &mut HashSet<Uuid>, stuck: Vec<Uuid>) -> Vec<Uuid> {
    stuck.into_iter().filter(|d| said.insert(*d)).collect()
}

/// When an open draft goes into the record unsigned, in zone `tz` with `today` its date. `stuck`
/// wins whether or not it is due: such a draft never goes in by itself ([`STUCK`] once
/// `overdue`, else [`STUCK_NOT_DUE`]). Otherwise: at its `due` time if nobody types in it; or,
/// `overdue` (the node decided, on its own clock), [`FAILED`] when this window's last commit of
/// it failed, else [`OVERDUE`], which is true after the pre-read commit ran. Pure.
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
