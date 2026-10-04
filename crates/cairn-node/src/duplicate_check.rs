//! The commit-time duplicate check's status, as the node reads it (repair path R4, #679).
//!
//! The matcher worker (`cairn-matcher watch`, matcher/) drains db/056's notice log. This module
//! answers two questions without ever claiming more than is true (principle 4):
//! - node-wide: is the check current, catching up, stalled, or has it never run?
//!   ([`classify`] over [`read_snapshot`]);
//! - one chart: has it been checked since its identity evidence last changed?
//!   ([`chart_check_pending`]).
//!
//! "Stalled" (the line reads "behind") means a change has been waiting AND the worker has completed
//! no work for longer than [`STALLED_AFTER_SECS`]. That "quiet time" is db/056's `quiet_age_s`:
//! seconds since the oldest waiting notice or the worker's last completed work, whichever is later.
//! - A restore or a `reproject --rebuild` queues every chart at once, so every notice soon looks
//!   old — but a healthy worker keeps stamping completed work as it goes, and the backlog reads as
//!   catching up, never as an alarm (ruling R13: judging by notice age alone raised that alarm).
//! - It runs from the OLDEST notice, not the newest (ruling R17): on a busy node a fresh identity
//!   change arrives every few minutes, and judged by the newest notice a fully stopped worker would
//!   read "running" forever.
//!
//! All wording lives here, in pure functions with a golden test; R5's window will reuse them.

use anyhow::Context;
use tokio_postgres::Client;
use uuid::Uuid;

/// How long the quiet time may run — since the oldest waiting notice or the worker's last completed
/// work, whichever is later — before the check counts as stalled. Soft policy. A running
/// worker stamps after every chart it checks (seconds apart) and about every 30 s while a sweep is
/// successfully scoring pairs; nothing else counts. A sweep's blocking phase, a single slow pair,
/// and a pair that fails stamp nothing. So a change left waiting through five minutes without
/// completed work means the worker is stopped, crash-looping, failing on every pair, or stuck in
/// work that long — each worth the "behind" line.
pub const STALLED_AFTER_SECS: i64 = 5 * 60;

/// One read of db/056's `cairn_duplicate_check_status()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueSnapshot {
    /// How many distinct CHARTS have at least one waiting notice (a chart edited three times
    /// counts once).
    pub charts_waiting: i64,
    /// Seconds since the oldest waiting notice or the worker's last completed work
    /// (`last_drained_at`), whichever is later — how long a change has gone unattended. None when
    /// nothing waits.
    pub quiet_age_secs: Option<i64>,
    /// A 'config' re-check (every chart, after a matcher update or a first run) is in progress.
    pub config_recheck: bool,
    /// A worker has run on this node at least once (its state row exists).
    pub worker_seen: bool,
    /// The worker's last completed work (a checked chart or a successfully scored sweep pair), or
    /// an empty round, as the database's local HH:MM. None until the first. While a worker is
    /// crash-looping or in a long blocking phase it is active but this stays earlier — on purpose.
    pub last_drained_hhmm: Option<String>,
}

/// What the node may honestly say about its duplicate check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckState {
    /// No worker has ever run on this node, so no chart can be called checked. `waiting` is the
    /// number of charts with a waiting notice.
    NeverRun { waiting: i64 },
    /// A change has waited longer than the threshold and the worker has completed no work in that
    /// time: it is stopped or stuck. `last_ran` is its last completed work (HH:MM), None if it
    /// never completed any.
    Stalled {
        waiting: i64,
        last_ran: Option<String>,
    },
    /// Charts are waiting, and either the oldest change is recent or the worker completed work
    /// recently: it is working through them.
    /// `config_recheck` is true while a whole-population re-check (a matcher update, or the
    /// first run) is among them.
    CatchingUp { waiting: i64, config_recheck: bool },
    /// Nothing is waiting: every chart has been checked since its identity evidence last changed.
    /// `last_ran` is the worker's last completed work (HH:MM).
    Current { last_ran: Option<String> },
}

/// Classify a snapshot (pure). A missing worker row wins over everything: until a worker has run
/// once, charts that predate db/056 have never been checked, whatever the queue shows. Otherwise
/// the quiet time decides: none (nothing waits) is Current, strictly more than
/// `stalled_after_secs` is Stalled, anything else is CatchingUp.
pub fn classify(s: &QueueSnapshot, stalled_after_secs: i64) -> CheckState {
    if !s.worker_seen {
        return CheckState::NeverRun {
            waiting: s.charts_waiting,
        };
    }
    match s.quiet_age_secs {
        None => CheckState::Current {
            last_ran: s.last_drained_hhmm.clone(),
        },
        Some(quiet) if quiet > stalled_after_secs => CheckState::Stalled {
            waiting: s.charts_waiting,
            last_ran: s.last_drained_hhmm.clone(),
        },
        Some(_) => CheckState::CatchingUp {
            waiting: s.charts_waiting,
            config_recheck: s.config_recheck,
        },
    }
}

fn charts(n: i64) -> String {
    if n == 1 {
        "1 chart".into()
    } else {
        format!("{n} charts")
    }
}

/// The one sentence for a state (pure; golden-tested).
pub fn status_line(state: &CheckState) -> String {
    match state {
        CheckState::NeverRun { waiting: 0 } => "Duplicate check has never run on this node.".into(),
        CheckState::NeverRun { waiting } => format!(
            "Duplicate check has never run on this node — {} waiting.",
            charts(*waiting)
        ),
        CheckState::Stalled {
            waiting,
            last_ran: Some(t),
        } => format!(
            "Duplicate check is behind — last ran {t}; {} waiting.",
            charts(*waiting)
        ),
        CheckState::Stalled {
            waiting,
            last_ran: None,
        } => format!(
            "Duplicate check is behind — it has not finished a round yet; {} waiting.",
            charts(*waiting)
        ),
        CheckState::CatchingUp {
            waiting,
            config_recheck,
        } => format!(
            "Duplicate check running — {} waiting{}.",
            charts(*waiting),
            if *config_recheck {
                " (re-checking all charts after a matcher update)"
            } else {
                ""
            }
        ),
        CheckState::Current { last_ran: Some(t) } => {
            format!("Duplicate check up to date — last ran {t}.")
        }
        CheckState::Current { last_ran: None } => "Duplicate check up to date.".into(),
    }
}

/// One chart's line (pure; golden-tested).
pub fn chart_line(pending: bool) -> &'static str {
    if pending {
        "This chart: duplicate check not yet run since its identity details last changed."
    } else {
        "This chart: duplicate check up to date."
    }
}

/// Read the node-wide snapshot.
pub async fn read_snapshot(client: &Client) -> anyhow::Result<QueueSnapshot> {
    let r = client
        .query_one("SELECT * FROM cairn_duplicate_check_status()", &[])
        .await
        .context("reading the duplicate-check status")?;
    Ok(QueueSnapshot {
        charts_waiting: r.get("charts_waiting"),
        quiet_age_secs: r.get("quiet_age_s"),
        config_recheck: r.get("config_recheck"),
        worker_seen: r.get("worker_seen"),
        last_drained_hhmm: r.get("last_drained_hhmm"),
    })
}

/// Has `patient` been checked since its identity evidence last changed? (db/056's
/// `cairn_chart_check_pending`: TRUE while a notice waits, and for every chart before the
/// first worker run.)
pub async fn chart_check_pending(client: &Client, patient: Uuid) -> anyhow::Result<bool> {
    Ok(client
        .query_one(
            "SELECT cairn_chart_check_pending($1::text::uuid)",
            &[&patient.to_string()],
        )
        .await
        .context("reading whether this chart's duplicate check is pending")?
        .get(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(waiting: i64, quiet: Option<i64>, config: bool, seen: bool) -> QueueSnapshot {
        QueueSnapshot {
            charts_waiting: waiting,
            quiet_age_secs: quiet,
            config_recheck: config,
            worker_seen: seen,
            last_drained_hhmm: seen.then(|| "09:41".to_string()),
        }
    }

    #[test]
    fn no_worker_row_is_never_run_whatever_is_waiting() {
        assert_eq!(
            classify(&snap(3, Some(9999), false, false), 300),
            CheckState::NeverRun { waiting: 3 }
        );
    }

    #[test]
    fn an_empty_queue_is_current() {
        assert_eq!(
            classify(&snap(0, None, false, true), 300),
            CheckState::Current {
                last_ran: Some("09:41".into())
            }
        );
    }

    #[test]
    fn behind_is_judged_by_quiet_time_strictly_past_the_threshold() {
        assert!(matches!(
            classify(&snap(5, Some(300), false, true), 300),
            CheckState::CatchingUp { .. }
        ));
        assert!(matches!(
            classify(&snap(5, Some(301), false, true), 300),
            CheckState::Stalled { .. }
        ));
    }

    #[test]
    fn the_wording_golden() {
        assert_eq!(
            status_line(&CheckState::NeverRun { waiting: 0 }),
            "Duplicate check has never run on this node."
        );
        assert_eq!(
            status_line(&CheckState::NeverRun { waiting: 2 }),
            "Duplicate check has never run on this node — 2 charts waiting."
        );
        assert_eq!(
            status_line(&CheckState::Stalled {
                waiting: 1,
                last_ran: Some("09:41".into())
            }),
            "Duplicate check is behind — last ran 09:41; 1 chart waiting."
        );
        assert_eq!(
            status_line(&CheckState::Stalled {
                waiting: 4,
                last_ran: None
            }),
            "Duplicate check is behind — it has not finished a round yet; 4 charts waiting."
        );
        assert_eq!(
            status_line(&CheckState::CatchingUp {
                waiting: 7,
                config_recheck: false
            }),
            "Duplicate check running — 7 charts waiting."
        );
        assert_eq!(
            status_line(&CheckState::CatchingUp { waiting: 7, config_recheck: true }),
            "Duplicate check running — 7 charts waiting (re-checking all charts after a matcher update)."
        );
        assert_eq!(
            status_line(&CheckState::Current {
                last_ran: Some("09:41".into())
            }),
            "Duplicate check up to date — last ran 09:41."
        );
        assert_eq!(
            chart_line(true),
            "This chart: duplicate check not yet run since its identity details last changed."
        );
        assert_eq!(chart_line(false), "This chart: duplicate check up to date.");
    }
}
