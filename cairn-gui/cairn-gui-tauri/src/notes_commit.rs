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
//! note. This file only asks, and says on stderr what could not go in. And it never gets in the
//! way: fixture mode and a window without the node key commit nothing, and a commit that fails is
//! never a failed read (Review Focus 5).
//!
//! Also here: the words each open draft carries for when it falls due (`due_words`).
use crate::state::AppState;
use cairn_node::note::commit::{commit_overdue_drafts, CommitReport};
use cairn_node::note::view::instant;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use std::time::Duration;
use tauri::Manager;
use uuid::Uuid;

/// How often the window asks the node to commit overdue drafts. A minute: the quarantine is
/// hours, so a draft goes in at most a minute late, and an idle pass is one cheap query.
pub const COMMIT_TICK: Duration = Duration::from_secs(60);

/// Commit the overdue drafts on `patient`'s chart (`None` = every chart), and say on stderr each
/// one that could not go in. `None` when nothing was asked: fixture mode, no node key, or no
/// connection. Never an error to the caller — a read must go on whatever this did.
///
/// The connection guard lives only inside this call, so the tick never holds it across its sleep.
// shortcut: one hold covers the whole pass, so a node-wide backlog of many drafts could outlast
// the 15 s hold limit (the cut rolls back the draft in flight; the next tick resumes) and makes
// a command wait meanwhile; commit draft by draft, one hold each, if a site ever meets that.
pub async fn commit_overdue_impl(state: &AppState, patient: Option<Uuid>) -> Option<CommitReport> {
    let node_sk = state.node_sk.as_ref()?;
    let mut db = state.db.as_ref()?.lock().await.ok()?;
    let report = match commit_overdue_drafts(&mut db, node_sk, patient, &state.node_origin).await {
        Ok(report) => report,
        Err(e) => {
            eprintln!(
                "the quarantine commit could not list the overdue drafts: {}",
                cairn_gui_live::cause_of(&db.explain(e))
            );
            return None;
        }
    };
    drop(db);
    // The draft id and the cause only: never its text or the patient's name (stderr is a log).
    for draft in &report.stuck {
        eprintln!(
            "draft {draft} is overdue but cannot go into the record: a writer of it is no longer \
             enrolled on this node"
        );
    }
    for (draft, cause) in &report.failed {
        eprintln!("draft {draft} is overdue but could not go into the record: {cause}");
    }
    Some(report)
}

/// The tick: forever, wait [`COMMIT_TICK`], then commit every chart's overdue drafts. Spawned
/// once, in `main`'s setup, on Tauri's runtime. Waits FIRST, so a launch is never slowed by it
/// (the first chart read commits that chart's drafts anyway).
pub async fn run_ticks(app: tauri::AppHandle) {
    loop {
        tokio::time::sleep(COMMIT_TICK).await;
        commit_overdue_impl(&app.state::<AppState>(), None).await;
    }
}

/// When an open draft goes into the record unsigned, in zone `tz` with `today` its date: at its
/// `due` time if nobody types in it, or — `overdue`, which the node decided on its own clock — at
/// the next check (the tick, or the next read of the chart). Pure.
pub(crate) fn due_words<Tz: TimeZone>(
    due: DateTime<Utc>,
    overdue: bool,
    tz: &Tz,
    today: NaiveDate,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    if overdue {
        "overdue — goes into the record unsigned at the next check".into()
    } else {
        format!(
            "goes into the record unsigned at {} if nobody types in it",
            instant(due, tz, today)
        )
    }
}

#[cfg(test)]
#[path = "notes_commit_tests.rs"]
mod tests;
