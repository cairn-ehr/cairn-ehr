//! The commit-time duplicate check's status, as the node reads it (repair path R4, #679).
//!
//! The matcher worker (`cairn-matcher watch`, matcher/) drains db/056's notice log. This module
//! answers two questions without ever claiming more than is true (principle 4):
//! - node-wide: is the check current, catching up, stalled, or has it never run?
//!   ([`classify`] over [`read_snapshot`]);
//! - one chart: has it been checked since its identity evidence last changed?
//!   ([`chart_check_pending`]).
//!
//! "Stalled" is judged by the NEWEST waiting notice. The worker drains newest first, so only a
//! stopped or stuck worker lets the newest notice age. A restore backlog — thousands of OLD notices
//! being worked through while fresh registrations are checked within seconds — reads as catching
//! up, never as an alarm. All wording lives here, in pure functions with a golden test; R5's window
//! will reuse them.

use anyhow::Context;
use tokio_postgres::Client;
use uuid::Uuid;

/// How old the newest waiting notice may be before the check counts as stalled. Soft policy: a
/// worker drains a fresh change in seconds, so five minutes is generous.
pub const STALLED_AFTER_SECS: i64 = 5 * 60;

/// One read of db/056's `cairn_duplicate_check_status()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueSnapshot {
    pub charts_waiting: i64,
    /// Age in seconds of the NEWEST waiting notice; None when nothing waits.
    pub newest_age_secs: Option<i64>,
    /// A 'config' re-check (every chart, after a matcher update or a first run) is in progress.
    pub config_recheck: bool,
    /// A worker has run on this node at least once (its state row exists).
    pub worker_seen: bool,
    /// When the worker last finished a drain, as the database's local HH:MM.
    pub last_drained_hhmm: Option<String>,
}

/// What the node may honestly say about its duplicate check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckState {
    NeverRun {
        waiting: i64,
    },
    Stalled {
        waiting: i64,
        last_ran: Option<String>,
    },
    CatchingUp {
        waiting: i64,
        config_recheck: bool,
    },
    Current {
        last_ran: Option<String>,
    },
}

/// Classify a snapshot (pure). A missing worker row wins over everything: until a worker has run
/// once, charts that predate db/056 have never been checked, whatever the queue shows.
pub fn classify(s: &QueueSnapshot, stalled_after_secs: i64) -> CheckState {
    if !s.worker_seen {
        return CheckState::NeverRun {
            waiting: s.charts_waiting,
        };
    }
    match s.newest_age_secs {
        None => CheckState::Current {
            last_ran: s.last_drained_hhmm.clone(),
        },
        Some(age) if age > stalled_after_secs => CheckState::Stalled {
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
        newest_age_secs: r.get("newest_age_s"),
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

    fn snap(waiting: i64, newest: Option<i64>, config: bool, seen: bool) -> QueueSnapshot {
        QueueSnapshot {
            charts_waiting: waiting,
            newest_age_secs: newest,
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
    fn behind_is_judged_by_the_newest_notice_strictly_past_the_threshold() {
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
